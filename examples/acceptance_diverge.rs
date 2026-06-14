//! Acceptance for 012-diverge-perspectives (offline mechanism shape).
//!
//! Drives the real server (Anthropic mocked by wiremock): distinct per-lens
//! framings come back lens-labeled with no verdict, and identical framings
//! deduplicate. The live SC-001/SC-003/SC-004 properties (real divergence; stance
//! does not narrow; no over-divergence) are the dogfood — a mock cannot diverge.
//!
//! Run: `cargo run --example acceptance_diverge`

#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::AnthropicClient;
use mcp_parallax::config::Config;
use mcp_parallax::server::Parallax;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::clock::SystemClock;
use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, method, path};
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

fn pass(framing: &str, implication: &str) -> Value {
    json!({ "framing": framing, "implication": implication })
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

fn dv(problem: &str) -> CallToolRequestParams {
    let mut p = CallToolRequestParams::new("diverge");
    p.arguments = json!({ "problem": problem }).as_object().cloned();
    p
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Distinct per-lens framings → 3 lens-labeled perspectives, no verdict.
    {
        let mock = MockServer::start().await;
        for (needle, framing) in [
            (
                "Flip the goal",
                "What if more steps is the fix, each earning trust?",
            ),
            (
                "Change whose problem",
                "Is this the user's problem or the team's metric?",
            ),
            (
                "Shift the time scale",
                "At a one-year horizon, is length even the lever?",
            ),
        ] {
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .and(body_string_contains(needle))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(end_turn(&pass(framing, "it changes the frame"))),
                )
                .mount(&mock)
                .await;
        }
        let (client, storage, _srv) = serve(&mock).await;
        let result = client
            .call_tool(dv("We need to cut steps from onboarding."))
            .await
            .unwrap();
        let s = result.structured_content.as_ref().unwrap();
        let ps = s["perspectives"].as_array().unwrap();
        assert_eq!(ps.len(), 3);
        let lenses: Vec<&str> = ps.iter().map(|p| p["lens"].as_str().unwrap()).collect();
        assert_eq!(lenses, vec!["invert", "actor", "horizon"]);
        assert!(s.get("verdict").is_none());
        assert_eq!(storage.list_invocations().await.unwrap().len(), 1);
        client.cancel().await.unwrap();
        println!("FR-002/003/007 distinct lens-labeled framings, no verdict: PASS");
    }

    // Identical framings across passes → deduplicated to one (earliest lens kept).
    {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&pass(
                "This is really a naming problem, not a flow problem.",
                "Renaming may resolve the felt friction.",
            ))))
            .mount(&mock)
            .await;
        let (client, _s, _srv) = serve(&mock).await;
        let result = client.call_tool(dv("Cut onboarding steps.")).await.unwrap();
        let s = result.structured_content.as_ref().unwrap();
        assert_eq!(s["perspectives"].as_array().unwrap().len(), 1);
        assert_eq!(s["perspectives"][0]["lens"], "invert");
        assert_eq!(s["passes"], 3);
        client.cancel().await.unwrap();
        println!("FR-004 deterministic dedup (3 identical -> 1): PASS");
    }

    println!("\nacceptance_diverge: ALL CHECKS PASS");
}
