//! The `turn_signal_records` table (066 FR-011): one row per covered turn
//! end, joined to the checkpoint record by `eval_id`. Holds no conversation
//! text: unmatched entities are stored as a kind and a hash.
//!
//! Bound to `specs/066-jev-turn-end/contracts/turn-signal-record.schema.json`
//! by a two-way test.

use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

/// Creates the table and its index; idempotent, run at startup.
pub const TURN_SIGNALS_MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS turn_signal_records (
    id                 TEXT PRIMARY KEY,
    created_at         TEXT NOT NULL,
    eval_id            TEXT NOT NULL,
    harness_session_id TEXT,
    transcript_ref     TEXT,
    turn_start_line    INTEGER,
    final_line         INTEGER,
    pushback_result    TEXT NOT NULL,
    unverified_result  TEXT NOT NULL,
    unmatched_entities TEXT NOT NULL,
    recommendations    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS turn_signal_records_eval ON turn_signal_records (eval_id);
";

/// One `turn_signal_records` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSignalRecord {
    /// Row id.
    pub id: String,
    /// When the turn end was evaluated.
    pub created_at: DateTime<Utc>,
    /// The checkpoint record this row joins.
    pub eval_id: String,
    /// The Claude Code session.
    pub harness_session_id: Option<String>,
    /// The transcript path.
    pub transcript_ref: Option<String>,
    /// Line index of the turn start in the tail read.
    pub turn_start_line: Option<u32>,
    /// Line index of the final message in the tail read.
    pub final_line: Option<u32>,
    /// `fired`, `not_fired`, or an exclusion reason.
    pub pushback_result: String,
    /// `fired`, `not_fired`, or an exclusion reason.
    pub unverified_result: String,
    /// `[{kind, hash}]` for each unmatched entity.
    pub unmatched_entities: Value,
    /// `{pushback_reversal, unverified_fact}` corrective names or null.
    pub recommendations: Value,
}

impl TurnSignalRecord {
    /// The row as the contract's JSON object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "created_at": self.created_at.to_rfc3339(),
            "eval_id": self.eval_id,
            "harness_session_id": self.harness_session_id,
            "transcript_ref": self.transcript_ref,
            "turn_start_line": self.turn_start_line,
            "final_line": self.final_line,
            "pushback_result": self.pushback_result,
            "unverified_result": self.unverified_result,
            "unmatched_entities": self.unmatched_entities,
            "recommendations": self.recommendations,
        })
    }
}

fn storage_err(what: &'static str) -> impl Fn(sqlx::Error) -> AppError {
    move |e| AppError::Storage(format!("turn_signal_records {what} failed: {e}"))
}

/// Write one row.
///
/// # Errors
///
/// Returns [`AppError::Storage`] when the write fails.
pub async fn insert(pool: &SqlitePool, r: &TurnSignalRecord) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO turn_signal_records (id, created_at, eval_id, harness_session_id,
            transcript_ref, turn_start_line, final_line, pushback_result,
            unverified_result, unmatched_entities, recommendations)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&r.id)
    .bind(r.created_at.to_rfc3339())
    .bind(&r.eval_id)
    .bind(&r.harness_session_id)
    .bind(&r.transcript_ref)
    .bind(r.turn_start_line.map(i64::from))
    .bind(r.final_line.map(i64::from))
    .bind(&r.pushback_result)
    .bind(&r.unverified_result)
    .bind(r.unmatched_entities.to_string())
    .bind(r.recommendations.to_string())
    .execute(pool)
    .await
    .map_err(storage_err("insert"))?;
    Ok(())
}

