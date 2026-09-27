//! Per-call-site wire coverage (BYOM §4–§5): every physical call site's **real**
//! output schema through the `openai_compat` adapter.
//!
//! The adapter is schema-agnostic by construction — it receives
//! `mode.sanitized_schema` and must place it on the wire intact — so what these
//! tests protect is the round trip: the exact schema each of the 12 routed call
//! sites (13 operations counting memory consolidation, which borrows Verify's
//! client and must inherit the backend with no special casing) hands to
//! `complete()` must arrive byte-equal in the selected structured-output slot,
//! and a canned response must come back as a parsed [`Completion`] with the
//! provider's usage mapped onto it.
//!
//! Schema *validity* of each mode's canned answers is that mode's own test
//! territory (`MockModelClient`, which validates); here the contract under test
//! is the wire.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::client::openai_compat::openai_test_config;
use crate::client::OpenAiCompatClient;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::routing::CallSite;
use crate::telemetry::ModelUsage;
use crate::traits::client::ModelClient;
use serde_json::{json, Value};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn stop_body(json_text: &str) -> Value {
    json!({
        "choices": [{ "message": { "content": json_text }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 25 }
    })
}

/// The full mode registry, built exactly as `server.rs` builds it at boot.
fn full_registry() -> ModeRegistry {
    let mut registry = ModeRegistry::new();
    crate::modes::verify::register(&mut registry, 3).unwrap();
    crate::modes::unstick::register(&mut registry).unwrap();
    crate::modes::diverge::register(&mut registry, 3).unwrap();
    crate::modes::decide::register(&mut registry).unwrap();
    crate::modes::elicit::register(&mut registry).unwrap();
    crate::modes::grounded_verify::register(&mut registry, 3).unwrap();
    crate::deterministic::translate::register(&mut registry).unwrap();
    crate::research::prompts::register(&mut registry).unwrap();
    crate::memory::consolidate::register(&mut registry).unwrap();
    crate::checkpoint::review::register(&mut registry).unwrap();
    registry
}

/// Every LLM-backed operation as `(label, mode)`: the 12 routed call sites in
/// `CallSite::ALL` order, then memory consolidation (the unrouted 13th — it
/// borrows the Verify client at `server.rs`, so it inherits whatever backend
/// Verify resolved to).
fn call_site_ops() -> Vec<(String, CorrectiveMode)> {
    let registry = full_registry();
    let verify = registry
        .get(crate::modes::verify::VERIFY_ID)
        .unwrap()
        .clone();
    let research_verify = crate::research::prompts::research_verify_mode(&verify);

    let mut ops: Vec<(String, CorrectiveMode)> = CallSite::ALL
        .iter()
        .map(|site| {
            let mode = match site {
                CallSite::Verify => verify.clone(),
                CallSite::Unstick => registry
                    .get(crate::modes::unstick::UNSTICK_ID)
                    .unwrap()
                    .clone(),
                CallSite::Diverge => registry
                    .get(crate::modes::diverge::DIVERGE_ID)
                    .unwrap()
                    .clone(),
                CallSite::Decide => registry
                    .get(crate::modes::decide::DECIDE_ID)
                    .unwrap()
                    .clone(),
                CallSite::Elicit => registry
                    .get(crate::modes::elicit::ELICIT_ID)
                    .unwrap()
                    .clone(),
                CallSite::GroundedVerify => registry
                    .get(crate::modes::grounded_verify::GROUNDED_VERIFY_ID)
                    .unwrap()
                    .clone(),
                CallSite::CheckTranslate => registry
                    .get(crate::deterministic::translate::TRANSLATE_MODE_ID)
                    .unwrap()
                    .clone(),
                CallSite::ResearchScope => registry
                    .get(crate::research::prompts::SCOPE_MODE_ID)
                    .unwrap()
                    .clone(),
                CallSite::ResearchExtract => registry
                    .get(crate::research::prompts::EXTRACT_MODE_ID)
                    .unwrap()
                    .clone(),
                CallSite::ResearchVerify => research_verify.clone(),
                CallSite::ResearchSynthesize => registry
                    .get(crate::research::prompts::SYNTH_MODE_ID)
                    .unwrap()
                    .clone(),
                CallSite::CheckpointReview => registry
                    .get(crate::checkpoint::review::REVIEW_MODE_ID)
                    .unwrap()
                    .clone(),
            };
            (format!("CallSite::{site:?}"), mode)
        })
        .collect();
    ops.push((
        "memory consolidation (unrouted, borrows Verify's client)".to_string(),
        registry
            .get(crate::memory::consolidate::CONSOLIDATION_MODE_ID)
            .unwrap()
            .clone(),
    ));
    ops
}

/// The list itself is the coverage claim: 12 sites in `CallSite::ALL` order
/// plus the unrouted consolidation judge. If a 13th site is ever routed, this
/// fails until its mode joins the list.
#[test]
fn the_op_list_covers_every_site_plus_consolidation() {
    let ops = call_site_ops();
    assert_eq!(ops.len(), CallSite::ALL.len() + 1);
    for (index, site) in CallSite::ALL.iter().enumerate() {
        assert_eq!(ops[index].0, format!("CallSite::{site:?}"));
    }
}

/// §5.1 request-shape, every op: the site's real sanitized schema arrives in
/// the structured-output slot byte-equal, and §4.4 — `effort`, routed or not,
/// never reaches this adapter's wire.
#[tokio::test]
async fn every_call_site_sends_its_schema_and_never_effort() {
    for (site, mode) in call_site_ops() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(stop_body("{}")))
            .mount(&mock)
            .await;

        let plain = OpenAiCompatClient::with_base_url(&openai_test_config(), &mock.uri())
            .with_backoff_base_ms(1);
        plain
            .complete("p", &mode.sanitized_schema)
            .await
            .unwrap_or_else(|e| panic!("{site}: {e}"));

        // The same call with the site routed to a reasoning effort — the level
        // is a documented no-op here and must be dropped, never mistranslated.
        let routed = OpenAiCompatClient::with_http_client(
            &openai_test_config(),
            &reqwest::Client::new(),
            &mock.uri(),
            "test-model",
            Some(crate::routing::Effort::High),
        )
        .with_backoff_base_ms(1);
        routed
            .complete("p", &mode.sanitized_schema)
            .await
            .unwrap_or_else(|e| panic!("{site}: {e}"));

        let requests = mock.received_requests().await.unwrap();
        for request in &requests {
            let body: Value = request.body_json().unwrap();
            assert_eq!(
                body["response_format"]["json_schema"]["schema"], mode.sanitized_schema,
                "{site}: the schema must round-trip intact"
            );
            let serialized = serde_json::to_string(&body).unwrap();
            assert!(
                !serialized.contains("effort"),
                "{site}: effort is dropped at the wire on this adapter: {serialized}"
            );
        }
        assert_eq!(requests.len(), 2, "{site}");
    }
}

