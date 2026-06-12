//! SQLite implementation of the [`Storage`] seam via `sqlx`.
//!
//! Append-only SQLite is the design-sanctioned observability sink (design
//! §6.6). The migration is idempotent and runs at startup; a bad database
//! path fails boot, not the first call. (The sqlite-vec extension caveat does
//! not apply here — no extensions are loaded in this feature.)

use crate::error::{AppError, Outcome};
use crate::memory::{Kind, Memory, Trust};
use crate::telemetry::InvocationRecord;
use crate::traits::storage::Storage;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

const MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memories (
    id              TEXT PRIMARY KEY,
    content         TEXT NOT NULL,
    kind            TEXT NOT NULL,
    origin          TEXT NOT NULL,
    external        INTEGER NOT NULL,
    trust           TEXT NOT NULL,
    tags            TEXT NOT NULL,
    embedding       BLOB NOT NULL,
    embedding_model TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS invocation_records (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL,
    tool          TEXT NOT NULL,
    model         TEXT NOT NULL,
    input_tokens  INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_usd      REAL NOT NULL,
    latency_ms    INTEGER NOT NULL,
    outcome       TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
";

/// SQLite-backed [`Storage`].
#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Open (creating if missing) the database at `path` and run the
    /// idempotent migration. Use `":memory:"` for an in-memory store (tests).
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Storage`] when the file cannot be opened/created or
    /// the migration fails.
    pub async fn connect(path: &str) -> Result<Self, AppError> {
        let options = if path == ":memory:" {
            SqliteConnectOptions::from_str("sqlite::memory:")
                .map_err(|e| AppError::Storage(format!("invalid memory database: {e}")))?
        } else {
            // filename() takes the path verbatim — no URL parsing, so Windows
            // backslashes, spaces, and '?'/'#' are safe.
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
        };

        // One connection: correct for :memory: (each pool connection would
        // otherwise get its own empty database) and ample for a single-user
        // stdio server.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| AppError::Storage(format!("cannot open database '{path}': {e}")))?;

        sqlx::raw_sql(MIGRATION)
            .execute(&pool)
            .await
            .map_err(|e| AppError::Storage(format!("migration failed: {e}")))?;

        Ok(Self { pool })
    }

    /// Read back all invocation records, newest first. Not part of the
    /// [`Storage`] trait — an implementation-level inspection surface used by
    /// tests and operators.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Storage`] on read failure or a row that violates
    /// the record contract (unknown outcome, unparseable timestamp).
    pub async fn list_invocations(&self) -> Result<Vec<InvocationRecord>, AppError> {
        let rows = sqlx::query(
            "SELECT id, session_id, tool, model, input_tokens, output_tokens,
                    cost_usd, latency_ms, outcome, created_at
             FROM invocation_records ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Storage(format!("read failed: {e}")))?;

        rows.into_iter()
            .map(|row| {
                let outcome_text: String = row.get("outcome");
                let outcome = Outcome::parse(&outcome_text).ok_or_else(|| {
                    AppError::Storage(format!("unknown outcome in store: {outcome_text}"))
                })?;
                let created_text: String = row.get("created_at");
                let created_at = DateTime::parse_from_rfc3339(&created_text)
                    .map_err(|e| AppError::Storage(format!("bad created_at: {e}")))?
                    .with_timezone(&Utc);
                // A negative count is the same class of contract violation as
                // an unknown outcome — loud, never coerced to zero.
                let unsigned = |field: &str, value: i64| {
                    u64::try_from(value).map_err(|_| {
                        AppError::Storage(format!("negative {field} in store: {value}"))
                    })
                };
                let input_tokens = unsigned("input_tokens", row.get("input_tokens"))?;
                let output_tokens = unsigned("output_tokens", row.get("output_tokens"))?;
                let latency_ms = unsigned("latency_ms", row.get("latency_ms"))?;
                Ok(InvocationRecord {
                    id: row.get("id"),
                    session_id: row.get("session_id"),
                    tool: row.get("tool"),
                    model: row.get("model"),
                    input_tokens,
                    output_tokens,
                    cost_usd: row.get("cost_usd"),
                    latency_ms,
                    outcome,
                    created_at,
                })
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl Storage for SqliteStorage {
    async fn save_session(&self, id: &str, data: &Value) -> Result<(), AppError> {
        sqlx::query("INSERT INTO sessions (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data")
            .bind(id)
            .bind(data.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Storage(format!("session write failed: {e}")))?;
        Ok(())
    }

    async fn load_session(&self, id: &str) -> Result<Option<Value>, AppError> {
        let row = sqlx::query("SELECT data FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Storage(format!("session read failed: {e}")))?;
        row.map(|r| {
            let text: String = r.get("data");
            serde_json::from_str(&text)
                .map_err(|e| AppError::Storage(format!("session blob corrupt: {e}")))
        })
        .transpose()
    }

    async fn save_memory(&self, memory: &Memory) -> Result<(), AppError> {
        let tags = serde_json::to_string(&memory.tags)
            .map_err(|e| AppError::Storage(format!("tags serialization: {e}")))?;
        sqlx::query(
            "INSERT INTO memories
                (id, content, kind, origin, external, trust, tags,
                 embedding, embedding_model, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&memory.id)
        .bind(&memory.content)
        .bind(memory.kind.as_str())
        .bind(&memory.origin)
        .bind(i64::from(memory.external))
        .bind(memory.trust.as_str())
        .bind(tags)
        .bind(embedding_to_blob(&memory.embedding))
        .bind(&memory.embedding_model)
        .bind(memory.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Storage(format!("memory write failed: {e}")))?;
        Ok(())
    }

    async fn load_memories(&self) -> Result<Vec<Memory>, AppError> {
        let rows = sqlx::query(
            "SELECT id, content, kind, origin, external, trust, tags,
                    embedding, embedding_model, created_at
             FROM memories",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Storage(format!("memory read failed: {e}")))?;

        rows.into_iter()
            .map(|row| {
                let id: String = row.get("id");
                // A misaligned BLOB is the same class of contract violation as
                // an unknown enum — loud, never silently truncated.
                let blob: &[u8] = row.get("embedding");
                if !blob.len().is_multiple_of(4) {
                    return Err(AppError::Storage(format!(
                        "embedding blob length {} is not a multiple of 4 for memory {id}",
                        blob.len()
                    )));
                }
                let kind_text: String = row.get("kind");
                let kind = Kind::parse(&kind_text).ok_or_else(|| {
                    AppError::Storage(format!("unknown memory kind in store: {kind_text}"))
                })?;
                let trust_text: String = row.get("trust");
                let trust = Trust::parse(&trust_text).ok_or_else(|| {
                    AppError::Storage(format!("unknown trust in store: {trust_text}"))
                })?;
                let tags_text: String = row.get("tags");
                let tags: Vec<String> = serde_json::from_str(&tags_text)
                    .map_err(|e| AppError::Storage(format!("tags corrupt: {e}")))?;
                let created_text: String = row.get("created_at");
                let created_at = DateTime::parse_from_rfc3339(&created_text)
                    .map_err(|e| AppError::Storage(format!("bad created_at: {e}")))?
                    .with_timezone(&Utc);
                let external: i64 = row.get("external");
                Ok(Memory {
                    id,
                    content: row.get("content"),
                    kind,
                    origin: row.get("origin"),
                    external: external != 0,
                    trust,
                    tags,
                    embedding: embedding_from_blob(blob),
                    embedding_model: row.get("embedding_model"),
                    created_at,
                })
            })
            .collect()
    }

    async fn delete_memory(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM memories WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Storage(format!("memory delete failed: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn record_invocation(&self, record: &InvocationRecord) -> Result<(), AppError> {
        #[allow(clippy::cast_possible_wrap)] // token/latency counts are far below i64::MAX
        sqlx::query(
            "INSERT INTO invocation_records
                (id, session_id, tool, model, input_tokens, output_tokens,
                 cost_usd, latency_ms, outcome, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&record.id)
        .bind(&record.session_id)
        .bind(&record.tool)
        .bind(&record.model)
        .bind(record.input_tokens as i64)
        .bind(record.output_tokens as i64)
        .bind(record.cost_usd)
        .bind(record.latency_ms as i64)
        .bind(record.outcome.as_str())
        .bind(record.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Storage(format!("record write failed: {e}")))?;
        Ok(())
    }
}

/// f32 slice → little-endian BLOB (bit-exact round trip; spike S1).
fn embedding_to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Little-endian BLOB → f32 vector. Callers must reject misaligned blobs
/// first (`load_memories` does, loudly, with the row id).
fn embedding_from_blob(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(outcome: Outcome) -> InvocationRecord {
        InvocationRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: "session-1".into(),
            tool: "verify".into(),
            model: "claude-opus-4-8".into(),
            input_tokens: 300,
            output_tokens: 30,
            cost_usd: 0.00225,
            latency_ms: 1200,
            outcome,
            created_at: DateTime::parse_from_rfc3339("2026-06-11T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[tokio::test]
    async fn migration_is_idempotent() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        // Re-running the migration on a live store must not fail or wipe data.
        storage
            .record_invocation(&sample(Outcome::Success))
            .await
            .unwrap();
        sqlx::raw_sql(MIGRATION)
            .execute(&storage.pool)
            .await
            .unwrap();
        assert_eq!(storage.list_invocations().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn every_recordable_outcome_round_trips() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let outcomes = [
            Outcome::Success,
            Outcome::Refusal,
            Outcome::Truncation,
            Outcome::Timeout,
            Outcome::RetriesExhausted,
            Outcome::InvalidInput,
            Outcome::ValidationFailure,
            Outcome::Cancelled,
        ];
        for outcome in outcomes {
            storage.record_invocation(&sample(outcome)).await.unwrap();
        }

        let records = storage.list_invocations().await.unwrap();
        assert_eq!(records.len(), outcomes.len());
        for outcome in outcomes {
            assert!(
                records.iter().any(|r| r.outcome == outcome),
                "{outcome:?} missing"
            );
        }
        // Field fidelity on one record.
        let any = &records[0];
        assert_eq!(any.tool, "verify");
        assert_eq!(any.input_tokens, 300);
        assert!((any.cost_usd - 0.00225).abs() < 1e-12);
    }

    #[tokio::test]
    async fn one_row_per_record_id() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let record = sample(Outcome::Success);
        storage.record_invocation(&record).await.unwrap();
        // Same id again violates the primary key — surfaced, not silent.
        let err = storage.record_invocation(&record).await.unwrap_err();
        assert!(matches!(err, AppError::Storage(_)));
        assert_eq!(storage.list_invocations().await.unwrap().len(), 1);
    }

    fn sample_memory(id: &str, trust: Trust) -> Memory {
        Memory {
            id: id.to_string(),
            content: format!("content for {id}"),
            kind: Kind::Lesson,
            origin: "test".into(),
            external: trust == Trust::Untrusted,
            trust,
            tags: vec!["alpha".into(), "beta".into()],
            embedding: vec![0.25, -1.5, 3.0e-7, 42.0],
            embedding_model: "voyage-4".into(),
            created_at: DateTime::parse_from_rfc3339("2026-06-11T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[tokio::test]
    async fn every_trust_value_round_trips_bit_exact() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let trusts = [Trust::FirstHand, Trust::Verified, Trust::Untrusted];
        for (i, trust) in trusts.into_iter().enumerate() {
            storage
                .save_memory(&sample_memory(&format!("m{i}"), trust))
                .await
                .unwrap();
        }

        let loaded = storage.load_memories().await.unwrap();
        assert_eq!(loaded.len(), trusts.len());
        for (i, trust) in trusts.into_iter().enumerate() {
            let expected = sample_memory(&format!("m{i}"), trust);
            let got = loaded.iter().find(|m| m.id == expected.id).unwrap();
            // Full struct fidelity including the bit-exact f32 embedding (spike S1).
            assert_eq!(got, &expected);
        }
    }

    #[tokio::test]
    async fn forget_deletes_by_id_and_reports_unknown() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        storage
            .save_memory(&sample_memory("keep", Trust::FirstHand))
            .await
            .unwrap();
        storage
            .save_memory(&sample_memory("drop", Trust::Untrusted))
            .await
            .unwrap();

        assert!(storage.delete_memory("drop").await.unwrap());
        assert!(!storage.delete_memory("drop").await.unwrap()); // already gone
        assert!(!storage.delete_memory("never-existed").await.unwrap());

        let remaining = storage.load_memories().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "keep");
    }

    #[tokio::test]
    async fn migration_rerun_preserves_memories() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        storage
            .save_memory(&sample_memory("m", Trust::Verified))
            .await
            .unwrap();
        sqlx::raw_sql(MIGRATION)
            .execute(&storage.pool)
            .await
            .unwrap();
        assert_eq!(storage.load_memories().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn misaligned_embedding_blob_is_a_loud_error_not_a_truncation() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        sqlx::query(
            "INSERT INTO memories
                (id, content, kind, origin, external, trust, tags,
                 embedding, embedding_model, created_at)
             VALUES ('bad', 'c', 'fact', 'o', 0, 'first_hand', '[]',
                     ?, 'voyage-4', '2026-06-11T12:00:00+00:00')",
        )
        .bind(vec![0_u8; 5]) // not a multiple of 4
        .execute(&storage.pool)
        .await
        .unwrap();

        let err = storage.load_memories().await.unwrap_err();
        assert!(matches!(err, AppError::Storage(_)));
        assert!(
            err.to_string().contains("not a multiple of 4") && err.to_string().contains("bad"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn duplicate_memory_id_is_a_loud_error() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let memory = sample_memory("m", Trust::FirstHand);
        storage.save_memory(&memory).await.unwrap();
        let err = storage.save_memory(&memory).await.unwrap_err();
        assert!(matches!(err, AppError::Storage(_)));
    }

    #[tokio::test]
    async fn sessions_round_trip() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        assert!(storage.load_session("missing").await.unwrap().is_none());
        storage
            .save_session("s1", &json!({ "k": 1 }))
            .await
            .unwrap();
        storage
            .save_session("s1", &json!({ "k": 2 }))
            .await
            .unwrap(); // overwrite
        assert_eq!(
            storage.load_session("s1").await.unwrap(),
            Some(json!({ "k": 2 }))
        );
    }
}
