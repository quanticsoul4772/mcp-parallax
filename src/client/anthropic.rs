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

pub(crate) const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Output budget per model call.
///
/// **Derived, not guessed** (018 D7 measurement procedure). The largest mode
/// schema bounds its own output: research synthesis allows
/// `MAX_ANSWER_CHARS` (8 000) answer characters plus `MAX_GAPS` × `MAX_GAP_CHARS`
/// (10 × 500) of gaps — ~13 000 characters, or roughly **3 500 tokens** of
/// answer before any reasoning. Every other mode schema is smaller. The budget
/// is set to ≥ 4× that floor, leaving the remainder for models that reason
/// before answering.
///
/// That headroom is why 4096 no longer suffices: on Claude 5 families,
/// omitting `thinking` runs adaptive reasoning which is charged against this
/// same ceiling, so a verdict could be truncated before its JSON was emitted.
///
/// **Raised 16 000 → 32 000 (2026-07-24) after a real truncation.** The 018
/// family sweep declared 16 000 validated, but it exercised only trivial
/// inputs — a two-option `decide` with one sentence of context. A genuine
/// four-option `decide` with a long context on `claude-sonnet-5` exhausted the
/// ceiling. The schema floor bounds only the *answer*; on a model that reasons
/// before answering the reasoning term is the larger one and is not bounded by
/// any schema.
///
/// This value is **not** derived from measurement, and saying so matters: the
/// largest successful thinking-inclusive output on record is 3 135 tokens,
/// while the failing call exceeded 16 000, and how far past it went is unknown
/// because truncated invocations currently record zero usage (see the
/// follow-up note in the branch description). 32 000 is ~9× the schema floor
/// and 2× the value that failed. It should be re-derived once truncation
/// records its real usage.
const MAX_TOKENS: u32 = 32_000;

/// Thin `reqwest` client implementing [`ModelClient`] via structured outputs.
pub struct AnthropicClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    /// Reasoning effort for this client's call sites (022). `None` sends no
    /// `effort` field, leaving the request byte-identical to pre-022.
    effort: Option<crate::routing::Effort>,
    timeout_ms: u64,
    max_retries: u32,
    backoff_base_ms: u64,
}

impl AnthropicClient {
    /// Build a client from configuration, targeting the production endpoint.
    #[must_use]
    pub fn new(config: &Config) -> Self {
        // 028: honour the configured base so no construction path bypasses it.
        Self::with_base_url(config, &config.anthropic_api_base)
    }

    /// Build a client for an explicitly named model, overriding
    /// `ANTHROPIC_MODEL` (018 — the client pool builds one of these per
    /// distinct routed model).
    #[must_use]
    pub fn for_model(config: &Config, model: &str) -> Self {
        Self {
            model: model.to_string(),
            ..Self::new(config)
        }
    }

    /// Build a client for a named model at a named reasoning effort (022).
    #[must_use]
    pub fn for_model_and_effort(
        config: &Config,
        model: &str,
        effort: Option<crate::routing::Effort>,
    ) -> Self {
        Self {
            effort,
            ..Self::for_model(config, model)
        }
    }

