//! The `jev_records` table (064): one row per Jev request, or per excluded
//! research claim, beside the big model's label for the same item.
//!
//! Bound to `specs/064-jev-first-pass/contracts/jev-record.schema.json` by a
//! two-way test. `applied` is 1 only for an `on` row whose answer was used
//! (065); every shadow row writes 0.

use crate::error::{AppError, Outcome};
use crate::traits::classifier::{JevMode, JevSite};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

/// Creates the table and its indexes; idempotent, run at startup.
pub const JEV_MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS jev_records (
    id                TEXT PRIMARY KEY,
    created_at        TEXT NOT NULL,
    server_session_id TEXT NOT NULL,
    harness_session_id TEXT,
    site              TEXT NOT NULL,
    mode              TEXT NOT NULL,
    questions_json    TEXT NOT NULL,
    model_requested   TEXT NOT NULL,
    model_answered    TEXT,
    outcome           TEXT NOT NULL,
    error_class       TEXT,
    request_id        TEXT,
    latency_ms        INTEGER,
    input_tokens      INTEGER,
    cost_usd          REAL,
    applied           INTEGER NOT NULL,
    eval_id           TEXT,
    invocation_id     TEXT,
    claim_key         TEXT,
    source_ids        TEXT,
    label_json        TEXT,
    label_source      TEXT,
    label_cost_usd    REAL,
    cooldown_suppressed TEXT,
    state_json        TEXT,
    state_hash        TEXT,
    transcript_ref    TEXT,
    redaction_count   INTEGER NOT NULL,
    truncated         INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS jev_records_site_created ON jev_records (site, created_at);
CREATE INDEX IF NOT EXISTS jev_records_eval ON jev_records (eval_id);
CREATE INDEX IF NOT EXISTS jev_records_invocation ON jev_records (invocation_id);
";

/// A row's outcome: one of the published outcomes, or an excluded claim
/// (no request sent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JevOutcome {
    /// A request was made (or attempted) and ended this way.
    Call(Outcome),
    /// Excluded in code before any request.
    Excluded,
}

impl JevOutcome {
    /// The stored string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Call(outcome) => outcome.as_str(),
            Self::Excluded => "excluded",
        }
    }

    /// Parse a stored string.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        if s == "excluded" {
            Some(Self::Excluded)
        } else {
            Outcome::parse(s).map(Self::Call)
        }
    }
}

/// One `jev_records` row; see the schema for each field's meaning.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)] // one field per column, documented in the schema file
pub struct JevRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub server_session_id: String,
    pub harness_session_id: Option<String>,
    pub site: JevSite,
    pub mode: JevMode,
    pub questions_json: Value,
    pub model_requested: String,
    pub model_answered: Option<String>,
    pub outcome: JevOutcome,
    pub error_class: Option<String>,
    pub request_id: Option<String>,
    pub latency_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub eval_id: Option<String>,
    pub invocation_id: Option<String>,
    pub claim_key: Option<String>,
    pub source_ids: Option<Vec<String>>,
    pub label_json: Option<Value>,
    pub label_source: Option<String>,
    /// The label call's cost (067 tool risk); `None` where none was made.
    pub label_cost_usd: Option<f64>,
    /// Turn signals (068): the signal kinds whose yes answer would have been
    /// suppressed by the cooldown; `None` on other sites.
    pub cooldown_suppressed: Option<Vec<String>>,
    pub state_json: Option<Value>,
    pub state_hash: Option<String>,
    pub transcript_ref: Option<String>,
    pub redaction_count: u32,
    pub truncated: bool,
    /// The answer changed what Parallax returned (065: an `on` injection row
    /// that warned). Always false in shadow.
    pub applied: bool,
}

fn storage_err(what: &str) -> impl Fn(sqlx::Error) -> AppError + '_ {
    move |e| AppError::Storage(format!("jev_records {what}: {e}"))
}

fn to_i64(value: Option<u64>, column: &str) -> Result<Option<i64>, AppError> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|_| AppError::Storage(format!("jev_records {column} out of range")))
}

fn json_text<T: serde::Serialize>(value: Option<&T>) -> Result<Option<String>, AppError> {
    value
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| AppError::Storage(format!("jev_records json: {e}")))
}

