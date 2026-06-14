//! Acceptance for 014-preference-elicitation (offline shape).
//!
//! Drives the real server (Anthropic mocked by wiremock): elicit surfaces the
//! assumed objective + traced preferences, reports `memory_consulted`, carries no
//! enforcement field, and on a low-signal inference fabricates nothing. The live
//! SC-001/SC-002/SC-003 properties (right objective, real conflict, no real
//! fabrication) and the SC-004 output-marking are the dogfood — a mock can't
//! produce them; the recall + assembly are proven offline (here and in tests).
//!
//! Run: `cargo run --example acceptance_elicit`

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

fn inference(
    objective: &str,
    prefs: Value,
    signals: Value,
    strengths: Value,
    signal: &str,
) -> Value {
    json!({
        "assumed_objective": objective,
        "preference_texts": prefs,
        "preference_signals": signals,
        "preference_strengths": strengths,
        "divergence_questions": [],
        "divergence_signals": [],
        "signal_level": signal,
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

fn ev(task: &str) -> CallToolRequestParams {
    let mut p = CallToolRequestParams::new("elicit");
    p.arguments = json!({ "task": task }).as_object().cloned();
    p
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Surfaces the objective + traced preference; no memory → memory_consulted false;
    // no enforcement field in the output.
    {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(end_turn(&inference(
                    "Add a cache to speed up the endpoint",
                    json!(["p99 latency is the target, not average"]),
                    json!(["the request mentions tail latency"]),
                    json!(["stated"]),
                    "medium",
                ))),
            )
            .mount(&mock)
            .await;
        let (client, storage, _srv) = serve(&mock).await;
        let r = client.call_tool(ev("Speed up the endpoint")).await.unwrap();
        let s = r.structured_content.as_ref().unwrap();
        assert_eq!(
            s["assumed_objective"],
            "Add a cache to speed up the endpoint"
        );
        assert_eq!(s["governing_preferences"][0]["strength"], "stated");
        assert_eq!(s["memory_consulted"], false);
        assert!(s.get("verdict").is_none() && s.get("hold").is_none());
        assert_eq!(storage.list_invocations().await.unwrap().len(), 1);
        client.cancel().await.unwrap();
        println!("FR-001/002/006 objective + traced pref, no enforcement, one record: PASS");
    }

    // Low signal → fabricates nothing.
    {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(end_turn(&inference(
                    "Rename tmp to a clearer name",
                    json!([]),
                    json!([]),
                    json!([]),
                    "low",
                ))),
            )
            .mount(&mock)
            .await;
        let (client, _s, _srv) = serve(&mock).await;
        let r = client.call_tool(ev("rename tmp")).await.unwrap();
        let s = r.structured_content.as_ref().unwrap();
        assert_eq!(s["signal_level"], "low");
        assert!(s["governing_preferences"].as_array().unwrap().is_empty());
        assert!(s["divergence_points"].as_array().unwrap().is_empty());
        client.cancel().await.unwrap();
        println!("FR-005/SC-003 low signal ⇒ nothing fabricated: PASS");
    }

    println!("\nacceptance_elicit: ALL CHECKS PASS");
}
