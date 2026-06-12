//! The persistence boundary. SQLite-backed in production; mocked in tests.

use crate::error::AppError;
use crate::telemetry::InvocationRecord;
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
