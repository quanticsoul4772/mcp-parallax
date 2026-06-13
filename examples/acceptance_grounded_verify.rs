//! Acceptance for 008-grounded-verify (SC-001..006).
//!
//! Drives the real server (Anthropic mocked by wiremock) against a real
//! temp-dir source root: gating, verbatim-grounded verdict + manifest,
//! all-or-nothing named errors, root confinement, and the completeness signal.
//!
//! Run: `cargo run --example acceptance_grounded_verify`

#![allow(clippy::print_stdout)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value
)]

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

fn config(root: Option<String>) -> Config {
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
        grounded_verify_root: root,
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

async fn serve(
    mock: &MockServer,
    root: Option<String>,
) -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    Arc<SqliteStorage>,
    rmcp::service::RunningService<rmcp::service::RoleServer, Parallax>,
) {
    let cfg = config(root);
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

fn gv(claim: &str, locators: Value) -> CallToolRequestParams {
    let mut p = CallToolRequestParams::new("grounded_verify");
    p.arguments = json!({ "claim": claim, "locators": locators })
        .as_object()
        .cloned();
    p
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("pub.rs"),
        "pub fn publish(&self) { self.emit(); telemetry(); }\n",
    )
    .unwrap();
    let root = dir.path().to_str().unwrap().to_string();

    // SC-005: unconfigured ⇒ the tool is absent from the catalog.
    {
        let mock = MockServer::start().await;
        let (client, _s, _srv) = serve(&mock, None).await;
        let names: Vec<String> = client
            .list_all_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(!names.contains(&"grounded_verify".to_string()));
        client.cancel().await.unwrap();
        println!("SC-005 gating (unconfigured ⇒ absent): PASS");
    }

    // SC-001/002/006: verbatim source reaches the pass (mock matches only on the
    // file body), verdict + manifest returned, completeness surfaced.
    {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_string_contains("self.emit()"))
            .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
                "verdict": "supported",
                "findings": [],
                "missing_evidence": ["the definition of telemetry()"]
            }))))
            .mount(&mock)
            .await;
        let (client, storage, _srv) = serve(&mock, Some(root.clone())).await;
        let result = client
            .call_tool(gv(
                "publish emits the tracing event",
                json!([{ "path": "pub.rs", "start_line": 1, "end_line": 1 }]),
            ))
            .await
            .unwrap();
        let s = result.structured_content.as_ref().unwrap();
        assert_eq!(s["verdict"], "supported");
        assert_eq!(s["manifest"][0]["path"], "pub.rs");
        assert!(s["manifest"][0]["bytes"].as_u64().unwrap() > 0);
        assert_eq!(
            s["missing_evidence"],
            json!(["the definition of telemetry()"])
        );
        assert_eq!(storage.list_invocations().await.unwrap().len(), 1);
        client.cancel().await.unwrap();
        println!("SC-001/002/006 verbatim verdict + manifest + completeness: PASS");
    }

    // SC-003: an unresolvable locator aborts, named, with no verdict.
    {
        let mock = MockServer::start().await;
        let (client, _s, _srv) = serve(&mock, Some(root.clone())).await;
        let err = client
            .call_tool(gv("c", json!([{ "path": "gone.rs" }])))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("source not found: gone.rs"));
        client.cancel().await.unwrap();
        println!("SC-003 all-or-nothing named error: PASS");
    }

    // SC-004: a traversal locator is rejected before any read.
    {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
        let escape = format!(
            "../{}/secret.txt",
            outside.path().file_name().unwrap().to_str().unwrap()
        );
        let mock = MockServer::start().await;
        let (client, _s, _srv) = serve(&mock, Some(root.clone())).await;
        let err = client
            .call_tool(gv("c", json!([{ "path": escape }])))
            .await
            .unwrap_err();
        // Either "not found" or an explicit escape — in both, no content leaves.
        assert!(
            err.to_string().to_lowercase().contains("source")
                || err.to_string().contains("escapes")
        );
        client.cancel().await.unwrap();
        println!("SC-004 root confinement (traversal rejected): PASS");
    }

    println!("\nacceptance_grounded_verify: ALL CHECKS PASS");
}
