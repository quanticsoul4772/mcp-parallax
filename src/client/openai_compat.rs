//! Thin OpenAI-compatible client: Chat Completions over any conforming
//! endpoint (OpenAI, Azure's OpenAI-compatible surface, Ollama, vLLM, LM
//! Studio).
//!
//! BYOM (design §3.1–§3.3): the second [`crate::traits::client::ModelClient`]
//! adapter, so a deployment can run the full tool catalog against a
//! non-Anthropic model with config changes only. The trait seam is unchanged —
//! `complete(prompt, schema)` — and so is the caller contract: schema-valid
//! JSON in, [`Completion`] with usage out.
//!
//! Three translation duties, each tested against wiremock:
//!
//! - **Structured output** (§3.2): a capability ladder — `response_format`
//!   JSON-schema → JSON-object + schema-in-prompt → forced tool call →
//!   prompt-only. `OPENAI_STRUCTURED_OUTPUT` pins one rung; `auto` walks the
//!   ladder, degrading exactly one rung when the endpoint rejects the strategy
//!   parameter itself.
//! - **Outcome taxonomy** (§3.3): `finish_reason: "length"` → `Truncation`,
//!   `"content_filter"` or a `refusal` field → `Refusal`, any other signal or
//!   an empty body → out-of-contract `Client`. The Anthropic mapping is the
//!   reference; `client::parity_tests` asserts both adapters agree row by row.
//! - **Usage** (§3.3): `prompt_tokens`/`completion_tokens` map to
//!   `Completion`; an omitted usage block records zeros **and** warns — an
//!   unattributed bill is never silent.
//!
//! Two deliberate non-translations, documented rather than guessed:
//!
//! - `effort` is dropped at the wire. Anthropic's `output_config.effort` has no
//!   Chat Completions spelling worth trusting (the levels do not even map
//!   1:1), so the adapter drops it with a notice instead of mistranslating it
//!   (§3.4). "Same effort" on another backend would not be the same
//!   computation anyway (§6.5).
//! - There is no client-side output ceiling. `max_tokens` vs
//!   `max_completion_tokens` is exactly the kind of compat disagreement this
//!   adapter exists to route around, and a ceiling sized for one model's
//!   context is a hard 400 on another's. The ceiling is provider-side; a
//!   provider-truncating `length` finish still bills as `Truncation`, which is
//!   what the ceiling existed to classify.

use crate::config::{Config, StructuredOutput};
use crate::error::AppError;
use crate::traits::client::{Completion, ModelClient};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// One wire strategy on the §3.2 ladder, in preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rung {
    /// `response_format: {type: "json_schema", json_schema: {...}}`.
    JsonSchema,
    /// `response_format: {type: "json_object"}` + the schema in the prompt.
    JsonObject,
    /// A forced single-function tool call; `function.arguments` carries the
    /// JSON.
    ToolShim,
    /// The schema in the prompt only.
    PromptOnly,
}

/// The rungs to try for a strategy selection, highest fidelity first.
fn ladder_for(selection: StructuredOutput) -> Vec<Rung> {
    match selection {
        StructuredOutput::Auto => vec![
            Rung::JsonSchema,
            Rung::JsonObject,
            Rung::ToolShim,
            Rung::PromptOnly,
        ],
        StructuredOutput::JsonSchema => vec![Rung::JsonSchema],
        StructuredOutput::JsonObject => vec![Rung::JsonObject],
        StructuredOutput::ToolShim => vec![Rung::ToolShim],
        StructuredOutput::PromptOnly => vec![Rung::PromptOnly],
    }
}

/// Thin `reqwest` client implementing [`ModelClient`] over Chat Completions.
pub struct OpenAiCompatClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    /// Routed reasoning effort — dropped at the wire on this adapter (§3.4).
    effort: Option<crate::routing::Effort>,
    /// The structured-output strategy (§3.2).
    structured: StructuredOutput,
    timeout_ms: u64,
    max_retries: u32,
    backoff_base_ms: u64,
    /// The effort no-op notice fires once per client, not per construction.
    effort_notice: AtomicBool,
}

impl OpenAiCompatClient {
    /// Build a client from configuration, targeting `OPENAI_API_BASE`.
    #[must_use]
    pub fn new(config: &Config) -> Self {
        Self::with_base_url(config, &config.openai_api_base)
    }

    /// Build a client for an explicitly named model, overriding `OPENAI_MODEL`
    /// (the pool builds one per distinct routed model, mirroring the
    /// Anthropic client).
    #[must_use]
    pub fn for_model(config: &Config, model: &str) -> Self {
        Self {
            model: model.to_string(),
            ..Self::new(config)
        }
    }

