//! End-to-end integration tests: a real rmcp client against the real server
//! over an in-process duplex transport, with the Anthropic API mocked by
//! wiremock (localhost). No real network, no pre-existing disk state.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::AnthropicClient;
use mcp_parallax::config::Config;
use mcp_parallax::error::Outcome;
use mcp_parallax::server::Parallax;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::clock::SystemClock;
use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CONTRACT: &str = include_str!("../specs/001-core-layer/contracts/verify.tool.json");

fn test_config(timeout_ms: u64) -> Config {
    Config {
        anthropic_api_key: "test-key".into(),
        anthropic_model: "claude-opus-4-8".into(),
        verify_ensemble_k: 3,
        verify_max_claim_chars: 50_000,
        database_path: ":memory:".into(),
        log_level: "info".into(),
        request_timeout_ms: timeout_ms,
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

/// Build the full server against a wiremock-backed Anthropic endpoint and an
/// in-memory store; serve it over a duplex pipe; return the connected client
/// plus the storage handle for record assertions.
async fn serve(
    mock: &MockServer,
    timeout_ms: u64,
) -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    Arc<SqliteStorage>,
    rmcp::service::RunningService<rmcp::service::RoleServer, Parallax>,
) {
    let config = test_config(timeout_ms);
    let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
    let anthropic =
        Arc::new(AnthropicClient::with_base_url(&config, &mock.uri()).with_backoff_base_ms(1));
    let server = Parallax::new(anthropic, storage.clone(), Arc::new(SystemClock), &config).unwrap();

    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move { server.serve(server_io).await });
    let client = ().serve(client_io).await.expect("client init");
    // Keep the server's RunningService alive — dropping it closes the transport.
    let running_server = server_task.await.expect("join").expect("server init");
    (client, storage, running_server)
}

fn call(claim: &str) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new("verify");
    params.arguments = json!({ "claim": claim }).as_object().cloned();
    params
}

// ---- T013: catalog matches the contract --------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_lists_verify_with_the_contracted_schemas() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve(&mock, 2_000).await;

    let tools = client.list_all_tools().await.unwrap();
    let verify = tools
        .iter()
        .find(|t| t.name == "verify")
        .expect("verify listed");
    let contract: Value = serde_json::from_str(CONTRACT).unwrap();

    assert_eq!(
        verify.description.as_deref().unwrap(),
        contract["description"]
    );

    // Input schema: same property set and required list.
    let input = serde_json::to_value(verify.input_schema.as_ref()).unwrap();
    let props = |schema: &Value| -> Vec<String> {
        schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    };
    assert_eq!(props(&input), props(&contract["inputSchema"]));
    assert_eq!(input["required"], json!(["claim"]));

    // Output schema: same property set; verdict enum values present.
    let output = serde_json::to_value(
        verify
            .output_schema
            .as_ref()
            .expect("outputSchema advertised"),
    )
    .unwrap();
    assert_eq!(props(&output), props(&contract["outputSchema"]));

    client.cancel().await.unwrap();
}

// ---- T013 + acceptance 2-4: structured verdict end-to-end ---------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_returns_schema_valid_structured_content() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(
            &json!({ "verdict": "refuted", "findings": ["1066, not 1067"] }),
        )))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client.call_tool(call("Hastings was 1067")).await.unwrap();
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    // The result validates against the contract's (unsanitized) outputSchema.
    let contract: Value = serde_json::from_str(CONTRACT).unwrap();
    mcp_parallax::schema::validate(&contract["outputSchema"], structured).unwrap();

    assert_eq!(structured["verdict"], "refuted");
    assert_eq!(structured["findings"], json!(["1066, not 1067"]));
    assert_eq!(structured["passes"], 3);
    assert!((structured["confidence"].as_f64().unwrap() - 1.0).abs() < f64::EPSILON);

    // SC-007: exactly one record, success, with summed usage.
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::Success);
    assert_eq!(records[0].input_tokens, 300);

    client.cancel().await.unwrap();
}

