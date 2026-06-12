//! Thin Anthropic client targeting native structured outputs.
//!
//! Deliberately hand-rolled over `reqwest` (research.md D2): no official
//! Anthropic Rust SDK exists, and the structured-outputs surface is small. The
//! request is `output_config.format` (JSON Outputs mode, validated live by
//! `examples/spike_client.rs`); `stop_reason` is checked before the body is
//! trusted, and each terminal condition maps to its outcome class.
//!
//! Retry policy: HTTP 429/5xx and transport errors retry with exponential
//! backoff up to `MAX_RETRIES`; a per-request timeout is terminal (`Timeout` —
//! it already consumed the full configured budget); other 4xx are terminal
//! (`Client` — the request itself is wrong, retrying cannot help).

use crate::config::Config;
use crate::error::AppError;
use crate::traits::client::{Completion, ModelClient};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Output budget per verification pass — far above any flat verdict payload,
/// far below runaway generation.
const MAX_TOKENS: u32 = 4096;

/// Thin `reqwest` client implementing [`ModelClient`] via structured outputs.
pub struct AnthropicClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    timeout_ms: u64,
    max_retries: u32,
    backoff_base_ms: u64,
}

impl AnthropicClient {
    /// Build a client from configuration, targeting the production endpoint.
    #[must_use]
    pub fn new(config: &Config) -> Self {
        Self::with_base_url(config, ANTHROPIC_API_BASE)
    }

    /// Build a client against a custom endpoint (tests point this at a local
    /// wiremock server; nothing else should override it).
    #[must_use]
    pub fn with_base_url(config: &Config, base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: config.anthropic_api_key.clone(),
            model: config.anthropic_model.clone(),
            timeout_ms: config.request_timeout_ms,
            max_retries: config.max_retries,
            backoff_base_ms: 200,
        }
    }

    /// Shrink the retry backoff base (test-only speedup).
    #[doc(hidden)]
    #[must_use]
    pub const fn with_backoff_base_ms(mut self, ms: u64) -> Self {
        self.backoff_base_ms = ms;
        self
    }

    async fn send_once(&self, body: &Value) -> Result<reqwest::Response, AppError> {
        self.http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .timeout(Duration::from_millis(self.timeout_ms))
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        ms: self.timeout_ms,
                    }
                } else {
                    // Transport-level failure (connect refused, reset) — retryable.
                    AppError::Client(format!("transport: {e}"))
                }
            })
    }
}

#[async_trait::async_trait]
impl ModelClient for AnthropicClient {
    async fn complete(&self, prompt: &str, schema: &Value) -> Result<Completion, AppError> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "messages": [{ "role": "user", "content": prompt }],
            "output_config": { "format": { "type": "json_schema", "schema": schema } },
        });

        let attempts_max = self.max_retries.saturating_add(1);
        let mut last_error = String::new();

        for attempt in 1..=attempts_max {
            if attempt > 1 {
                let backoff = self
                    .backoff_base_ms
                    .saturating_mul(1 << (attempt - 2).min(8));
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }

            let response = match self.send_once(&body).await {
                Ok(r) => r,
                // A timeout consumed the full per-request budget — terminal.
                Err(timeout @ AppError::Timeout { .. }) => return Err(timeout),
                Err(e) => {
                    last_error = e.to_string();
                    continue;
                }
            };

            let status = response.status();
            if status.as_u16() == 429 || status.is_server_error() {
                last_error = format!("HTTP {status}");
                continue;
            }
            if !status.is_success() {
                let detail = response.text().await.unwrap_or_default();
                return Err(AppError::Client(format!("HTTP {status}: {detail}")));
            }

            // reqwest's .timeout() covers the body read too — a timeout that
            // elapses here is still a Timeout, not an out-of-contract response.
            let payload: MessagesResponse = response.json().await.map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        ms: self.timeout_ms,
                    }
                } else {
                    AppError::Client(format!("response body unreadable: {e}"))
                }
            })?;
            return interpret(&payload);
        }

        Err(AppError::RetriesExhausted {
            attempts: attempts_max,
            last: last_error,
        })
    }
}