    /// Build a client for a named model and effort over an **existing**
    /// `reqwest::Client` (028 T001).
    ///
    /// 028 pre-builds one client per `(routed model, effort level)` pair so a
    /// per-call effort resolves to a lookup rather than a construction. That
    /// cross product is small — at most twelve models by six effort states —
    /// but [`Self::with_base_url`] calls `reqwest::Client::new()` each time,
    /// and every one of those owns a separate connection pool. Sharing the
    /// transport makes each additional entry cost a `String` and an
    /// `Option<Effort>` instead.
    ///
    /// `reqwest::Client` is explicitly documented as cheap to clone and
    /// intended to be reused; the clone shares the underlying pool.
    ///
    /// This is the base constructor — every other one delegates here — so no
    /// path builds a transport it then discards.
    #[must_use]
    pub fn with_http_client(
        config: &Config,
        http: &reqwest::Client,
        base_url: &str,
        model: &str,
        effort: Option<crate::routing::Effort>,
    ) -> Self {
        Self {
            http: http.clone(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: config.anthropic_api_key.clone(),
            model: model.to_string(),
            effort,
            timeout_ms: config.request_timeout_ms,
            max_retries: config.max_retries,
            backoff_base_ms: 200,
        }
    }

    /// Build a client against a custom endpoint (tests point this at a local
    /// wiremock server; nothing else should override it).
    #[must_use]
    pub fn with_base_url(config: &Config, base_url: &str) -> Self {
        Self::with_http_client(
            config,
            &reqwest::Client::new(),
            base_url,
            &config.anthropic_model,
            None,
        )
    }

    /// Shrink the retry backoff base (test-only speedup).
    #[doc(hidden)]
    #[must_use]
    pub const fn with_backoff_base_ms(mut self, ms: u64) -> Self {
        self.backoff_base_ms = ms;
        self
    }

    /// Name the operator's setting when the provider rejects the effort
    /// parameter (027).
    ///
    /// The provider's message describes its own view — *this model* does not
    /// support the parameter. The operator needs the other half: which of their
    /// settings sent it. This client holds both facts the message omits, so it
    /// is where the two can be joined.
    ///
    /// **Appended, never substituted.** The hint is an inference about *why* a
    /// request was rejected, and a confident wrong diagnosis in front of the
    /// operator is worse than the bare message. The provider's own text always
    /// survives beside it.
    ///
    /// The guard is deliberately narrow — a client-error status, an effort
    /// actually configured, and a body naming the parameter. If the provider
    /// rewords its rejection the guard stops matching and the message degrades
    /// to what it was before this change, which is the safe direction: a lost
    /// hint costs nothing, a false one misdirects.
    ///
    /// Naming the *variable* rather than the model would need one client per
    /// call site, discarding the pooling that keys on `(model, effort)`. Given
    /// the model and the level, the responsible setting is immediate.
    ///
    /// **All three sources are named because this client cannot tell which one
    /// applied** (030). 027 wrote this message when the environment namespace
    /// was the only way to set an effort; 028 added a per-call argument, and
    /// the pool serves the *same* client for a configured `low` and a
    /// caller-supplied `low` — that sharing is the point of keying on
    /// `(model, effort)`, so provenance is genuinely not recoverable here.
    ///
    /// Listing beats guessing, and the live verification of 027 landed on
    /// exactly the case the old text got wrong: a per-call `effort` with no
    /// variable set anywhere, told to unset a variable that did not exist while
    /// the remedy that applied went unmentioned. The per-call argument is named
    /// first because dropping it is the only remedy needing no restart.
    fn effort_rejection_hint(&self, status: reqwest::StatusCode, body: &str) -> String {
        let Some(effort) = self.effort else {
            return String::new();
        };
        if !status.is_client_error() || !body.to_lowercase().contains("effort") {
            return String::new();
        }
        format!(
            " — parallax sent effort=`{}` to `{}`, which does not accept it. \
             Remove the `effort` argument from this call, unset the \
             PARALLAX_EFFORT_* variable covering this call site, or route the \
             site to a model that accepts effort.",
            effort.as_str(),
            self.model
        )
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
                        what: "request",
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
        // 022: `effort` joins `format` under `output_config` only when the
        // operator set one. Unset omits the key entirely, so an unrouted
        // deployment sends exactly the pre-022 body.
        let mut output_config = json!({ "format": { "type": "json_schema", "schema": schema } });
        if let (Some(effort), Some(map)) = (self.effort, output_config.as_object_mut()) {
            map.insert("effort".to_string(), json!(effort.as_str()));
        }
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "messages": [{ "role": "user", "content": prompt }],
            "output_config": output_config,
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
                return Err(AppError::Client(format!(
                    "HTTP {status}: {detail}{}",
                    self.effort_rejection_hint(status, &detail)
                )));
            }

            // reqwest's .timeout() covers the body read too — a timeout that
            // elapses here is still a Timeout, not an out-of-contract response.
            let payload: MessagesResponse = response.json().await.map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        what: "request",
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
///
/// Every failure here is a 200 that was billed — the provider ran the model
/// and returned a `stop_reason` the contract cannot use. Each error carries
/// that usage (020); before this, a truncated call recorded zero tokens, which
/// under-reported spend and made the output ceiling unsizable from data.
fn interpret(payload: &MessagesResponse) -> Result<Completion, AppError> {
    let billed =
        |error: AppError| error.metered(payload.usage.input_tokens, payload.usage.output_tokens);
    match payload.stop_reason.as_deref() {
        Some("end_turn") => {
            let text = payload.first_text().ok_or_else(|| {
                billed(AppError::Client(
                    "out-of-contract provider response: no text block".to_string(),
                ))
            })?;
            let value = serde_json::from_str(text).map_err(|e| {
                billed(AppError::Client(format!(
                    "out-of-contract provider response: constrained body failed to parse: {e}"
                )))
            })?;
            Ok(Completion {
                value,
                input_tokens: payload.usage.input_tokens,
                output_tokens: payload.usage.output_tokens,
            })
        }
        Some("refusal") => Err(billed(AppError::Refusal(
            payload
                .first_text()
                .unwrap_or("the provider declined to answer")
                .to_string(),
        ))),
        Some("max_tokens") => Err(billed(AppError::Truncation(format!(
            "output budget exhausted after {} output tokens",
            payload.usage.output_tokens
        )))),
        other => Err(billed(AppError::Client(format!(
            "out-of-contract provider response: unexpected stop_reason: {other:?}"
        )))),
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
            anthropic_api_base: "http://127.0.0.1:1".into(),
            routing: crate::routing::RoutingTable::single("claude-opus-4-8"),
            verify_ensemble_k: 3,
            input_max_chars: 50_000,
            voyage_api_key: None,
            voyage_model: "voyage-4".into(),
            memory_recall_limit: 5,
            brave_api_key: None,
            fetch_timeout_ms: 10_000,
            research_concurrency: 8,
            fetch_allow_private: false,
            checkpoint_gate_patterns: vec![],
            grounded_verify_root: None,
            grounded_verify_max_bytes: 262_144,
            grounded_verify_max_locators: 64,
            database_path: ":memory:".into(),
            log_level: "info".into(),
            request_timeout_ms: 2_000,
            max_retries: 2,
        }
    }

    fn client_for(mock: &MockServer) -> AnthropicClient {
        AnthropicClient::with_base_url(&test_config(), &mock.uri()).with_backoff_base_ms(1)
    }

    /// 028 T002 / D1: the eager `(model, effort)` cross product shares one
    /// transport. Each entry must still be independently configured — if the
    /// shared client leaked model or effort between entries, a per-call effort
    /// would silently ride on the next call.
    #[tokio::test]
    async fn clients_sharing_one_transport_stay_independently_configured() {
        let config = test_config();
        let http = reqwest::Client::new();
        let mock = MockServer::start().await;

        let low = AnthropicClient::with_http_client(
            &config,
            &http,
            &mock.uri(),
            "claude-haiku-4-5",
            Some(crate::routing::Effort::Low),
        );
        let none =
            AnthropicClient::with_http_client(&config, &http, &mock.uri(), "claude-opus-5", None);

        assert_eq!(low.model, "claude-haiku-4-5");
        assert_eq!(low.effort, Some(crate::routing::Effort::Low));
        assert_eq!(none.model, "claude-opus-5");
        assert_eq!(
            none.effort, None,
            "absent must stay absent, not inherit the sibling's level"
        );

        // Both reach the same endpoint over the shared transport.
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(end_turn_body(r#"{"ok":true}"#)))
            .mount(&mock)
            .await;
        assert!(low.complete("p", &json!({})).await.is_ok());
        assert!(none.complete("p", &json!({})).await.is_ok());
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

    /// 022: an unrouted client sends no `effort` key at all. This is the
    /// off-by-default guarantee — the request body is byte-identical to
    /// pre-022, so enabling the feature is an operator decision and never a
    /// side effect of upgrading.
    #[tokio::test]
    async fn without_a_routed_effort_the_request_carries_no_effort_key() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let output_config = body["output_config"].as_object().unwrap();
                assert!(
                    !output_config.contains_key("effort"),
                    "unset effort must not appear on the wire: {output_config:?}"
                );
                let mut keys: Vec<&str> = output_config.keys().map(String::as_str).collect();
                keys.sort_unstable();
                assert_eq!(keys, ["format"]);
                ResponseTemplate::new(200).set_body_json(end_turn_body("{}"))
            })
            .mount(&mock)
            .await;

        client_for(&mock).complete("p", &json!({})).await.unwrap();
    }

    /// 022: a routed effort reaches `output_config.effort` in its wire
    /// spelling, alongside the format rather than replacing it.
    #[tokio::test]
    async fn a_routed_effort_reaches_the_request_body() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                assert_eq!(body["output_config"]["effort"], "low");
                // The constrained-output contract survives alongside it.
                assert_eq!(body["output_config"]["format"]["type"], "json_schema");
                ResponseTemplate::new(200).set_body_json(end_turn_body("{}"))
            })
            .mount(&mock)
            .await;

        let client = AnthropicClient {
            effort: Some(crate::routing::Effort::Low),
            ..client_for(&mock)
        };
        client.complete("p", &json!({})).await.unwrap();
    }

    /// 028 T016 / FR-003, SC-002: silence stays byte-identical.
    ///
    /// 022 proved an unset *namespace* sends no `effort` key. 028 adds a layer
    /// above it, so the guarantee now has to survive a caller that also said
    /// nothing — the case that is by far the most common and the one a
    /// regression would hide in. Asserting on the whole serialized body rather
    /// than on `output_config.effort` alone is deliberate: a key appearing
    /// anywhere else would pass the narrower check.
    #[tokio::test]
    async fn neither_configuration_nor_caller_means_no_effort_key_anywhere() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                assert!(
                    !serde_json::to_string(&body).unwrap().contains("effort"),
                    "no layer set an effort, so the word must not appear: {body}"
                );
                assert!(body["output_config"].get("effort").is_none());
                // The constrained-output contract is untouched by its absence.
                assert_eq!(body["output_config"]["format"]["type"], "json_schema");
                ResponseTemplate::new(200).set_body_json(end_turn_body("{}"))
            })
            .mount(&mock)
            .await;

        // `client_for` carries no effort, standing in for both layers unset.
        client_for(&mock).complete("p", &json!({})).await.unwrap();
    }

    /// 027 / FR-001, FR-002, FR-003: the provider says *this model*; the
    /// operator needs to know which of their settings sent it. This is the
    /// production failure of 2026-07-25 — `PARALLAX_EFFORT_BULK=low` on a tier
    /// routed to a model that rejects the parameter.
    #[tokio::test]
    async fn an_effort_rejection_names_the_model_and_the_level() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "type": "error",
                "error": { "type": "invalid_request_error",
                           "message": "This model does not support the effort parameter." }
            })))
            .mount(&mock)
            .await;

        let client = AnthropicClient {
            effort: Some(crate::routing::Effort::Low),
            model: "claude-haiku-4-5".to_string(),
            ..client_for(&mock)
        };
        let err = client.complete("p", &json!({})).await.unwrap_err();
        let message = err.to_string();

        // FR-001: both facts the provider's message omits.
        assert!(message.contains("claude-haiku-4-5"), "{message}");
        assert!(message.contains("effort=`low`"), "{message}");
        // FR-002, amended by 030: **all three** remedies, because the client
        // cannot tell which source set the effort. The per-call argument was
        // missing until the live verification of 027 hit precisely that case —
        // a caller-supplied level with no variable set anywhere, told to unset
        // a variable that did not exist. It is asserted first because dropping
        // the argument is the only remedy that needs no restart.
        assert!(
            message.contains("Remove the `effort` argument"),
            "the per-call source is a remedy too (028 added it): {message}"
        );
        assert!(message.contains("PARALLAX_EFFORT_"), "{message}");
        assert!(message.contains("route the site"), "{message}");
        // FR-003: the provider's own text survives beside the diagnosis.
        assert!(
            message.contains("This model does not support the effort parameter"),
            "the provider's message must not be replaced: {message}"
        );
    }

    /// 027 / FR-004: the hint is an inference about *why* a request failed, so
    /// it must not appear on a failure it cannot explain. A confident wrong
    /// diagnosis is worse than the bare message.
    #[tokio::test]
    async fn a_rejection_the_hint_cannot_explain_is_left_alone() {
        // (a) no effort configured — nothing to blame.
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": { "message": "This model does not support the effort parameter." }
            })))
            .mount(&mock)
            .await;
        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(
            !err.to_string().contains("parallax sent effort"),
            "no effort was configured: {err}"
        );

        // (b) effort configured, but the rejection is about something else.
        let other = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": { "message": "max_tokens: must be greater than 0" }
            })))
            .mount(&other)
            .await;
        let client = AnthropicClient {
            effort: Some(crate::routing::Effort::Low),
            ..client_for(&other)
        };
        let err = client.complete("p", &json!({})).await.unwrap_err();
        assert!(
            !err.to_string().contains("parallax sent effort"),
            "the rejection does not name the parameter: {err}"
        );
        assert!(err.to_string().contains("max_tokens"), "{err}");
    }

    /// 027 / FR-004: a server-side failure is not a configuration mistake, and
    /// saying so would send the operator to edit a setting that is fine.
    #[tokio::test]
    async fn a_server_error_is_never_blamed_on_the_effort_setting() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("effort effort effort"))
            .mount(&mock)
            .await;
        let client = AnthropicClient {
            effort: Some(crate::routing::Effort::Max),
            max_retries: 0,
            ..client_for(&mock)
        };
        let err = client.complete("p", &json!({})).await.unwrap_err();
        // A 5xx exhausts retries rather than returning Client, but either way
        // the effort diagnosis must not appear — the body naming the word is
        // not enough when the status says the fault is the provider's.
        assert!(
            !err.to_string().contains("parallax sent effort"),
            "a server error is not a configuration mistake: {err}"
        );
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

    // T034 / 018 FR-014, D7: one request shape every family accepts.
    //
    // The families disagree about `thinking` in a way that admits exactly one
    // universally-accepted shape. Opus 5 and Sonnet 5 would accept
    // `thinking: {"type": "disabled"}`; Fable 5 rejects it with a 400 at any
    // effort. Omitting the field works everywhere — so the server must not
    // start sending one as an "optimization".
    #[tokio::test]
    async fn request_sends_no_thinking_field_so_every_family_accepts_it() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                assert!(
                    body.get("thinking").is_none(),
                    "no `thinking` field: Fable 5 rejects an explicit disable, \
                     so omission is the only shape every family accepts"
                );
                // Sampling parameters are removed on the Claude 5 families too.
                for removed in ["temperature", "top_p", "top_k"] {
                    assert!(body.get(removed).is_none(), "{removed} must not be sent");
                }
                ResponseTemplate::new(200).set_body_json(end_turn_body("{}"))
            })
            .mount(&mock)
            .await;

        client_for(&mock)
            .complete("p", &json!({ "type": "object" }))
            .await
            .unwrap();
    }

    // T035 / 018 FR-013, D7 step 1: the output budget clears the schema-derived
    // answer floor with room for reasoning that shares the same ceiling.
    #[tokio::test]
    async fn output_budget_clears_the_schema_floor_with_headroom() {
        // The largest mode schema bounds its own output — computed from the
        // schemas, not observed, so this check needs no network.
        let floor_chars = crate::research::MAX_ANSWER_CHARS
            + crate::research::MAX_GAPS * crate::research::MAX_GAP_CHARS;
        // A conservative chars-per-token figure; the real ratio is higher, so
        // this over-estimates the floor and under-estimates the headroom.
        let floor_tokens = floor_chars / 4;
        assert!(
            u64::from(MAX_TOKENS) >= 4 * floor_tokens as u64,
            "budget {MAX_TOKENS} must be >= 4x the {floor_tokens}-token answer floor"
        );

        // And the budget actually reaches the wire.
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                assert_eq!(body["max_tokens"], serde_json::json!(MAX_TOKENS));
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
        assert!(matches!(err.root(), AppError::Refusal(_)), "got: {err}");
        // 020: the provider ran the model and billed for it. Those tokens
        // reach the record instead of being dropped on the floor.
        assert_eq!(err.billed(), (10, 0));
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
        assert!(matches!(err.root(), AppError::Truncation(_)), "got: {err}");
        // 020: this is the case that made the output ceiling unsizable — a
        // truncated call recorded zero tokens, so nothing said how much
        // headroom the call actually wanted.
        assert_eq!(err.billed(), (10, 4096));
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
        assert!(
            matches!(err, AppError::Timeout { ms: 2_000, .. }),
            "got: {err}"
        );
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
        assert!(matches!(err.root(), AppError::Client(_)), "got: {err}");
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
        assert!(matches!(err.root(), AppError::Client(_)), "got: {err}");
        // An out-of-contract body is still a 200 the provider billed for.
        assert_eq!(err.billed(), (100, 25));
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