// ---- T013: concurrency — results are never crossed ----------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_invocations_do_not_cross_results() {
    let mock = MockServer::start().await;
    // The prompt carries the claim verbatim — route distinct responses by claim.
    Mock::given(method("POST"))
        .and(body_string_contains("CLAIM-ALPHA"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(
            &json!({ "verdict": "refuted", "findings": ["alpha is wrong"] }),
        )))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("CLAIM-BETA"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(end_turn(&json!({ "verdict": "supported", "findings": [] }))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 5_000).await;

    let (alpha, beta) = tokio::join!(
        client.call_tool(call("CLAIM-ALPHA")),
        client.call_tool(call("CLAIM-BETA"))
    );
    let alpha = alpha.unwrap().structured_content.unwrap();
    let beta = beta.unwrap().structured_content.unwrap();

    assert_eq!(alpha["verdict"], "refuted");
    assert_eq!(alpha["findings"], json!(["alpha is wrong"]));
    assert_eq!(beta["verdict"], "supported");

    // T027: two invocations, two records.
    assert_eq!(storage.list_invocations().await.unwrap().len(), 2);

    client.cancel().await.unwrap();
}

// ---- T020: induced-failure matrix (US2) ---------------------------------

async fn expect_failure(mock: &MockServer, marker: &str, expected_outcome: Outcome) {
    let (client, storage, _server) = serve(mock, 500).await;

    let err = client.call_tool(call("any claim")).await.unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains(&format!("[{}]", expected_outcome.as_str())),
        "{marker}: error must name its class; got: {text}"
    );
    // Never a partial verdict; exactly one record with the class.
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1, "{marker}");
    assert_eq!(records[0].outcome, expected_outcome, "{marker}");

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refusal_surfaces_as_refusal() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [],
            "stop_reason": "refusal",
            "usage": { "input_tokens": 10, "output_tokens": 0 }
        })))
        .mount(&mock)
        .await;
    expect_failure(&mock, "refusal", Outcome::Refusal).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn truncation_surfaces_as_truncation_never_salvaged() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{ "type": "text", "text": "{\"verdict\": \"refu" }],
            "stop_reason": "max_tokens",
            "usage": { "input_tokens": 10, "output_tokens": 4096 }
        })))
        .mount(&mock)
        .await;
    expect_failure(&mock, "truncation", Outcome::Truncation).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exhausted_retries_surface_with_attempt_count() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock)
        .await;
    expect_failure(&mock, "retries", Outcome::RetriesExhausted).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_provider_surfaces_as_timeout() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(end_turn(&json!({ "verdict": "supported", "findings": [] })))
                .set_delay(Duration::from_secs(10)),
        )
        .mount(&mock)
        .await;
    expect_failure(&mock, "timeout", Outcome::Timeout).await;
}

// ---- T020/T027: cancellation — client drops mid-invocation --------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_disconnect_mid_invocation_records_cancelled() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(end_turn(&json!({ "verdict": "supported", "findings": [] })))
                .set_delay(Duration::from_mins(1)),
        )
        .mount(&mock)
        .await;
    let (client, storage, server) = serve(&mock, 120_000).await;

    // Fire the call, then tear the whole client connection down while the
    // invocation is in flight (the stdio reality: client gone → EOF → the
    // service shuts down and aborts in-flight request tasks).
    let call_task = tokio::spawn(async move { client.call_tool(call("abandoned")).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    call_task.abort();
    let _ = call_task.await;
    let _ = server.cancel().await;

    // Wait for the guard's cancelled record to land.
    let mut records = Vec::new();
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        records = storage.list_invocations().await.unwrap();
        if !records.is_empty() {
            break;
        }
    }
    assert_eq!(records.len(), 1, "abandoned invocation must still record");
    assert_eq!(records[0].outcome, Outcome::Cancelled);
}

// ---- T021: startup failures name the exact variable ----------------------

#[test]
fn startup_without_api_key_exits_naming_the_variable() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mcp-parallax"))
        .env_remove("ANTHROPIC_API_KEY")
        .output()
        .expect("binary spawns");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ANTHROPIC_API_KEY"), "stderr: {stderr}");
}

