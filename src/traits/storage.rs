//! The persistence boundary. SQLite-backed in production; mocked in tests.

use crate::checkpoint::CheckpointRecord;
use crate::error::AppError;
use crate::memory::consolidate::ConsolidationRecord;
use crate::memory::push::PushRecord;
use crate::memory::{Memory, Status};
use crate::telemetry::InvocationRecord;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Durable storage for sessions, accumulated state, and invocation records.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// Persist a session blob under `id`, overwriting any prior value.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the write fails.
    async fn save_session(&self, id: &str, data: &Value) -> Result<(), AppError>;

    /// Load a session blob by `id`, or `None` if no such session exists.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the read fails.
    async fn load_session(&self, id: &str) -> Result<Option<Value>, AppError>;

    /// Persist one invocation record. Called exactly once per invocation, on
    /// every exit path (FR-010).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the write fails.
    async fn record_invocation(&self, record: &InvocationRecord) -> Result<(), AppError>;

    /// Persist one memory (memory capability).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the write fails.
    async fn save_memory(&self, memory: &Memory) -> Result<(), AppError>;

    /// Load every stored memory (ranking happens in process — research.md 003
    /// S1: brute force at v1 scale).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] on read failure or a contract-violating row.
    async fn load_memories(&self) -> Result<Vec<Memory>, AppError>;

    /// Permanently delete a memory by id; returns whether it existed.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the delete fails.
    async fn delete_memory(&self, id: &str) -> Result<bool, AppError>;

    /// Persist one checkpoint evaluation record (checkpoint layer, FR-006 —
    /// exactly one per evaluation).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the write fails.
    async fn record_checkpoint(&self, record: &CheckpointRecord) -> Result<(), AppError>;

    /// Signal keys delivered (verdict ≠ silence, not suppressed) in this
    /// session since `since` — the FR-010 cooldown lookup.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] on read failure or a contract-violating row.
    async fn delivered_signal_keys_since(
        &self,
        session_id: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<String>, AppError>;

    /// Persist one push evaluation record (016 FR-008 — exactly one per
    /// evaluation; the audit trail is also the suppression source).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the write fails.
    async fn record_push(&self, record: &PushRecord) -> Result<(), AppError>;

    /// Memory ids already surfaced in this session — the 016 FR-005
    /// once-per-session suppression lookup (union of the session's
    /// `push_records.surfaced_ids`).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] on read failure or a contract-violating row.
    async fn pushed_memory_ids(&self, session_id: &str) -> Result<Vec<String>, AppError>;

    /// Persist one consolidation audit record (017 FR-009 — exactly one per
    /// applied action / capture proposal / cap drop).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the write fails.
    async fn record_consolidation(&self, record: &ConsolidationRecord) -> Result<(), AppError>;

    /// Count of capture proposals already recorded for this session — the
    /// 017 `CAPTURE_SESSION_CAP` lookup.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] on read failure.
    async fn captures_in_session(&self, session_id: &str) -> Result<u32, AppError>;

    /// Update ONLY a memory's status columns (017 FR-010 — content columns
    /// are never written after admission).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the write fails.
    async fn update_memory_status(
        &self,
        id: &str,
        status: Status,
        replaced_by: Option<String>,
    ) -> Result<(), AppError>;

    /// Refresh `last_reinforced_at` for the given memories (017 research
    /// D5 — retrieval reinforcement; fire-and-forget at call sites).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the write fails.
    async fn touch_reinforcement(&self, ids: &[String], now: DateTime<Utc>)
        -> Result<(), AppError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mock_storage_honors_the_load_contract() {
        let mut mock = MockStorage::new();
        mock.expect_load_session()
            .returning(|_| Ok(Some(json!({ "seen": true }))));

        let got = mock.load_session("s1").await.unwrap();
        assert_eq!(got, Some(json!({ "seen": true })));
    }
}