/// Write `rows` in one transaction: all or none.
///
/// # Errors
///
/// [`AppError::Storage`] when any row fails; nothing is kept.
pub async fn insert(pool: &SqlitePool, rows: &[JevRecord]) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(storage_err("begin"))?;
    for r in rows {
        sqlx::query(
            "INSERT INTO jev_records (id, created_at, server_session_id, harness_session_id,
                site, mode, questions_json, model_requested, model_answered, outcome,
                error_class, request_id, latency_ms, input_tokens, cost_usd, applied,
                eval_id, invocation_id, claim_key, source_ids, label_json, label_source,
                label_cost_usd, cooldown_suppressed, state_json, state_hash, transcript_ref,
                redaction_count, truncated)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.id)
        .bind(r.created_at.to_rfc3339())
        .bind(&r.server_session_id)
        .bind(&r.harness_session_id)
        .bind(r.site.as_str())
        .bind(r.mode.as_str())
        .bind(json_text(Some(&r.questions_json))?)
        .bind(&r.model_requested)
        .bind(&r.model_answered)
        .bind(r.outcome.as_str())
        .bind(&r.error_class)
        .bind(&r.request_id)
        .bind(to_i64(r.latency_ms, "latency_ms")?)
        .bind(to_i64(r.input_tokens, "input_tokens")?)
        .bind(r.cost_usd)
        .bind(i64::from(r.applied))
        .bind(&r.eval_id)
        .bind(&r.invocation_id)
        .bind(&r.claim_key)
        .bind(json_text(r.source_ids.as_ref())?)
        .bind(json_text(r.label_json.as_ref())?)
        .bind(&r.label_source)
        .bind(r.label_cost_usd)
        .bind(json_text(r.cooldown_suppressed.as_ref())?)
        .bind(json_text(r.state_json.as_ref())?)
        .bind(&r.state_hash)
        .bind(&r.transcript_ref)
        .bind(i64::from(r.redaction_count))
        .bind(i64::from(r.truncated))
        .execute(&mut *tx)
        .await
        .map_err(storage_err("insert"))?;
    }
    tx.commit().await.map_err(storage_err("commit"))
}

/// The label fields of a tool-risk row, filled after its label call ends
/// (067): the row is written when Jev answers, so a finished answer survives
/// a process exit while the label is still pending.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JevLabelUpdate {
    /// The label, or `None` when the label call failed or was skipped.
    pub label_json: Option<Value>,
    /// The label model and prompt version.
    pub label_source: Option<String>,
    /// The label call's cost.
    pub label_cost_usd: Option<f64>,
    /// The row's `error_class` after the label step (a `label:` note on failure).
    pub error_class: Option<String>,
}

/// Fill one row's label fields.
///
/// # Errors
///
/// [`AppError::Storage`] when the write fails or no row has `id`.
pub async fn update_label(
    pool: &SqlitePool,
    id: &str,
    update: &JevLabelUpdate,
) -> Result<(), AppError> {
    let done = sqlx::query(
        "UPDATE jev_records SET label_json = ?, label_source = ?, label_cost_usd = ?,
            error_class = ? WHERE id = ?",
    )
    .bind(json_text(update.label_json.as_ref())?)
    .bind(&update.label_source)
    .bind(update.label_cost_usd)
    .bind(&update.error_class)
    .bind(id)
    .execute(pool)
    .await
    .map_err(storage_err("label update"))?;
    if done.rows_affected() == 1 {
        Ok(())
    } else {
        Err(AppError::Storage(format!(
            "jev_records label update: no row {id}"
        )))
    }
}

fn bad(column: &str, value: &str) -> AppError {
    AppError::Storage(format!("jev_records {column} unreadable: {value}"))
}

fn parse_json<T: serde::de::DeserializeOwned>(
    text: Option<String>,
    column: &str,
) -> Result<Option<T>, AppError> {
    text.map(|t| serde_json::from_str(&t).map_err(|_| bad(column, &t)))
        .transpose()
}

fn to_u64(value: Option<i64>, column: &str) -> Result<Option<u64>, AppError> {
    value
        .map(|v| u64::try_from(v).map_err(|_| bad(column, &v.to_string())))
        .transpose()
}

/// Every row, oldest first.
///
/// # Errors
///
/// [`AppError::Storage`] when the read fails or a value is unreadable.
pub async fn load(pool: &SqlitePool) -> Result<Vec<JevRecord>, AppError> {
    let rows = sqlx::query("SELECT * FROM jev_records ORDER BY created_at, id")
        .fetch_all(pool)
        .await
        .map_err(storage_err("read"))?;
    rows.iter().map(from_row).collect()
}

