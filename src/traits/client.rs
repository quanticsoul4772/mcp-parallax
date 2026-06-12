//! The model-client boundary.
//!
//! The core contract of the server is **constrained output**: a prompt plus an
//! output JSON Schema go in, and schema-valid JSON comes back. There is no
//! free-text parsing on the happy path — the schema is guaranteed by the
//! provider's structured-output feature. The returned [`Completion`] carries
//! token usage so every pass is attributable on the invocation record.

use crate::error::AppError;
use serde_json::Value;

/// One constrained completion: the schema-conforming value plus its usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The parsed, schema-shaped JSON value.
    pub value: Value,
    /// Input tokens billed for this pass.
    pub input_tokens: u64,
    /// Output tokens billed for this pass.
    pub output_tokens: u64,
}

/// An LLM backend that returns schema-constrained JSON.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait ModelClient: Send + Sync {
    /// Run `prompt` and return JSON conforming to `schema` (the **sanitized**,
    /// grammar-legal schema — value-range checks against the unsanitized
    /// schema are the caller's job via [`crate::schema::validate`]).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] classified by the outcome taxonomy: `Refusal`,
    /// `Truncation`, `Timeout`, `RetriesExhausted`, or `Client` for
    /// out-of-contract responses.
    async fn complete(&self, prompt: &str, schema: &Value) -> Result<Completion, AppError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mock_model_client_returns_constrained_value() {
        let mut mock = MockModelClient::new();
        mock.expect_complete().returning(|_, _| {
            Ok(Completion {
                value: json!({ "ok": true }),
                input_tokens: 10,
                output_tokens: 5,
            })
        });

        let out = mock.complete("hello", &json!({})).await.unwrap();
        assert_eq!(out.value, json!({ "ok": true }));
        assert_eq!((out.input_tokens, out.output_tokens), (10, 5));
    }
}