#[test]
fn startup_with_zero_ensemble_k_exits_naming_the_variable() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mcp-parallax"))
        .env("ANTHROPIC_API_KEY", "dummy")
        .env("VERIFY_ENSEMBLE_K", "0")
        .output()
        .expect("binary spawns");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("VERIFY_ENSEMBLE_K"), "stderr: {stderr}");
}

// ---- T018: spawn-the-binary stdio smoke test (FR-008) --------------------

#[test]
fn stdio_smoke_test_stdout_carries_only_protocol_frames() {
    use std::io::{BufRead, BufReader, Write};

    let db = std::env::temp_dir().join(format!("parallax-smoke-{}.db", uuid::Uuid::new_v4()));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_mcp-parallax"))
        .env("ANTHROPIC_API_KEY", "dummy-key-no-model-call-happens")
        .env("DATABASE_PATH", db.to_string_lossy().to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("binary spawns");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();

    // initialize → response must be the FIRST bytes on stdout, valid JSON-RPC.
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"smoke","version":"0"}}}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    stdout.read_line(&mut line).unwrap();
    let init: Value = serde_json::from_str(&line).expect("first stdout line is JSON-RPC");
    assert_eq!(init["jsonrpc"], "2.0");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    // initialized notification, then tools/list.
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    stdin.flush().unwrap();

    line.clear();
    stdout.read_line(&mut line).unwrap();
    let tools: Value = serde_json::from_str(&line).expect("tools/list reply is JSON-RPC");
    let mut names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["unstick", "verify"]);

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&db);
}

// ====== 002-unstick-mode (US2: guarantee parity) ==========================

const UNSTICK_CONTRACT: &str =
    include_str!("../specs/002-unstick-mode/contracts/unstick.tool.json");

fn unstick_call(goal: &str, blocked: &str, tried: &[&str]) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new("unstick");
    params.arguments = json!({ "goal": goal, "blocked": blocked, "tried": tried })
        .as_object()
        .cloned();
    params
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_lists_unstick_with_the_contracted_schemas() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve(&mock, 2_000).await;

    let tools = client.list_all_tools().await.unwrap();
    let unstick = tools
        .iter()
        .find(|t| t.name == "unstick")
        .expect("unstick listed");
    let contract: Value = serde_json::from_str(UNSTICK_CONTRACT).unwrap();

    assert_eq!(
        unstick.description.as_deref().unwrap(),
        contract["description"]
    );
    let props = |schema: &Value| -> Vec<String> {
        schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    };
    let input = serde_json::to_value(unstick.input_schema.as_ref()).unwrap();
    assert_eq!(props(&input), props(&contract["inputSchema"]));
    let output = serde_json::to_value(
        unstick
            .output_schema
            .as_ref()
            .expect("outputSchema advertised"),
    )
    .unwrap();
    assert_eq!(props(&output), props(&contract["outputSchema"]));

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unstick_returns_schema_valid_structured_step_and_one_record() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "next_step": "Bisect the failing config by halving the overrides file",
            "rationale": "Halving isolates the bad key in O(log n) runs.",
            "watch_for": "Overrides that only fail in combination"
        }))))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(unstick_call(
            "find the bad config key",
            "service crashes on boot with the full overrides file",
            &["reading the file top to bottom", "grepping for typos"],
        ))
        .await
        .unwrap();
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    let contract: Value = serde_json::from_str(UNSTICK_CONTRACT).unwrap();
    mcp_parallax::schema::validate(&contract["outputSchema"], structured).unwrap();
    assert!(structured["next_step"]
        .as_str()
        .unwrap()
        .starts_with("Bisect"));

    // Exactly one record, attributed to unstick, single-pass usage (not x3).
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "unstick");
    assert_eq!(records[0].outcome, Outcome::Success);
    assert_eq!(records[0].input_tokens, 100);

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unstick_failures_use_the_same_distinct_classes() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [],
            "stop_reason": "refusal",
            "usage": { "input_tokens": 10, "output_tokens": 0 }
        })))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 500).await;

    let err = client
        .call_tool(unstick_call("g", "b", &[]))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("[refusal]"),
        "same class naming as verify; got: {err}"
    );
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "unstick");
    assert_eq!(records[0].outcome, Outcome::Refusal);

    client.cancel().await.unwrap();
}
