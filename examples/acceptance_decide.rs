//! Acceptance for 013-decide-methodology (offline calibration shape).
//!
//! Drives the real server (Anthropic mocked by wiremock): a scored single pass
//! yields a server-derived recommendation with margin-calibrated confidence, and
//! a near-tie reads lower than a dominant winner. The live SC-004 property (the
//! model picks the *fitting* methodology) is the dogfood — a mock can't judge fit;
//! the calibration math here is fully offline (server math over the scores).
//!
//! Run: `cargo run --example acceptance_decide`

#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(clippy::needless_pass_by_value)]

use mcp_parallax::client::AnthropicClient;
use mcp_parallax::config::Config;
use mcp_parallax::server::Parallax;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::clock::SystemClock;
use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config() -> Config {
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
        fetch_allow_private: false,
        checkpoint_gate_patterns: vec![],
        grounded_verify_root: None,
        grounded_verify_max_bytes: 262_144,
        grounded_verify_max_locators: 64,
        database_path: ":memory:".into(),
        log_level: "info".into(),
        request_timeout_ms: 5_000,
        max_retries: 1,
    }
}

fn end_turn(value: &Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 100, "output_tokens": 10 }
    })
}

fn assessment(methodology: &str, scores: Value, rationales: Value, factors: Value) -> Value {
    json!({
        "methodology": methodology,
        "option_scores": scores,
        "option_rationales": rationales,
        "deciding_factors": factors,
    })
}

async fn serve(
    mock: &MockServer,
) -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    Arc<SqliteStorage>,
    rmcp::service::RunningService<rmcp::service::RoleServer, Parallax>,
) {
    let cfg = config();
    let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
    let anthropic =
        Arc::new(AnthropicClient::with_base_url(&cfg, &mock.uri()).with_backoff_base_ms(1));
    let server = Parallax::new(anthropic, storage.clone(), Arc::new(SystemClock), &cfg).unwrap();
    let (sio, cio) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(async move { server.serve(sio).await });
    let client = ().serve(cio).await.unwrap();
    let running = task.await.unwrap().unwrap();
    (client, storage, running)
}

fn dv(decision: &str, options: Value) -> CallToolRequestParams {
    let mut p = CallToolRequestParams::new("decide");
    p.arguments = json!({ "decision": decision, "options": options })
        .as_object()
        .cloned();
    p
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Dominant winner → recommended top, high confidence (margin 45 → 0.725).
    {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(end_turn(&assessment(
                    "weigh",
                    json!([85, 40]),
                    json!(["safe and reversible", "fast but risky"]),
                    json!(["blast radius", "rollback speed"]),
                ))),
            )
            .mount(&mock)
            .await;
        let (client, storage, _srv) = serve(&mock).await;
        let r = client
            .call_tool(dv("ship?", json!(["ramp", "big-bang"])))
            .await
            .unwrap();
        let s = r.structured_content.as_ref().unwrap();
        assert_eq!(s["recommended"], "ramp");
        assert_eq!(s["methodology"], "weigh");
        assert!((s["confidence"].as_f64().unwrap() - 0.725).abs() < 1e-9);
        assert!(s.get("verdict").is_none());
        assert_eq!(storage.list_invocations().await.unwrap().len(), 1);
        client.cancel().await.unwrap();
        println!("FR-002/004/005 dominant winner → high confidence (0.725), no verdict: PASS");
    }

    // Near-tie → lower confidence (margin 5 → 0.525), tracking closeness.
    {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(end_turn(&assessment(
                    "causal",
                    json!([60, 55]),
                    json!(["a", "b"]),
                    json!(["downstream effect"]),
                ))),
            )
            .mount(&mock)
            .await;
        let (client, _s, _srv) = serve(&mock).await;
        let r = client.call_tool(dv("d", json!(["x", "y"]))).await.unwrap();
        let s = r.structured_content.as_ref().unwrap();
        assert!((s["confidence"].as_f64().unwrap() - 0.525).abs() < 1e-9);
        client.cancel().await.unwrap();
        println!("SC-002 near-tie → lower confidence (0.525): PASS");
    }

    println!("\nacceptance_decide: ALL CHECKS PASS");
}
