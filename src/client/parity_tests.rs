//! Taxonomy-parity harness (BYOM design §3.3 / §5.3) — the acceptance gate
//! for "provider-agnostic".
//!
//! Each row states one provider outcome as each backend spells it on the wire,
//! and asserts both adapters produce the **identical** `AppError` variant,
//! outcome class, and billed usage. The Anthropic mapping is the reference
//! behavior: a row passing means a deployment can switch `PARALLAX_BACKEND`
//! without the tool layer learning a second error dialect — truncations stay
//! truncations (billed), refusals stay refusals, and everything unexplained
//! stays a loud out-of-contract `Client` error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::client::{AnthropicClient, OpenAiCompatClient};
use crate::config::{test_config, Config};
use crate::error::AppError;
use crate::traits::client::ModelClient;
use serde_json::{json, Value};
use std::time::Duration;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// One taxonomy row: the same outcome, spelled the way each provider spells
/// it, plus what both adapters must make of it.
struct Row {
    name: &'static str,
    anthropic_body: Value,
    openai_body: Value,
    /// The `AppError` variant both must produce, by discriminant name.
    expected: &'static str,
    /// Billed usage both must attach, `(input, output)`.
    billed: (u64, u64),
    /// The value both must return on the success rows.
    value: Option<Value>,
}