    /// Build a client for a named model at a named reasoning effort. The
    /// effort is accepted for symmetry with the pool's keying and dropped at
    /// the wire (§3.4) — see the module docs.
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
    /// `reqwest::Client`, so the pool's `(model, effort)` cross product shares
    /// one connection pool (the 028 T001 contract, mirrored).
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
            api_key: config.openai_api_key.clone(),
            model: model.to_string(),
            effort,
            structured: config.openai_structured_output,
            timeout_ms: config.request_timeout_ms,
            max_retries: config.max_retries,
            backoff_base_ms: 200,
            effort_notice: AtomicBool::new(false),
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
            &config.openai_model,
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

    /// The documented effort no-op (§3.4), announced once per client.
    ///
    /// The pool eagerly materialises every `(model, effort)` pair, so a
    /// per-construction notice would fire for clients that are never used.
    /// A per-call effort arrives through `for_site_with_effort`, so the first
    /// real use of a dropped level is still announced.
    fn notice_effort_no_op(&self) {
        let Some(effort) = self.effort else {
            return;
        };
        if !self.effort_notice.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                model = %self.model,
                effort = effort.as_str(),
                "reasoning effort is a documented no-op on the openai_compat \
                 backend: `effort` is dropped at the wire, not translated"
            );
        }
    }

    /// The Chat Completions body for one ladder rung.
    ///
    /// `mode.sanitized_schema` is the single source of truth for every rung
    /// (§3.2); rungs 2 and 4 carry it in the prompt because the endpoint gives
    /// them no schema field. No `effort` key and no output ceiling appear —
    /// see the module docs for both.
    fn build_body(&self, prompt: &str, schema: &Value, rung: Rung) -> Value {
        let content = if matches!(rung, Rung::JsonObject | Rung::PromptOnly) {
            format!("{prompt}\n\nRespond with a single JSON object matching this JSON Schema:\n{schema}")
        } else {
            prompt.to_string()
        };
        let mut body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": content }],
        });
        match rung {
            Rung::JsonSchema => {
                body["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": { "name": "response", "strict": true, "schema": schema },
                });
            }
            Rung::JsonObject => {
                body["response_format"] = json!({ "type": "json_object" });
            }
            Rung::ToolShim => {
                body["tools"] = json!([{
                    "type": "function",
                    "function": { "name": "emit_response", "strict": true, "parameters": schema },
                }]);
                body["tool_choice"] =
                    json!({ "type": "function", "function": { "name": "emit_response" } });
            }
            Rung::PromptOnly => {}
        }
        body
    }

    async fn send_once(&self, body: &Value) -> Result<reqwest::Response, AppError> {
        let mut request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .timeout(Duration::from_millis(self.timeout_ms))
            .json(body);
        // Keyless local endpoints (Ollama, LM Studio, vLLM) see no
        // `Authorization` header at all — a bare "Bearer " would be noise
        // some servers reject.
        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        request.send().await.map_err(|e| {
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
impl ModelClient for OpenAiCompatClient {
    async fn complete(&self, prompt: &str, schema: &Value) -> Result<Completion, AppError> {
        self.notice_effort_no_op();

        // The ladder (§3.2): one rung per request, degrading only on a
        // capability rejection. Retry policy mirrors the Anthropic client —
        // 429/5xx and transport errors retry with exponential backoff; a
        // timeout is terminal (it consumed the whole budget); other 4xx are
        // terminal unless they reject the strategy parameter itself.
        let mut last_capability: Option<String> = None;
        for rung in ladder_for(self.structured) {
            let body = self.build_body(prompt, schema, rung);
            let attempts_max = self.max_retries.saturating_add(1);
            let mut last_error = String::new();
            let mut capability: Option<String> = None;
            let mut terminal: Option<AppError> = None;

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
                    Err(timeout @ AppError::Timeout { .. }) => {
                        terminal = Some(timeout);
                        break;
                    }
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
                    // A rejection of the *strategy parameter* is a ladder
                    // signal, not a failure — the next rung is the remedy.
                    // Prompt-only sends nothing to reject, so its 400s are
                    // always terminal.
                    if rung != Rung::PromptOnly && is_capability_rejection(status, &detail) {
                        capability = Some(detail);
                        break;
                    }
                    terminal = Some(AppError::Client(format!("HTTP {status}: {detail}")));
                    break;
                }

                let payload: ChatResponse = match response.json().await {
                    Ok(payload) => payload,
                    // reqwest's .timeout() covers the body read too.
                    Err(e) if e.is_timeout() => {
                        terminal = Some(AppError::Timeout {
                            what: "request",
                            ms: self.timeout_ms,
                        });
                        break;
                    }
                    Err(e) => {
                        terminal = Some(AppError::Client(format!("response body unreadable: {e}")));
                        break;
                    }
                };
                return interpret(&payload, rung, &self.model);
            }

            if let Some(error) = terminal {
                return Err(error);
            }
            if let Some(detail) = capability {
                last_capability = Some(detail);
                continue;
            }
            return Err(AppError::RetriesExhausted {
                attempts: attempts_max,
                last: last_error,
            });
        }

        // Only reachable for a pinned rung: `auto` always ends on prompt-only,
        // which cannot be capability-rejected. Fail loud either way.
        Err(AppError::Client(format!(
            "out-of-contract provider response: `{}` rejected the {} structured-output \
             strategy as unsupported; last rejection: {}",
            self.model,
            self.structured.as_str(),
            last_capability.as_deref().unwrap_or("none")
        )))
    }
}

/// Whether a 4xx rejects the *strategy parameter* rather than the request.
///
/// Conservative on purpose, in two independent ways:
///
/// 1. **Status:** only `400 Bad Request` can be a strategy rejection.
///    Providers signal parameter-unsupported with 400 in practice; anything
///    else (401 auth, 403 forbidden, 404 model, 422 validation) is about the
///    request as a whole, not one parameter we could remove. This stops an
///    auth failure phrased as "unsupported auth scheme" from being read as a
///    ladder signal.
/// 2. **Body:** the body must both name one of the *parameters this adapter
///    actually sends* and contain a rejection phrase. Either alone is too
///    loose: "unknown tool: X" (a config mistake) contains no parameter
///    name, and "unsupported auth scheme" contains no parameter name either;
///    matching either single token degraded the ladder on failures that had
///    nothing to do with the strategy.
///
/// A rejection it cannot explain (wrong key, wrong model, malformed request)
/// stays terminal and loud instead of silently degrading the ladder.
/// Degrading a rung is safe; hiding a real failure behind one is not.
/// The parameters this adapter sends; a capability rejection must name one.
const PARAMETER_TOKENS: [&str; 5] = [
    "response_format",
    "json_schema",
    "json_object",
    "tool_choice",
    "tools",
];

/// Phrases providers use to say a parameter is rejected outright.
const REJECTION_TOKENS: [&str; 5] = [
    "not support",
    "unsupported",
    "unknown",
    "unrecognized",
    "unexpected",
];

fn is_capability_rejection(status: reqwest::StatusCode, detail: &str) -> bool {
    if status.as_u16() != 400 {
        return false;
    }
    let body = detail.to_lowercase();
    PARAMETER_TOKENS.iter().any(|t| body.contains(t))
        && REJECTION_TOKENS.iter().any(|t| body.contains(t))
}

/// Map a 2xx Chat Completions response to a [`Completion`] or its outcome
/// class (§3.3).
///
/// The taxonomy mirrors [`crate::client::anthropic`]'s `interpret`: a 200
/// whose signal the contract cannot use is a **billed** failure — the provider
/// ran the model — so every error here carries its usage.
fn interpret(payload: &ChatResponse, rung: Rung, model: &str) -> Result<Completion, AppError> {
    let (input_tokens, output_tokens) = payload.usage_or_warn(model);
    let billed = |error: AppError| error.metered(input_tokens, output_tokens);

    let choice = payload.choices.first().ok_or_else(|| {
        billed(AppError::Client(
            "out-of-contract provider response: no choices".to_string(),
        ))
    })?;

    // A refusal can arrive with any finish_reason; the field is the signal.
    if let Some(refusal) = choice.message.refusal.as_deref().filter(|r| !r.is_empty()) {
        return Err(billed(AppError::Refusal(refusal.to_string())));
    }

    match (rung, choice.finish_reason.as_deref()) {
        (_, Some("length")) => Err(billed(AppError::Truncation(format!(
            "output budget exhausted after {output_tokens} output tokens"
        )))),
        (_, Some("content_filter")) => Err(billed(AppError::Refusal(
            choice
                .message
                .content
                .as_deref()
                .unwrap_or("the provider declined to answer")
                .to_string(),
        ))),
        // The tool-shim rung's contract is a forced function call; some compat
        // servers still close it with `stop`, so both endings are accepted
        // when a tool call is actually present.
        (Rung::ToolShim, Some("tool_calls" | "stop")) => {
            let arguments = choice
                .message
                .tool_calls
                .first()
                .and_then(|call| call.function.as_ref())
                .and_then(|function| function.arguments.as_deref())
                .filter(|args| !args.trim().is_empty())
                .ok_or_else(|| {
                    billed(AppError::Client(
                        "out-of-contract provider response: empty tool_calls".to_string(),
                    ))
                })?;
            parse_constrained(arguments)
                .map_err(&billed)
                .map(|value| Completion {
                    value,
                    input_tokens,
                    output_tokens,
                })
        }
        (_, Some("stop")) => {
            let text = choice
                .message
                .content
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| {
                    billed(AppError::Client(
                        "out-of-contract provider response: no text block".to_string(),
                    ))
                })?;
            parse_constrained(text)
                .map_err(&billed)
                .map(|value| Completion {
                    value,
                    input_tokens,
                    output_tokens,
                })
        }
        // Anything else — including a `tool_calls` finish on a rung that never
        // asked for tools — is out of contract. When unsure, fail loud.
        (_, other) => Err(billed(AppError::Client(format!(
            "out-of-contract provider response: unexpected finish_reason: {other:?}"
        )))),
    }
}