/// The turn-signal kinds a session's Jev answered yes (0.5 or more) at or
/// after `since` (068 plan D3).
///
/// # Errors
///
/// [`AppError::Storage`] when the read fails or a value is unreadable.
pub async fn turn_signal_yes_kinds_since(
    pool: &SqlitePool,
    session_id: &str,
    since: DateTime<Utc>,
) -> Result<Vec<String>, AppError> {
    // Only rows whose turn call wrote its checkpoint record were delivered;
    // a cancelled call leaves rows with no record (068 review finding 1).
    let rows = sqlx::query(
        "SELECT j.* FROM jev_records j JOIN checkpoint_records c ON c.id = j.eval_id \
         WHERE j.site = ? AND j.harness_session_id = ?",
    )
    .bind(JevSite::TurnSignals.as_str())
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(storage_err("read"))?;
    let mut kinds = Vec::new();
    for row in &rows {
        let record = from_row(row)?;
        if record.created_at < since {
            continue;
        }
        for entry in record.questions_json.as_array().into_iter().flatten() {
            let yes = entry
                .get("answer")
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|p| p >= 0.5);
            if let (true, Some(kind)) = (yes, entry.get("id").and_then(serde_json::Value::as_str)) {
                // As the real cooldown: a yes the cooldown suppressed was
                // never delivered, so it starts no window (review finding 2).
                let suppressed = record
                    .cooldown_suppressed
                    .as_ref()
                    .is_some_and(|listed| listed.iter().any(|k| k == kind));
                if !suppressed && !kinds.iter().any(|k| k == kind) {
                    kinds.push(kind.to_string());
                }
            }
        }
    }
    Ok(kinds)
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<JevRecord, AppError> {
    let created: String = row.get("created_at");
    let site: String = row.get("site");
    let mode: String = row.get("mode");
    let outcome: String = row.get("outcome");
    let redaction_count: i64 = row.get("redaction_count");
    Ok(JevRecord {
        id: row.get("id"),
        created_at: DateTime::parse_from_rfc3339(&created)
            .map_err(|_| bad("created_at", &created))?
            .with_timezone(&Utc),
        server_session_id: row.get("server_session_id"),
        harness_session_id: row.get("harness_session_id"),
        site: JevSite::ALL
            .into_iter()
            .find(|s| s.as_str() == site)
            .ok_or_else(|| bad("site", &site))?,
        mode: JevMode::parse(&mode).ok_or_else(|| bad("mode", &mode))?,
        questions_json: parse_json(Some(row.get("questions_json")), "questions_json")?
            .unwrap_or(Value::Null),
        model_requested: row.get("model_requested"),
        model_answered: row.get("model_answered"),
        outcome: JevOutcome::parse(&outcome).ok_or_else(|| bad("outcome", &outcome))?,
        error_class: row.get("error_class"),
        request_id: row.get("request_id"),
        latency_ms: to_u64(row.get("latency_ms"), "latency_ms")?,
        input_tokens: to_u64(row.get("input_tokens"), "input_tokens")?,
        cost_usd: row.get("cost_usd"),
        eval_id: row.get("eval_id"),
        invocation_id: row.get("invocation_id"),
        claim_key: row.get("claim_key"),
        source_ids: parse_json(row.get("source_ids"), "source_ids")?,
        label_json: parse_json(row.get("label_json"), "label_json")?,
        label_source: row.get("label_source"),
        label_cost_usd: row.get("label_cost_usd"),
        cooldown_suppressed: parse_json(row.get("cooldown_suppressed"), "cooldown_suppressed")?,
        state_json: parse_json(row.get("state_json"), "state_json")?,
        state_hash: row.get("state_hash"),
        transcript_ref: row.get("transcript_ref"),
        redaction_count: u32::try_from(redaction_count)
            .map_err(|_| bad("redaction_count", &redaction_count.to_string()))?,
        truncated: row.get::<i64, _>("truncated") != 0,
        applied: row.get::<i64, _>("applied") != 0,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{JevOutcome, JevRecord, JEV_MIGRATION};
    use crate::error::Outcome;
    use crate::storage::SqliteStorage;
    use crate::traits::classifier::{JevMode, JevSite};
    use crate::traits::storage::Storage;
    use chrono::{TimeZone, Utc};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::Row;
    use std::collections::BTreeSet;

    fn schema() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../specs/064-jev-first-pass/contracts/jev-record.schema.json"
        ))
        .unwrap()
    }

    #[test]
    fn every_turn_label_the_server_writes_fits_the_contract() {
        use crate::checkpoint::review::{CaptureKind, ReviewRaw};
        use crate::jev::shadow::{turn_label, ReviewOutcome};
        let label_schema = schema()["properties"]["label_json"].clone();
        let raw = ReviewRaw {
            contradicts: true,
            violates: true,
            capture_kind: CaptureKind::Lesson,
        };
        let did = ReviewOutcome {
            contradiction_flagged: true,
            violation_flagged: true,
            captured: true,
        };
        for memory_pairs in [false, true] {
            for capture_enabled in [false, true] {
                let label = turn_label(&raw, memory_pairs, capture_enabled, did);
                let checked = crate::schema::validate(&label_schema, &label);
                assert!(
                    checked.is_ok(),
                    "memory {memory_pairs}, capture {capture_enabled}: {checked:?}"
                );
            }
        }
    }

    #[test]
    fn the_contract_says_labels_without_the_outcome_fields_are_unlabeled() {
        let turn = &schema()["properties"]["label_json"]["oneOf"][1];
        let description = turn["description"].as_str().unwrap_or_default();
        assert!(description.contains("unlabeled"), "{description}");
    }

    /// `(name, notnull)` for every column of a fresh `jev_records` table.
    async fn columns() -> Vec<(String, bool)> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(JEV_MIGRATION).execute(&pool).await.unwrap();
        sqlx::query("PRAGMA table_info(jev_records)")
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
        let schema = schema();
        let properties: BTreeSet<String> = schema["properties"]
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
        let schema = schema();
        let required: BTreeSet<String> = schema["required"]
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

    #[test]
    fn site_and_mode_enums_match_the_code() {
        let schema = schema();
        let names = |key: &str| -> Vec<String> {
            schema["properties"][key]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        };
        let sites: Vec<String> = JevSite::ALL.iter().map(|s| s.as_str().into()).collect();
        assert_eq!(names("site"), sites);
        // `off` sends nothing, so it is never recorded.
        let modes: Vec<String> = JevMode::ALL
            .iter()
            .filter(|m| **m != JevMode::Off)
            .map(|m| m.as_str().into())
            .collect();
        assert_eq!(names("mode"), modes);
    }

    // 067 T004: the contract's label_json has a tool_risk branch, the big
    // model's action_class answer for the same projection.
    #[test]
    fn the_contract_has_a_tool_risk_label_branch() {
        let schema = schema();
        let branches = schema["properties"]["label_json"]["oneOf"]
            .as_array()
            .unwrap();
        let tool = branches
            .iter()
            .find(|b| b["required"] == serde_json::json!(["class"]))
            .expect("a tool_risk label branch");
        assert_eq!(
            tool["properties"]["class"]["enum"],
            serde_json::json!(["spend_or_destructive", "state_changing", "read_only"])
        );
        assert_eq!(tool["additionalProperties"], serde_json::json!(false));
    }

    // 068 T005 (plan D2a, D3, D7): questions_json entries may carry
    // `recommends` and `number_diff`; the row may carry `cooldown_suppressed`.
    #[test]
    fn the_contract_has_the_turn_signal_fields() {
        let schema = schema();
        let entry = &schema["properties"]["questions_json"]["items"]["properties"];
        assert!(entry["recommends"].is_object(), "recommends");
        assert!(entry["number_diff"].is_object(), "number_diff");
        assert!(entry["excluded_claims"].is_object(), "excluded_claims");
        let required: Vec<&str> = schema["properties"]["questions_json"]["items"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for field in ["recommends", "number_diff", "excluded_claims"] {
            assert!(!required.contains(&field), "{field} is required");
        }
        assert!(schema["properties"]["cooldown_suppressed"].is_object());
    }

    #[test]
    fn outcomes_are_the_published_list_plus_excluded() {
        for outcome in Outcome::ALL {
            let o = JevOutcome::Call(*outcome);
            assert_eq!(JevOutcome::parse(o.as_str()), Some(o));
        }
        assert_eq!(JevOutcome::parse("excluded"), Some(JevOutcome::Excluded));
        assert!(Outcome::ALL.iter().all(|o| o.as_str() != "excluded"));
        assert_eq!(JevOutcome::parse("nonsense"), None);
    }

    fn record(id: &str) -> JevRecord {
        JevRecord {
            id: id.into(),
            created_at: Utc.with_ymd_and_hms(2026, 11, 24, 12, 0, 0).unwrap(),
            server_session_id: "server-1".into(),
            harness_session_id: None,
            site: JevSite::ResearchClaims,
            mode: JevMode::Shadow,
            questions_json: serde_json::json!([
                { "id": "relation_1", "version": 1,
                  "option_order": ["says_nothing", "contradicts", "supports"],
                  "answer": "supports",
                  "probabilities": { "says_nothing": 0.1, "contradicts": 0.1, "supports": 0.8 },
                  "band": null }
            ]),
            model_requested: "jev-1.14.0".into(),
            model_answered: Some("jev-1.14.0".into()),
            outcome: JevOutcome::Call(Outcome::Success),
            error_class: None,
            request_id: Some("req-1".into()),
            latency_ms: Some(120),
            input_tokens: Some(300),
            cost_usd: Some(0.000_012_6),
            eval_id: None,
            invocation_id: Some("inv-1".into()),
            claim_key: Some("the sky is blue".into()),
            source_ids: Some(vec!["src-1".into()]),
            label_json: Some(serde_json::json!({
                "support": "confirmed", "votes_supported": 3, "votes_refuted": 0,
                "votes_completed": 3, "sources": 1
            })),
            label_source: Some("claude-opus-4-8/verify-v1".into()),
            label_cost_usd: None,
            cooldown_suppressed: None,
            state_json: Some(serde_json::json!({ "claim": "The sky is blue." })),
            state_hash: None,
            transcript_ref: None,
            redaction_count: 0,
            truncated: false,
            applied: false,
        }
    }

    #[tokio::test]
    async fn a_row_reads_back_unchanged() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let written = record("row-1");
        storage
            .record_jev(std::slice::from_ref(&written))
            .await
            .unwrap();
        assert_eq!(storage.load_jev_records().await.unwrap(), vec![written]);
    }

    // 068 T014 (FR-007, plan D3): the session's turn-signal kinds answered
    // yes (p >= 0.5) since a time; other sessions, sites, older rows and
    // answers below 0.5 are left out.
    #[tokio::test]
    async fn turn_signal_yes_kinds_are_this_sessions_yes_answers_since_a_time() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let since = Utc.with_ymd_and_hms(2026, 11, 24, 12, 0, 0).unwrap();
        let signal = |id: &str, session: &str, kind: &str, p: f64, minutes: i64| {
            let mut r = record(id);
            r.site = JevSite::TurnSignals;
            r.harness_session_id = Some(session.into());
            r.created_at = since + chrono::Duration::minutes(minutes);
            r.questions_json = serde_json::json!([
                { "id": kind, "version": 1, "option_order": ["true", "false"],
                  "answer": p, "probabilities": null, "band": null }
            ]);
            r
        };
        let mut other_site = signal("other-site", "s1", "pushback_reversal", 0.9, 5);
        other_site.site = JevSite::TurnReview;
        let mut rows = vec![
            signal("yes", "s1", "pushback_reversal", 0.9, 5),
            signal("edge", "s1", "unverified_fact", 0.5, 0),
            signal("no", "s1", "unverified_fact", 0.4, 6),
            signal("old", "s1", "unverified_fact", 0.9, -1),
            signal("other-session", "s2", "unverified_fact", 0.9, 5),
            other_site,
        ];
        // Delivered rows: each names a checkpoint record (068 review finding 1).
        for r in &mut rows {
            r.eval_id = Some(r.id.clone());
            storage.record_checkpoint(&delivered(&r.id)).await.unwrap();
        }
        storage.record_jev(&rows).await.unwrap();
        let mut kinds = storage
            .turn_signal_yes_kinds_since("s1", since)
            .await
            .unwrap();
        kinds.sort();
        assert_eq!(kinds, ["pushback_reversal", "unverified_fact"]);
        assert!(storage
            .turn_signal_yes_kinds_since("s3", since)
            .await
            .unwrap()
            .is_empty());
    }

    // 068 review finding 2: as the real cooldown, a yes the cooldown itself
    // suppressed does not count; a row with no cooldown record (null) does.
    #[tokio::test]
    async fn a_suppressed_yes_does_not_count_toward_the_cooldown() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let since = Utc.with_ymd_and_hms(2026, 11, 24, 12, 0, 0).unwrap();
        let signal = |id: &str, kind: &str, suppressed: Option<Vec<String>>| {
            let mut r = record(id);
            r.site = JevSite::TurnSignals;
            r.harness_session_id = Some("s1".into());
            r.created_at = since + chrono::Duration::minutes(5);
            r.questions_json = serde_json::json!([
                { "id": kind, "version": 1, "option_order": ["true", "false"],
                  "answer": 0.9, "probabilities": null, "band": null }
            ]);
            r.cooldown_suppressed = suppressed;
            r
        };
        let mut rows = vec![
            signal(
                "suppressed",
                "pushback_reversal",
                Some(vec!["pushback_reversal".into()]),
            ),
            signal("delivered", "unverified_fact", Some(vec![])),
        ];
        // Delivered rows: each names a checkpoint record (068 review finding 1).
        for r in &mut rows {
            r.eval_id = Some(r.id.clone());
            storage.record_checkpoint(&delivered(&r.id)).await.unwrap();
        }
        storage.record_jev(&rows).await.unwrap();
        assert_eq!(
            storage
                .turn_signal_yes_kinds_since("s1", since)
                .await
                .unwrap(),
            ["unverified_fact"]
        );
    }

    /// A checkpoint record with `id`, so a turn-signal row that names it
    /// counts as delivered.
    fn delivered(id: &str) -> crate::checkpoint::CheckpointRecord {
        crate::checkpoint::CheckpointRecord {
            id: id.into(),
            session_id: "s1".into(),
            boundary: crate::checkpoint::Boundary::Turn,
            signals_evaluated: vec![],
            signals_fired: vec![],
            delivered_keys: vec![],
            review_ran: false,
            verdict: crate::checkpoint::Verdict::Silence,
            suppressed: false,
            fail_open: false,
            latency_ms: 0,
            cost_usd: 0.0,
            jev_cost_usd: None,
            review_skipped_by: None,
            created_at: Utc.with_ymd_and_hms(2026, 11, 24, 12, 0, 0).unwrap(),
        }
    }

    // 068 review finding 1 (timing fix): a turn call Claude Code cancelled
    // writes rows but no checkpoint record; its yes was never delivered, so
    // only rows whose eval_id has a checkpoint record count.
    #[tokio::test]
    async fn a_yes_from_a_cancelled_turn_call_does_not_count_toward_the_cooldown() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let since = Utc.with_ymd_and_hms(2026, 11, 24, 12, 0, 0).unwrap();
        let signal = |id: &str, kind: &str, eval_id: &str| {
            let mut r = record(id);
            r.site = JevSite::TurnSignals;
            r.harness_session_id = Some("s1".into());
            r.created_at = since + chrono::Duration::minutes(5);
            r.eval_id = Some(eval_id.into());
            r.questions_json = serde_json::json!([
                { "id": kind, "version": 1, "option_order": ["true", "false"],
                  "answer": 0.9, "probabilities": null, "band": null }
            ]);
            r
        };
        storage
            .record_checkpoint(&delivered("eval-recorded"))
            .await
            .unwrap();
        let rows = vec![
            signal("recorded", "pushback_reversal", "eval-recorded"),
            signal("cancelled", "unverified_fact", "eval-cancelled"),
        ];
        storage.record_jev(&rows).await.unwrap();
        assert_eq!(
            storage
                .turn_signal_yes_kinds_since("s1", since)
                .await
                .unwrap(),
            ["pushback_reversal"]
        );
    }

    #[tokio::test]
    async fn an_applied_injection_row_reads_back_applied() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let mut written = record("row-i");
        written.site = JevSite::ResearchInjection;
        written.mode = JevMode::On;
        written.applied = true;
        written.label_json = Some(serde_json::json!({ "patterns063": ["role_marker"] }));
        written.state_json = None;
        storage
            .record_jev(std::slice::from_ref(&written))
            .await
            .unwrap();
        let read = storage.load_jev_records().await.unwrap();
        assert_eq!(read, vec![written]);
        assert!(read[0].applied);
    }

    #[tokio::test]
    async fn a_batch_is_written_all_or_none() {
        let storage = SqliteStorage::connect(":memory:").await.unwrap();
        let batch = [record("a"), record("b"), record("c")];
        storage.record_jev(&batch).await.unwrap();
        assert_eq!(storage.load_jev_records().await.unwrap().len(), 3);

        // The second row repeats an existing id, so the whole batch fails.
        let failing = [record("d"), record("a"), record("e")];
        assert!(storage.record_jev(&failing).await.is_err());
        let ids: BTreeSet<String> = storage
            .load_jev_records()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["a", "b", "c"].map(String::from).into());
    }
}