/// Every row, oldest first.
///
/// # Errors
///
/// Returns [`AppError::Storage`] when the read fails or a stored value does
/// not parse.
pub async fn load(pool: &SqlitePool) -> Result<Vec<TurnSignalRecord>, AppError> {
    sqlx::query("SELECT * FROM turn_signal_records ORDER BY created_at, id")
        .fetch_all(pool)
        .await
        .map_err(storage_err("read"))?
        .iter()
        .map(from_row)
        .collect()
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<TurnSignalRecord, AppError> {
    let bad = |what: &str| AppError::Storage(format!("turn_signal_records: bad {what}"));
    let line = |name: &str| -> Result<Option<u32>, AppError> {
        row.get::<Option<i64>, _>(name)
            .map(|v| u32::try_from(v).map_err(|_| bad(name)))
            .transpose()
    };
    let json = |name: &str| -> Result<Value, AppError> {
        serde_json::from_str(&row.get::<String, _>(name)).map_err(|_| bad(name))
    };
    Ok(TurnSignalRecord {
        id: row.get("id"),
        created_at: DateTime::parse_from_rfc3339(&row.get::<String, _>("created_at"))
            .map_err(|_| bad("created_at"))?
            .with_timezone(&Utc),
        eval_id: row.get("eval_id"),
        harness_session_id: row.get("harness_session_id"),
        transcript_ref: row.get("transcript_ref"),
        turn_start_line: line("turn_start_line")?,
        final_line: line("final_line")?,
        pushback_result: row.get("pushback_result"),
        unverified_result: row.get("unverified_result"),
        unmatched_entities: json("unmatched_entities")?,
        recommendations: json("recommendations")?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{TurnSignalRecord, TURN_SIGNALS_MIGRATION};
    use crate::storage::SqliteStorage;
    use crate::traits::storage::Storage;
    use chrono::{TimeZone, Utc};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::Row;
    use std::collections::BTreeSet;

    fn schema() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../specs/066-jev-turn-end/contracts/turn-signal-record.schema.json"
        ))
        .unwrap()
    }

    /// `(name, notnull)` for every column of a fresh table.
    async fn columns() -> Vec<(String, bool)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(TURN_SIGNALS_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA table_info(turn_signal_records)")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row.get::<String, _>("name"),
                    row.get::<i64, _>("notnull") == 1 || row.get::<i64, _>("pk") == 1,
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn every_column_is_a_schema_property_and_every_property_a_column() {
        let properties: BTreeSet<String> = schema()["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let columns: BTreeSet<String> = columns().await.into_iter().map(|(n, _)| n).collect();
        assert_eq!(columns, properties);
    }

    #[tokio::test]
    async fn the_required_properties_are_exactly_the_not_null_columns() {
        let required: BTreeSet<String> = schema()["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let not_null: BTreeSet<String> = columns()
            .await
            .into_iter()
            .filter(|(_, not_null)| *not_null)
            .map(|(n, _)| n)
            .collect();
        assert_eq!(not_null, required);
    }

    fn record(id: &str) -> TurnSignalRecord {
        TurnSignalRecord {
            id: id.into(),
            created_at: Utc.with_ymd_and_hms(2026, 11, 26, 12, 0, 0).unwrap(),
            eval_id: "eval-1".into(),
            harness_session_id: Some("fixture-session".into()),
            transcript_ref: Some("fixture.jsonl".into()),
            turn_start_line: Some(2),
            final_line: Some(19),
            pushback_result: "no previous final message".into(),
            unverified_result: "fired".into(),
            unmatched_entities: serde_json::json!([
                { "kind": "number", "hash": "0123456789abcdef" }
            ]),
            recommendations: serde_json::json!({
                "pushback_reversal": "verify", "unverified_fact": null
            }),
        }
    }

    #[tokio::test]
    async fn a_row_reads_back_unchanged_and_fits_the_contract() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let written = record("row-1");
        storage.record_turn_signals(&written).await.unwrap();
        let read = storage.load_turn_signal_records().await.unwrap();
        assert_eq!(read, vec![written]);
        let checked = crate::schema::validate(&schema(), &read[0].to_json());
        assert!(checked.is_ok(), "{checked:?}");
    }

    #[tokio::test]
    async fn a_row_with_no_anchors_reads_back_with_nulls() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let written = TurnSignalRecord {
            harness_session_id: None,
            transcript_ref: None,
            turn_start_line: None,
            final_line: None,
            pushback_result: "transcript unreadable".into(),
            unverified_result: "transcript unreadable".into(),
            unmatched_entities: serde_json::json!([]),
            ..record("row-2")
        };
        storage.record_turn_signals(&written).await.unwrap();
        assert_eq!(
            storage.load_turn_signal_records().await.unwrap(),
            vec![written]
        );
    }
}