/// Parse the constrained body. A provider that promised JSON and did not
/// deliver it is out of contract, not a schema violation the caller can act
/// on — the same classification the Anthropic client gives this case, whose
/// `outcome()` is `ValidationFailure`.
fn parse_constrained(text: &str) -> Result<Value, AppError> {
    serde_json::from_str(text).map_err(|e| {
        AppError::Client(format!(
            "out-of-contract provider response: constrained body failed to parse: {e}"
        ))
    })
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
    /// Absent on some compat providers — see [`Self::usage_or_warn`].
    usage: Option<Usage>,
}

impl ChatResponse {
    /// Usage mapping (§3.3): `prompt/completion_tokens` become the
    /// [`Completion`] counts. A provider that omits usage yields zeros **and**
    /// a warning per call — billing transparency is a parallax invariant, and
    /// every zeroed record is a mispriced one an operator needs to see.
    fn usage_or_warn(&self, model: &str) -> (u64, u64) {
        self.usage.as_ref().map_or_else(
            || {
                tracing::warn!(
                    model = %model,
                    "provider response carried no usage block; recording zero tokens \
                     (billing attribution for this call is incomplete)"
                );
                (0, 0)
            },
            |usage| (usage.prompt_tokens, usage.completion_tokens),
        )
    }
}