/// The §3.3 table as data, plus the parse-failure rows §5.4 names.
#[allow(clippy::too_many_lines)] // one taxonomy table; splitting it scatters the contract
fn rows() -> Vec<Row> {
    vec![
        Row {
            name: "success: end_turn / stop",
            anthropic_body: json!({
                "content": [{ "type": "text", "text": "{\"verdict\":\"ok\"}" }],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 100, "output_tokens": 25 }
            }),
            openai_body: json!({
                "choices": [{
                    "message": { "content": "{\"verdict\":\"ok\"}" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            expected: "success",
            billed: (100, 25),
            value: Some(json!({ "verdict": "ok" })),
        },
        Row {
            name: "truncation: max_tokens / length",
            anthropic_body: json!({
                "content": [{ "type": "text", "text": "{\"verdict\":\"trunc" }],
                "stop_reason": "max_tokens",
                "usage": { "input_tokens": 100, "output_tokens": 25 }
            }),
            openai_body: json!({
                "choices": [{
                    "message": { "content": "{\"verdict\":\"trunc" },
                    "finish_reason": "length"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            expected: "Truncation",
            billed: (100, 25),
            value: None,
        },
        Row {
            name: "refusal: refusal stop / content_filter",
            anthropic_body: json!({
                "content": [{ "type": "text", "text": "declined" }],
                "stop_reason": "refusal",
                "usage": { "input_tokens": 100, "output_tokens": 25 }
            }),
            openai_body: json!({
                "choices": [{
                    "message": { "content": null },
                    "finish_reason": "content_filter"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            expected: "Refusal",
            billed: (100, 25),
            value: None,
        },
        Row {
            name: "refusal: via the refusal field on a stop finish",
            anthropic_body: json!({
                "content": [{ "type": "text", "text": "declined" }],
                "stop_reason": "refusal",
                "usage": { "input_tokens": 100, "output_tokens": 25 }
            }),
            openai_body: json!({
                "choices": [{
                    "message": { "content": null, "refusal": "declined" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            expected: "Refusal",
            billed: (100, 25),
            value: None,
        },
        Row {
            name: "out-of-contract: foreign stop/finish reason",
            anthropic_body: json!({
                "content": [{ "type": "text", "text": "{}" }],
                "stop_reason": "tool_use",
                "usage": { "input_tokens": 100, "output_tokens": 25 }
            }),
            openai_body: json!({
                "choices": [{
                    "message": { "content": "{}" },
                    "finish_reason": "unexpected_signal"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            expected: "Client",
            billed: (100, 25),
            value: None,
        },
        Row {
            name: "out-of-contract: empty content on a good finish",
            anthropic_body: json!({
                "content": [],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 100, "output_tokens": 25 }
            }),
            openai_body: json!({
                "choices": [{
                    "message": { "content": null },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            expected: "Client",
            billed: (100, 25),
            value: None,
        },
        Row {
            name: "out-of-contract: constrained body fails to parse (§5.4)",
            anthropic_body: json!({
                "content": [{ "type": "text", "text": "not json" }],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 100, "output_tokens": 25 }
            }),
            openai_body: json!({
                "choices": [{
                    "message": { "content": "not json" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
            }),
            expected: "Client",
            billed: (100, 25),
            value: None,
        },
    ]
}

/// Discriminant name of the underlying failure — metering is orthogonal to
/// class, so parity is judged on [`AppError::root`].
fn variant_name(error: &AppError) -> &'static str {
    match error {
        AppError::Config(_) => "Config",
        AppError::Storage(_) => "Storage",
        AppError::Client(_) => "Client",
        AppError::Refusal(_) => "Refusal",
        AppError::Truncation(_) => "Truncation",
        AppError::Timeout { .. } => "Timeout",
        AppError::RetriesExhausted { .. } => "RetriesExhausted",
        AppError::InvalidInput(_) => "InvalidInput",
        AppError::ValidationFailure(_) => "ValidationFailure",
        AppError::Cancelled => "Cancelled",
        AppError::EmbeddingProvider(_) => "EmbeddingProvider",
        AppError::SearchProvider(_) => "SearchProvider",
        AppError::Metered { source, .. } => variant_name(source),
    }
}

async fn anthropic_answer(body: Value) -> Result<crate::traits::client::Completion, AppError> {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&mock)
        .await;
    AnthropicClient::with_base_url(&test_config(), &mock.uri())
        .with_backoff_base_ms(1)
        .complete("p", &json!({ "type": "object" }))
        .await
}

async fn openai_answer(body: Value) -> Result<crate::traits::client::Completion, AppError> {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&mock)
        .await;
    OpenAiCompatClient::with_base_url(&openai_test_config(), &mock.uri())
        .with_backoff_base_ms(1)
        .complete("p", &json!({ "type": "object" }))
        .await
}

use crate::client::openai_compat::openai_test_config;

/// Every §3.3 row, both backends: identical variant, identical outcome class,
/// identical billed usage.
#[tokio::test]
async fn every_taxonomy_row_maps_identically_on_both_backends() {
    for row in rows() {
        let anthropic = anthropic_answer(row.anthropic_body.clone()).await;
        let openai = openai_answer(row.openai_body.clone()).await;

        match (&row.value, &anthropic, &openai) {
            (Some(expected), Ok(a), Ok(o)) => {
                assert_eq!(a.value, *expected, "[{}]", row.name);
                assert_eq!(o.value, *expected, "[{}]", row.name);
                assert_eq!((a.input_tokens, a.output_tokens), row.billed, "[{}]", row.name);
                assert_eq!((o.input_tokens, o.output_tokens), row.billed, "[{}]", row.name);
            }
            (None, Err(a), Err(o)) => {
                assert_eq!(
                    variant_name(a.root()),
                    row.expected,
                    "[{}] anthropic: {a:?}",
                    row.name
                );
                assert_eq!(
                    variant_name(o.root()),
                    row.expected,
                    "[{}] openai_compat: {o:?}",
                    row.name
                );
                // The parity gate itself: both dialects, one class.
                assert_eq!(variant_name(a.root()), variant_name(o.root()), "[{}]", row.name);
                assert_eq!(a.outcome(), o.outcome(), "[{}]", row.name);
                assert_eq!(a.billed(), row.billed, "[{}] anthropic: {a:?}", row.name);
                assert_eq!(o.billed(), row.billed, "[{}] openai_compat: {o:?}", row.name);
            }
            (value, a, o) => panic!(
                "[{}] shape mismatch: expected value {value:?}, got anthropic {a:?} / openai_compat {o:?}",
                row.name
            ),
        }
    }
}

/// Billed usage attaches to failures identically: the tokens were not free,
/// and the adapters must not disagree about it.
#[tokio::test]
async fn billed_usage_agrees_on_a_truncation_row() {
    let truncation = rows()
        .into_iter()
        .find(|row| row.expected == "Truncation")
        .unwrap();
    let (Err(anthropic), Err(openai)) = (
        anthropic_answer(truncation.anthropic_body).await,
        openai_answer(truncation.openai_body).await,
    ) else {
        panic!("truncation rows must fail");
    };
    assert_eq!(anthropic.billed(), openai.billed());
    assert_eq!(anthropic.billed(), (100, 25));
}

/// One canned provider response through each adapter, at an explicit timeout
/// and retry budget — the client-side taxonomy rows need to steer both.
async fn anthropic_with(
    template: ResponseTemplate,
    timeout_ms: u64,
    retries: u32,
) -> Result<crate::traits::client::Completion, AppError> {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(template)
        .mount(&mock)
        .await;
    let config = Config {
        request_timeout_ms: timeout_ms,
        max_retries: retries,
        ..test_config()
    };
    AnthropicClient::with_base_url(&config, &mock.uri())
        .with_backoff_base_ms(1)
        .complete("p", &json!({ "type": "object" }))
        .await
}

async fn openai_with(
    template: ResponseTemplate,
    timeout_ms: u64,
    retries: u32,
) -> Result<crate::traits::client::Completion, AppError> {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(template)
        .mount(&mock)
        .await;
    let config = Config {
        request_timeout_ms: timeout_ms,
        max_retries: retries,
        ..openai_test_config()
    };
    OpenAiCompatClient::with_base_url(&config, &mock.uri())
        .with_backoff_base_ms(1)
        .complete("p", &json!({ "type": "object" }))
        .await
}

/// §3.3's `Timeout` row: a provider slower than the request budget is a
/// terminal `Timeout` on both backends — it consumed the whole budget, so
/// retrying it would be the wrong answer twice.
#[tokio::test]
async fn timeout_is_a_timeout_on_both_backends() {
    let slow = ResponseTemplate::new(200)
        .set_body_json(json!({
            "content": [{ "type": "text", "text": "{}" }],
            "stop_reason": "end_turn"
        }))
        .set_delay(Duration::from_millis(400));

    let anthropic = anthropic_with(slow.clone(), 50, 2).await.unwrap_err();
    let openai = openai_with(slow, 50, 2).await.unwrap_err();

    assert_eq!(variant_name(anthropic.root()), "Timeout", "{anthropic:?}");
    assert_eq!(variant_name(anthropic.root()), variant_name(openai.root()));
    assert_eq!(anthropic.outcome(), openai.outcome());
}

/// §3.3's `RetriesExhausted` row: retry-policy exhaustion classifies the same
/// on both backends, and never as a success.
#[tokio::test]
async fn retry_exhaustion_is_retries_exhausted_on_both_backends() {
    let broken = ResponseTemplate::new(500);

    let anthropic = anthropic_with(broken.clone(), 2_000, 2).await.unwrap_err();
    let openai = openai_with(broken, 2_000, 2).await.unwrap_err();

    assert_eq!(
        variant_name(anthropic.root()),
        "RetriesExhausted",
        "{anthropic:?}"
    );
    assert_eq!(variant_name(anthropic.root()), variant_name(openai.root()));
    assert_eq!(anthropic.outcome(), openai.outcome());
}

/// The config seam each harness row relies on — if the two test configs ever
/// diverge (endpoint, retries), the rows above compare noise.
#[test]
fn both_test_configs_share_the_test_endpoint() {
    let anthropic: Config = test_config();
    let openai: Config = openai_test_config();
    assert_eq!(anthropic.anthropic_api_base, openai.openai_api_base);
    assert_eq!(anthropic.max_retries, openai.max_retries);
}
