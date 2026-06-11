//! The model-client boundary.
//!
//! The core contract of the server is **constrained output**: a prompt plus an
//! output JSON Schema go in, and schema-valid JSON comes back. There is no
//! free-text parsing on the happy path — the schema is guaranteed by the
//! provider's structured-output / tool-use feature.

use crate::error::AppError;
use serde_json::Value;

/// An LLM backend that returns schema-constrained JSON.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait ModelClient: Send + Sync {
    /// Run `prompt` and return JSON conforming to `schema`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] if the upstream call fails or a schema-valid
    /// response cannot be produced.
    async fn complete(&self, prompt: &str, schema: &Value) -> Result<Value, AppError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mock_model_client_returns_constrained_value() {
        let mut mock = MockModelClient::new();
        mock.expect_complete()
            .returning(|_, _| Ok(json!({ "ok": true })));

        let out = mock.complete("hello", &json!({})).await.unwrap();
        assert_eq!(out, json!({ "ok": true }));
    }
}