/// Map a 2xx Messages response to a [`Completion`] or its outcome class.
fn interpret(payload: &MessagesResponse) -> Result<Completion, AppError> {
    match payload.stop_reason.as_deref() {
        Some("end_turn") => {
            let text = payload.first_text().ok_or_else(|| {
                AppError::Client("out-of-contract provider response: no text block".to_string())
            })?;
            let value = serde_json::from_str(text).map_err(|e| {
                AppError::Client(format!(
                    "out-of-contract provider response: constrained body failed to parse: {e}"
                ))
            })?;
            Ok(Completion {
                value,
                input_tokens: payload.usage.input_tokens,
                output_tokens: payload.usage.output_tokens,
            })
        }
        Some("refusal") => Err(AppError::Refusal(
            payload
                .first_text()
                .unwrap_or("the provider declined to answer")
                .to_string(),
        )),
        Some("max_tokens") => Err(AppError::Truncation(format!(
            "output budget exhausted after {} output tokens",
            payload.usage.output_tokens
        ))),
        other => Err(AppError::Client(format!(
            "out-of-contract provider response: unexpected stop_reason: {other:?}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Usage,
}

impl MessagesResponse {
    /// First `text` content block — with thinking enabled it is not
    /// necessarily `content[0]` (spike 4 finding).
    fn first_text(&self) -> Option<&str> {
        self.content.iter().find_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Other => None,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text {
        text: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn test_config() -> Config {
        Config {
            anthropic_api_key: "test-key".into(),
            anthropic_model: "claude-opus-4-8".into(),
            verify_ensemble_k: 3,
            input_max_chars: 50_000,
            voyage_api_key: None,
            voyage_model: "voyage-4".into(),
            memory_recall_limit: 5,
            brave_api_key: None,
            fetch_timeout_ms: 10_000,
            research_concurrency: 8,
            database_path: ":memory:".into(),
            log_level: "info".into(),
            request_timeout_ms: 2_000,
            max_retries: 2,
        }
    }

    fn client_for(mock: &MockServer) -> AnthropicClient {
        AnthropicClient::with_base_url(&test_config(), &mock.uri()).with_backoff_base_ms(1)
    }

    fn end_turn_body(json_text: &str) -> serde_json::Value {
        json!({
            "content": [{ "type": "text", "text": json_text }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 100, "output_tokens": 25 }
        })
    }

    #[tokio::test]
    async fn end_turn_parses_value_and_usage() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("anthropic-version", ANTHROPIC_VERSION))
            .respond_with(ResponseTemplate::new(200).set_body_json(end_turn_body(r#"{"ok":true}"#)))
            .mount(&mock)
            .await;

        let out = client_for(&mock).complete("p", &json!({})).await.unwrap();
        assert_eq!(out.value, json!({ "ok": true }));
        assert_eq!((out.input_tokens, out.output_tokens), (100, 25));
    }

    #[tokio::test]
    async fn request_carries_constrained_output_config() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                assert_eq!(body["output_config"]["format"]["type"], "json_schema");
                assert_eq!(body["output_config"]["format"]["schema"]["type"], "object");
                ResponseTemplate::new(200).set_body_json(end_turn_body("{}"))
            })
            .mount(&mock)
            .await;

        client_for(&mock)
            .complete("p", &json!({ "type": "object" }))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn refusal_is_its_own_class() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [],
                "stop_reason": "refusal",
                "usage": { "input_tokens": 10, "output_tokens": 0 }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Refusal(_)), "got: {err}");
    }

    #[tokio::test]
    async fn max_tokens_is_truncation_not_a_parse_attempt() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{ "type": "text", "text": "{\"partial\":" }],
                "stop_reason": "max_tokens",
                "usage": { "input_tokens": 10, "output_tokens": 4096 }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Truncation(_)), "got: {err}");
    }

    #[tokio::test]
    async fn persistent_5xx_exhausts_retries_with_attempt_count() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3) // max_retries = 2 → 3 attempts total
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        match err {
            AppError::RetriesExhausted { attempts, ref last } => {
                assert_eq!(attempts, 3);
                assert!(last.contains("503"), "last error: {last}");
            }
            other => panic!("expected RetriesExhausted, got {other}"),
        }
    }

    #[tokio::test]
    async fn recovers_when_a_retry_succeeds() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(end_turn_body(r#"{"ok":1}"#)))
            .mount(&mock)
            .await;

        let out = client_for(&mock).complete("p", &json!({})).await.unwrap();
        assert_eq!(out.value, json!({ "ok": 1 }));
    }

    #[tokio::test]
    async fn slow_provider_is_a_timeout_not_a_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(end_turn_body("{}"))
                    .set_delay(Duration::from_secs(10)),
            )
            .expect(1) // terminal: no second attempt
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Timeout { ms: 2_000 }), "got: {err}");
    }

    #[tokio::test]
    async fn non_retryable_4xx_is_terminal_and_descriptive() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad schema"))
            .expect(1)
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        match err {
            AppError::Client(msg) => {
                assert!(msg.contains("400") && msg.contains("bad schema"), "{msg}");
            }
            other => panic!("expected Client, got {other}"),
        }
    }

    #[tokio::test]
    async fn unexpected_stop_reason_is_out_of_contract() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{ "type": "text", "text": "{}" }],
                "stop_reason": "pause_turn",
                "usage": {}
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Client(_)), "got: {err}");
        assert!(err.to_string().contains("pause_turn"));
    }

    #[tokio::test]
    async fn unparseable_end_turn_body_is_out_of_contract() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(end_turn_body("not json at all")),
            )
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Client(_)), "got: {err}");
    }

    #[tokio::test]
    async fn text_block_is_found_after_thinking_blocks() {
        // Spike 4: with adaptive thinking the text block follows thinking blocks.
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [
                    { "type": "thinking", "thinking": "..." },
                    { "type": "text", "text": "{\"ok\":true}" }
                ],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 1, "output_tokens": 1 }
            })))
            .mount(&mock)
            .await;

        let out = client_for(&mock).complete("p", &json!({})).await.unwrap();
        assert_eq!(out.value, json!({ "ok": true }));
    }
}