/// §5.2 happy path, every op: a canned response becomes a `Completion` with
/// the parsed value and the provider's usage mapped onto it.
#[tokio::test]
async fn every_call_site_parses_its_response_and_usage() {
    for (site, mode) in call_site_ops() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(stop_body(r#"{"ok":true}"#)))
            .mount(&mock)
            .await;

        let client = OpenAiCompatClient::with_base_url(&openai_test_config(), &mock.uri())
            .with_backoff_base_ms(1);
        let out = client
            .complete("p", &mode.sanitized_schema)
            .await
            .unwrap_or_else(|e| panic!("{site}: {e}"));
        assert_eq!(out.value, json!({ "ok": true }), "{site}");
        assert_eq!(
            (out.input_tokens, out.output_tokens),
            (100, 25),
            "{site}: usage must map onto Completion unchanged"
        );
    }
}

/// §4.5 token metering: every call site feeds `meter.add(model_for(site), in,
/// out)` with the `Completion` counts verbatim (`pipeline.rs`), so what the
/// adapter maps is what the meter records. An unpriced model id must stay
/// visibly unpriced — cost figures on the openai_compat backend are exactly
/// where a silent mispricing would hide.
#[tokio::test]
async fn completion_counts_arrive_at_the_meter_unchanged() {
    let (site, mode) = call_site_ops().into_iter().next().unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(stop_body("{}")))
        .mount(&mock)
        .await;

    let client = OpenAiCompatClient::with_base_url(&openai_test_config(), &mock.uri())
        .with_backoff_base_ms(1);
    let out = client
        .complete("p", &mode.sanitized_schema)
        .await
        .unwrap_or_else(|e| panic!("{site}: {e}"));

    let mut meter = ModelUsage::single("test-model", out.input_tokens, out.output_tokens);
    meter.add("test-model", out.input_tokens, out.output_tokens);
    assert_eq!(meter.totals(), (200, 50), "{site}");
    assert!(
        !crate::telemetry::pricing_known("test-model"),
        "an unpriced backend model must not look priced"
    );
}