#[derive(Debug, Default, Deserialize)]
struct Choice {
    #[serde(default)]
    message: Message,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Message {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
    /// Present when the model declines (structured-outputs refusals).
    refusal: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolCall {
    function: Option<Function>,
}

#[derive(Debug, Deserialize)]
struct Function {
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

/// A [`Config`] with the `openai_compat` backend selected, for tests and the
/// parity harness. The endpoint is `127.0.0.1:1` so a test that escapes its
/// mock fails by connection refusal rather than reaching a live API
/// (the 028 lesson, mirrored).
#[cfg(test)]
#[must_use]
pub(crate) fn openai_test_config() -> Config {
    Config {
        backend: crate::config::Backend::OpenAiCompat,
        openai_api_key: "test-key".into(),
        openai_model: "test-model".into(),
        ..crate::config::test_config()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::error::Outcome;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn client_for(mock: &MockServer) -> OpenAiCompatClient {
        OpenAiCompatClient::with_base_url(&openai_test_config(), &mock.uri())
            .with_backoff_base_ms(1)
    }

    fn client_with(config: &Config, mock: &MockServer) -> OpenAiCompatClient {
        OpenAiCompatClient::with_base_url(config, &mock.uri()).with_backoff_base_ms(1)
    }

    fn stop_body(json_text: &str) -> serde_json::Value {
        json!({
            "choices": [{ "message": { "content": json_text }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
        })
    }

    async fn captured_body(mock: &MockServer, index: usize) -> serde_json::Value {
        let requests = mock.received_requests().await.unwrap();
        requests[index].body_json().unwrap()
    }

    #[tokio::test]
    async fn stop_parses_the_constrained_value_and_maps_usage() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(stop_body(r#"{"ok":true}"#)))
            .mount(&mock)
            .await;

        let out = client_for(&mock).complete("p", &json!({})).await.unwrap();
        assert_eq!(out.value, json!({ "ok": true }));
        assert_eq!((out.input_tokens, out.output_tokens), (100, 25));
    }

    /// §3.2 rung 1: the schema rides in `response_format.json_schema`, and
    /// nothing else may claim the schema's place.
    #[tokio::test]
    async fn the_json_schema_rung_sends_the_schema_in_response_format() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(stop_body("{}")))
            .mount(&mock)
            .await;
        let schema = json!({"type": "object", "additionalProperties": false});

        let config = Config {
            openai_structured_output: StructuredOutput::JsonSchema,
            ..openai_test_config()
        };
        client_with(&config, &mock)
            .complete("p", &schema)
            .await
            .unwrap();

        let body = captured_body(&mock, 0).await;
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert!(body.get("tools").is_none(), "{body}");
    }

    /// §3.2 rung 2: JSON-object mode plus the schema injected into the prompt.
    #[tokio::test]
    async fn the_json_object_rung_sends_the_schema_in_the_prompt() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(stop_body("{}")))
            .mount(&mock)
            .await;
        let schema = json!({"type": "object", "additionalProperties": false});

        let config = Config {
            openai_structured_output: StructuredOutput::JsonObject,
            ..openai_test_config()
        };
        client_with(&config, &mock)
            .complete("p", &schema)
            .await
            .unwrap();

        let body = captured_body(&mock, 0).await;
        assert_eq!(body["response_format"]["type"], "json_object");
        let content = body["messages"][0]["content"].as_str().unwrap();
        assert!(content.contains(&schema.to_string()), "{content}");
    }

    /// §3.2 rung 3: one forced function whose parameters are the schema; the
    /// response's `function.arguments` carries the JSON.
    #[tokio::test]
    async fn the_tool_shim_rung_forces_a_function_call_and_parses_its_arguments() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{ "function": { "arguments": "{\"ok\":true}" } }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            })))
            .mount(&mock)
            .await;
        let schema = json!({"type": "object", "additionalProperties": false});

        let config = Config {
            openai_structured_output: StructuredOutput::ToolShim,
            ..openai_test_config()
        };
        let out = client_with(&config, &mock)
            .complete("p", &schema)
            .await
            .unwrap();
        assert_eq!(out.value, json!({ "ok": true }));

        let body = captured_body(&mock, 0).await;
        assert_eq!(body["tools"][0]["function"]["parameters"], schema);
        assert_eq!(body["tool_choice"]["function"]["name"], "emit_response");
        assert!(body.get("response_format").is_none(), "{body}");
    }

    /// Keyless local endpoints (Ollama, LM Studio, vLLM) are first-class
    /// BYOM targets: the `Authorization` header exists iff a key is
    /// configured, and a keyless request carries none at all.
    #[tokio::test]
    async fn the_authorization_header_is_sent_only_when_a_key_is_configured() {
        let body = json!({
            "choices": [{ "message": { "content": "{}" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1 }
        });

        let keyless = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
            .mount(&keyless)
            .await;
        let config = Config {
            openai_api_key: String::new(),
            ..openai_test_config()
        };
        client_with(&config, &keyless)
            .complete("p", &json!({}))
            .await
            .unwrap();
        let requests = keyless.received_requests().await.unwrap();
        assert!(
            requests[0].headers.get("authorization").is_none(),
            "{:?}",
            requests[0].headers
        );

        let keyed = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&keyed)
            .await;
        let config = Config {
            openai_api_key: "test-key".into(),
            ..openai_test_config()
        };
        client_with(&config, &keyed)
            .complete("p", &json!({}))
            .await
            .unwrap();
        let requests = keyed.received_requests().await.unwrap();
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .map(|v| v.to_str().unwrap()),
            Some("Bearer test-key")
        );
    }

    /// The tool-shim rung's documented leniency (§3.3): some compat servers
    /// close even a forced tool call with `stop`. That is still a success —
    /// when a tool call is actually present.
    #[tokio::test]
    async fn the_tool_shim_rung_accepts_a_stop_finish_with_a_tool_call() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{ "function": { "arguments": "{\"ok\":true}" } }]
                    },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            })))
            .mount(&mock)
            .await;

        let config = Config {
            openai_structured_output: StructuredOutput::ToolShim,
            ..openai_test_config()
        };
        let out = client_with(&config, &mock)
            .complete("p", &json!({}))
            .await
            .unwrap();
        assert_eq!(out.value, json!({ "ok": true }));
    }

    /// The guard on that leniency: a `stop` finish with **no** tool call is
    /// out of contract — the rung's whole promise is a forced function call,
    /// and quietly accepting the content instead would be a silent fallback
    /// to a strategy nobody chose.
    #[tokio::test]
    async fn the_tool_shim_rung_rejects_a_stop_finish_without_a_tool_call() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": "{\"ok\":true}" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            })))
            .mount(&mock)
            .await;

        let config = Config {
            openai_structured_output: StructuredOutput::ToolShim,
            ..openai_test_config()
        };
        let err = client_with(&config, &mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err.root(), AppError::Client(_)), "{err:?}");
        assert!(err.to_string().contains("empty tool_calls"), "{err}");
        assert_eq!(err.billed(), (100, 25));
    }

    /// §3.2 rung 4: schema in the prompt only. This is the rung that must
    /// never silently accept garbage — see the parse-failure taxonomy test.
    #[tokio::test]
    async fn the_prompt_only_rung_sends_no_structured_output_parameter() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(stop_body("{}")))
            .mount(&mock)
            .await;
        let schema = json!({"type": "object", "additionalProperties": false});

        let config = Config {
            openai_structured_output: StructuredOutput::PromptOnly,
            ..openai_test_config()
        };
        client_with(&config, &mock)
            .complete("p", &schema)
            .await
            .unwrap();

        let body = captured_body(&mock, 0).await;
        assert!(body.get("response_format").is_none(), "{body}");
        assert!(body.get("tools").is_none(), "{body}");
        let content = body["messages"][0]["content"].as_str().unwrap();
        assert!(content.contains(&schema.to_string()), "{content}");
    }

    /// §3.2 fallback: `auto` degrades exactly one rung per capability
    /// rejection and succeeds when a lower rung is accepted.
    #[tokio::test]
    async fn auto_degrades_one_rung_on_a_capability_rejection() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                if body["response_format"]["type"] == "json_schema" {
                    ResponseTemplate::new(400).set_body_json(json!({
                        "error": { "message": "response_format json_schema is not supported" }
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(stop_body(r#"{"ok":true}"#))
                }
            })
            .mount(&mock)
            .await;

        let out = client_for(&mock)
            .complete("p", &json!({"type": "object"}))
            .await
            .unwrap();
        assert_eq!(out.value, json!({ "ok": true }));

        let requests = mock.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2, "degrade exactly one rung per rejection");
        let second: serde_json::Value = requests[1].body_json().unwrap();
        assert_eq!(second["response_format"]["type"], "json_object");
    }

    /// A pinned rung that the endpoint does not support fails loud, naming the
    /// model and the strategy — never a silent fall-through to a weaker rung
    /// the operator did not choose.
    #[tokio::test]
    async fn a_rejected_pinned_rung_fails_loud_naming_model_and_strategy() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": { "message": "response_format is not supported" }
            })))
            .mount(&mock)
            .await;

        let config = Config {
            openai_structured_output: StructuredOutput::JsonSchema,
            ..openai_test_config()
        };
        let err = client_with(&config, &mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("test-model"), "{message}");
        assert!(message.contains("json_schema"), "{message}");
    }

    /// §3.3: a 400 the ladder cannot explain is terminal. A wrong API key
    /// must never be answered by quietly weakening the request.
    #[tokio::test]
    async fn an_inexplicable_rejection_is_terminal_not_a_ladder_signal() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": { "message": "Incorrect API key provided" }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err.root(), AppError::Client(_)), "{err:?}");
        assert!(err.to_string().contains("Incorrect API key"), "{err}");
    }

    /// An auth failure phrased as "unsupported" must NOT be a ladder signal:
    /// no parameter name in the body, and a 401 is not 400. Before the
    /// tightening, the broad `unsupported` token degraded four rungs on a
    /// dead key and the final error named the strategy, not the auth.
    #[tokio::test]
    async fn an_unsupported_auth_scheme_is_terminal_not_a_ladder_signal() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": { "message": "unsupported auth scheme" }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({"type": "object"}))
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("unsupported auth scheme"), "{message}");
        assert!(!message.contains("structured-output strategy"), "{message}");
        let requests = mock.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "terminal: no ladder retries");
    }

    /// A bad tool name is a config mistake, not a strategy rejection: the
    /// body names `tools` but with no rejection phrase, and stays terminal.
    #[tokio::test]
    async fn an_unknown_tool_error_is_terminal_not_a_ladder_signal() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": { "message": "Error: unknown tool: web_search" }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({"type": "object"}))
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("unknown tool"), "{message}");
        assert!(!message.contains("structured-output strategy"), "{message}");
        let requests = mock.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "terminal: no ladder retries");
    }

    /// A parameter name WITHOUT a rejection phrase (plain validation error)
    /// is not a capability signal either — both tokens must co-occur.
    #[tokio::test]
    async fn a_parameter_error_without_a_rejection_phrase_is_terminal() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": { "message": "response_format must be an object" }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({"type": "object"}))
            .await
            .unwrap_err();
        assert!(matches!(err.root(), AppError::Client(_)), "{err:?}");
        let requests = mock.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "terminal: no ladder retries");
    }

    /// A loosely-phrased 400 that names the parameter and rejects it still
    /// degrades the ladder — the tightening must not regress real signals.
    #[tokio::test]
    async fn a_loosely_phrased_400_rejection_still_degrades_the_ladder() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                if body["response_format"]["type"] == "json_schema" {
                    ResponseTemplate::new(400).set_body_json(json!({
                        "error": { "message": "json_schema output is not supported by this model" }
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(stop_body(r#"{"ok":true}"#))
                }
            })
            .mount(&mock)
            .await;

        let out = client_for(&mock)
            .complete("p", &json!({"type": "object"}))
            .await
            .unwrap();
        assert_eq!(out.value, json!({ "ok": true }));
        let requests = mock.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2, "degraded exactly one rung");
    }

    /// §3.3 taxonomy, truncation: `finish_reason: "length"` is the
    /// OpenAI-compat spelling of `stop_reason: "max_tokens"` — a billed
    /// `Truncation`, never a silent success.
    #[tokio::test]
    async fn a_length_finish_is_a_billed_truncation() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": "{\"ok\":tru" }, "finish_reason": "length" }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err.root(), AppError::Truncation(_)), "{err:?}");
        assert!(matches!(err.outcome(), Outcome::Truncation), "{err:?}");
        assert_eq!(err.billed(), (100, 25));
    }

    /// §3.3 taxonomy, refusal: `content_filter` is the compat spelling.
    #[tokio::test]
    async fn a_content_filter_finish_is_a_billed_refusal() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": null }, "finish_reason": "content_filter" }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err.root(), AppError::Refusal(_)), "{err:?}");
        assert!(matches!(err.outcome(), Outcome::Refusal), "{err:?}");
        assert_eq!(err.billed(), (100, 25));
    }

    /// §3.3 taxonomy, refusal: a `refusal` field wins even on a `stop` finish.
    #[tokio::test]
    async fn a_refusal_field_is_a_refusal_even_on_a_stop_finish() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": null, "refusal": "I cannot help with that." },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err.root(), AppError::Refusal(_)), "{err:?}");
        assert!(err.to_string().contains("I cannot help"), "{err}");
        assert_eq!(err.billed(), (100, 25));
    }

    /// §3.3 taxonomy, out-of-contract: any other `finish_reason`, and an empty
    /// body on a good one. Default to `Client` — fail loud, never guess.
    #[tokio::test]
    async fn unexpected_or_empty_signals_are_out_of_contract_client_errors() {
        for body in [
            json!({
                "choices": [{ "message": { "content": "{}" }, "finish_reason": "tool_calls" }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            json!({
                "choices": [{ "message": { "content": null }, "finish_reason": "stop" }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            json!({
                "choices": [{ "message": { "content": "not json" }, "finish_reason": "stop" }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            json!({
                "choices": [{ "message": { "content": "{}" }, "finish_reason": "max_tokens" }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
        ] {
            let mock = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&mock)
                .await;

            let err = client_for(&mock)
                .complete("p", &json!({}))
                .await
                .unwrap_err();
            assert!(matches!(err.root(), AppError::Client(_)), "{err:?}");
            assert!(
                matches!(err.outcome(), Outcome::ValidationFailure),
                "{err:?}"
            );
            assert_eq!(err.billed(), (100, 25), "{err}");
        }
    }

    /// §3.3 usage policy: an omitted usage block records zeros and warns —
    /// the mispriced record is loud, never silent.
    #[tokio::test]
    async fn an_omitted_usage_block_records_zeros_and_warns() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": "{\"ok\":true}" }, "finish_reason": "stop" }]
            })))
            .mount(&mock)
            .await;

        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let out = client_for(&mock).complete("p", &json!({})).await.unwrap();
        assert_eq!((out.input_tokens, out.output_tokens), (0, 0));

        let logged = String::from_utf8_lossy(&buffer.contents()).to_string();
        assert!(logged.contains("no usage block"), "{logged}");
    }

    /// §3.3 usage policy: zeros are zeros — `metered` never wraps a failure
    /// that cost nothing, so the error stays exactly the error it was.
    #[tokio::test]
    async fn zero_usage_never_wraps_the_error_in_metered() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": "{\"ok\":tru" }, "finish_reason": "length" }]
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Truncation(_)), "{err:?}");
        assert_eq!(err.billed(), (0, 0));
    }

    /// §3.4: `effort` is a documented no-op on this adapter — dropped at the
    /// wire, not translated. The word must not appear anywhere in the body.
    #[tokio::test]
    async fn a_routed_effort_is_dropped_at_the_wire() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let serialized = serde_json::to_string(&body).unwrap();
                assert!(
                    !serialized.contains("effort"),
                    "effort is a no-op on this adapter and must not appear: {serialized}"
                );
                assert!(
                    !serialized.contains("reasoning_effort"),
                    "no mistranslation either: {serialized}"
                );
                ResponseTemplate::new(200).set_body_json(stop_body("{}"))
            })
            .mount(&mock)
            .await;

        let client = OpenAiCompatClient {
            effort: Some(crate::routing::Effort::Low),
            ..client_for(&mock)
        };
        client.complete("p", &json!({})).await.unwrap();
    }

    /// Retry policy parity with the Anthropic client: 429 retries, and the
    /// recovered call still returns the parsed value.
    #[tokio::test]
    async fn a_429_retries_then_succeeds() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(stop_body(r#"{"ok":true}"#)))
            .mount(&mock)
            .await;

        let out = client_for(&mock).complete("p", &json!({})).await.unwrap();
        assert_eq!(out.value, json!({ "ok": true }));
    }

    /// Retry policy parity: a timeout is terminal — it already consumed the
    /// full per-request budget — and classifies as `Timeout`.
    #[tokio::test]
    async fn a_timeout_is_terminal_and_classified_as_timeout() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(stop_body("{}"))
                    .set_delay(Duration::from_millis(400)),
            )
            .mount(&mock)
            .await;

        let config = Config {
            request_timeout_ms: 50,
            ..openai_test_config()
        };
        let err = client_with(&config, &mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err.root(), AppError::Timeout { .. }), "{err:?}");
        assert!(matches!(err.outcome(), Outcome::Timeout), "{err:?}");
    }

    /// Retry policy parity: a persistent 5xx retries and then reports
    /// `RetriesExhausted`, naming the last error — never a success.
    #[tokio::test]
    async fn persistent_5xx_exhausts_retries_loudly() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock)
            .await;
        let err = client_for(&mock)
            .complete("p", &json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err.root(), AppError::RetriesExhausted { .. }),
            "{err:?}"
        );
        assert!(err.to_string().contains("HTTP 500"), "{err}");
    }

    /// Live smoke — opt-in, never part of the offline gate:
    /// `cargo test -- --ignored live_smoke` with `OPENAI_MODEL` set (plus
    /// `OPENAI_API_KEY` and `OPENAI_API_BASE` when the endpoint authenticates
    /// or is not the default). One representative schema per
    /// tool group (verify, check, research, memory) is all it takes to prove
    /// the catalog's shapes round-trip on a real endpoint.
    #[tokio::test]
    #[ignore = "live smoke: hits a real endpoint; run with --ignored"]
    async fn live_smoke_openai_compat_returns_schema_shaped_json() {
        let config = Config {
            openai_api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            openai_model: std::env::var("OPENAI_MODEL").expect("OPENAI_MODEL"),
            openai_api_base: std::env::var("OPENAI_API_BASE")
                .unwrap_or_else(|_| crate::config::DEFAULT_OPENAI_API_BASE.to_string()),
            ..openai_test_config()
        };
        let client = OpenAiCompatClient::new(&config);

        // Shaped like the schemas the four tool groups actually send
        // (`mode.sanitized_schema` is flat + closed everywhere).
        let group_schemas = [
            (
                "verify",
                json!({"type": "object", "properties": {"verdict": {"type": "string", "enum": ["supported", "refuted", "unclear"]}}, "required": ["verdict"], "additionalProperties": false}),
            ),
            (
                "check",
                json!({"type": "object", "properties": {"result": {"type": "string", "enum": ["violation", "clean"]}}, "required": ["result"], "additionalProperties": false}),
            ),
            (
                "research",
                json!({"type": "object", "properties": {"claim": {"type": "string"}}, "required": ["claim"], "additionalProperties": false}),
            ),
            (
                "memory",
                json!({"type": "object", "properties": {"relation": {"type": "string", "enum": ["duplicate", "contradiction", "refinement", "unrelated"]}}, "required": ["relation"], "additionalProperties": false}),
            ),
        ];

        for (group, schema) in group_schemas {
            let out = client
                .complete("Answer with JSON only. Which verdict applies?", &schema)
                .await
                .unwrap_or_else(|e| panic!("{group}: {e}"));
            assert!(out.value.is_object(), "{group}: {}", out.value);
        }
    }

    /// A minimal `io::Write` sink so a test can read what the subscriber was
    /// told. Thread-local `set_default` keeps this isolated per test.
    #[derive(Clone, Default)]
    struct Buffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Buffer {
        fn contents(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    impl std::io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }
}
