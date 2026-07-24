//! End-to-end integration tests: a real rmcp client against the real server
//! over an in-process duplex transport, with the Anthropic API mocked by
//! wiremock (localhost). No real network, no pre-existing disk state.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value
)]

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
        // The developer machine may carry a real key — this test asserts the
        // capability-OFF catalog (FR-007).
        .env_remove("VOYAGE_API_KEY")
        .env_remove("BRAVE_API_KEY")
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
    assert_eq!(
        names,
        vec![
            "check",
            "checkpoint_action",
            "checkpoint_batch",
            "checkpoint_turn",
            "decide",
            "diverge",
            "elicit",
            "unstick",
            "verify"
        ]
    );

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

// ====== 003-memory-layer ====================================================

use mcp_parallax::client::VoyageClient;
use mcp_parallax::traits::storage::Storage;
use wiremock::Request;

/// Build the full server with the memory capability ON: a real `VoyageClient`
/// pointed at the same wiremock server (`/v1/embeddings` beside
/// `/v1/messages`), persisted to `db_path`.
async fn serve_with_memory(
    mock: &MockServer,
    db_path: &str,
) -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    Arc<SqliteStorage>,
    rmcp::service::RunningService<rmcp::service::RoleServer, Parallax>,
) {
    let mut config = test_config(2_000);
    config.voyage_api_key = Some("voyage-test-key".into());
    let storage = Arc::new(SqliteStorage::connect(db_path).await.unwrap());
    let anthropic =
        Arc::new(AnthropicClient::with_base_url(&config, &mock.uri()).with_backoff_base_ms(1));
    let voyage = Arc::new(
        VoyageClient::with_base_url(&config, &mock.uri())
            .unwrap()
            .with_backoff_base_ms(1),
    );
    let server = Parallax::with_embedder(
        anthropic,
        storage.clone(),
        Arc::new(SystemClock),
        &config,
        Some(voyage),
    )
    .unwrap();

    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move { server.serve(server_io).await });
    let client = ().serve(client_io).await.expect("client init");
    let running_server = server_task.await.expect("join").expect("server init");
    (client, storage, running_server)
}

fn tool_call(name: &str, arguments: &Value) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new(name.to_string());
    params.arguments = arguments.as_object().cloned();
    params
}

/// Deterministic per-content embeddings: distinct contents get nearly
/// orthogonal vectors; the query lands closest to the "alpha" document.
fn mount_embeddings(mock: &MockServer) -> impl std::future::Future<Output = ()> + '_ {
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(|req: &Request| {
            let body: Value = req.body_json().unwrap();
            let text = body["input"][0].as_str().unwrap();
            let vector: Vec<f32> = if text.contains("alpha") {
                vec![1.0, 0.0]
            } else if text.contains("beta") {
                vec![0.0, 1.0]
            } else {
                // The recall query — closest to alpha.
                vec![0.9, 0.1]
            };
            ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "embedding": vector, "index": 0 }],
                "usage": { "total_tokens": 5 }
            }))
        })
        .mount(mock)
}

// ---- T014: catalog gating (FR-007) ----------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_voyage_key_the_catalog_has_no_memory_tools() {
    let mock = MockServer::start().await;
    // serve() builds the server via Parallax::new with voyage_api_key = None —
    // and construction must not touch the network (no mounts needed).
    let (client, _storage, _server) = serve(&mock, 2_000).await;

    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "check",
            "checkpoint_action",
            "checkpoint_batch",
            "checkpoint_turn",
            "decide",
            "diverge",
            "elicit",
            "unstick",
            "verify"
        ]
    );

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_a_voyage_key_the_catalog_matches_the_memory_contracts() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve_with_memory(&mock, ":memory:").await;

    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "check",
            "checkpoint_action",
            "checkpoint_batch",
            "checkpoint_turn",
            "decide",
            "diverge",
            "elicit",
            "forget",
            "recall",
            "save",
            "surface",
            "unstick",
            "verify"
        ]
    );

    // Descriptions and schema property sets match the contract files.
    let props = |schema: &Value| -> Vec<String> {
        schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    };
    for (name, contract_text) in [
        (
            "save",
            include_str!("../specs/003-memory-layer/contracts/save.tool.json"),
        ),
        (
            "recall",
            include_str!("../specs/003-memory-layer/contracts/recall.tool.json"),
        ),
        (
            "forget",
            include_str!("../specs/003-memory-layer/contracts/forget.tool.json"),
        ),
    ] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        let contract: Value = serde_json::from_str(contract_text).unwrap();
        assert_eq!(
            tool.description.as_deref().unwrap(),
            contract["description"],
            "{name} description"
        );
        let input = serde_json::to_value(tool.input_schema.as_ref()).unwrap();
        assert_eq!(
            props(&input),
            props(&contract["inputSchema"]),
            "{name} input"
        );
        let output =
            serde_json::to_value(tool.output_schema.as_ref().expect("outputSchema")).unwrap();
        assert_eq!(
            props(&output),
            props(&contract["outputSchema"]),
            "{name} output"
        );
    }

    client.cancel().await.unwrap();
}

// ---- T015: save → recall round trip, records, attribution ------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_then_recall_returns_the_relevant_memory_first_with_records() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    // Two first-hand saves with distinct embeddings.
    let saved_alpha = client
        .call_tool(tool_call(
            "save",
            &json!({
                "content": "alpha: pin the toolchain before bumping MSRV",
                "kind": "lesson",
                "origin": "this session",
                "external": false
            }),
        ))
        .await
        .unwrap();
    let alpha = saved_alpha.structured_content.as_ref().unwrap();
    assert_eq!(alpha["trust"], "first_hand");
    client
        .call_tool(tool_call(
            "save",
            &json!({
                "content": "beta: wiremock mounts are additive",
                "kind": "fact",
                "origin": "this session",
                "external": false
            }),
        ))
        .await
        .unwrap();

    // The query embeds closest to alpha — it must come back first.
    let recalled = client
        .call_tool(tool_call(
            "recall",
            &json!({ "query": "toolchain pinning" }),
        ))
        .await
        .unwrap();
    let structured = recalled.structured_content.as_ref().unwrap();
    let contract: Value = serde_json::from_str(include_str!(
        "../specs/003-memory-layer/contracts/recall.tool.json"
    ))
    .unwrap();
    mcp_parallax::schema::validate(&contract["outputSchema"], structured).unwrap();
    let memories = structured["memories"].as_array().unwrap();
    assert_eq!(memories.len(), 2);
    assert!(memories[0]["content"].as_str().unwrap().contains("alpha"));
    assert_eq!(memories[0]["id"], alpha["id"]);
    assert!(memories[0]["score"].as_f64().unwrap() > memories[1]["score"].as_f64().unwrap());

    // One record per call, attributed to its tool and the embedding model.
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 3);
    let mut tools_seen: Vec<&str> = records.iter().map(|r| r.tool.as_str()).collect();
    tools_seen.sort_unstable();
    assert_eq!(tools_seen, ["recall", "save", "save"]);
    assert!(records.iter().all(|r| r.model == "voyage-4"));
    assert!(records.iter().all(|r| r.outcome == Outcome::Success));

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verifying_save_runs_the_ensemble_and_attributes_the_anthropic_model() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(end_turn(&json!({ "verdict": "supported", "findings": [] }))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    let saved = client
        .call_tool(tool_call(
            "save",
            &json!({
                "content": "alpha claim from a blog post",
                "kind": "fact",
                "origin": "https://example.com/post",
                "external": true,
                "verify": true
            }),
        ))
        .await
        .unwrap();
    let structured = saved.structured_content.as_ref().unwrap();
    assert_eq!(structured["trust"], "verified");

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "save");
    assert_eq!(records[0].model, "claude-opus-4-8");
    // Ensemble usage (3 × 100) plus the embedding tokens (5).
    assert_eq!(records[0].input_tokens, 305);

    client.cancel().await.unwrap();
}

// ---- T013: trust banding end-to-end (FR-004) -------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_equal_relevance_first_hand_outranks_untrusted_and_both_are_labeled() {
    let mock = MockServer::start().await;
    // Both contents contain "alpha" — identical embeddings, equal relevance.
    mount_embeddings(&mock).await;
    let (client, _storage, _server) = serve_with_memory(&mock, ":memory:").await;

    // Save the untrusted one FIRST so insertion order can't fake the win.
    client
        .call_tool(tool_call(
            "save",
            &json!({ "content": "alpha hearsay from a forum", "kind": "fact",
                    "origin": "forum", "external": true }),
        ))
        .await
        .unwrap();
    client
        .call_tool(tool_call(
            "save",
            &json!({ "content": "alpha observed directly", "kind": "fact",
                    "origin": "this session", "external": false }),
        ))
        .await
        .unwrap();

    let recalled = client
        .call_tool(tool_call("recall", &json!({ "query": "the thing" })))
        .await
        .unwrap();
    let memories = recalled.structured_content.unwrap()["memories"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(memories.len(), 2);
    assert_eq!(memories[0]["trust"], "first_hand");
    assert_eq!(memories[0]["external"], false);
    assert_eq!(memories[1]["trust"], "untrusted");
    assert_eq!(memories[1]["external"], true);

    client.cancel().await.unwrap();
}

// ---- T015: embedding provider failure surfaces with its class --------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedding_provider_failure_surfaces_named_with_one_record() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid input"))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    let err = client
        .call_tool(tool_call(
            "save",
            &json!({
                "content": "anything",
                "kind": "fact",
                "origin": "here",
                "external": false
            }),
        ))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("[embedding_provider]"),
        "got: {err}"
    );

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::EmbeddingProvider);

    client.cancel().await.unwrap();
}

// ---- T015: forget is permanent, across store reopen -------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forget_is_permanent_across_store_reopen() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    let db = std::env::temp_dir().join(format!("parallax-it-{}.db", uuid::Uuid::new_v4()));
    let db_path = db.to_string_lossy().to_string();

    {
        let (client, _storage, server) = serve_with_memory(&mock, &db_path).await;
        let keep = client
            .call_tool(tool_call(
                "save",
                &json!({ "content": "alpha keeper", "kind": "skill",
                        "origin": "s", "external": false }),
            ))
            .await
            .unwrap();
        let drop_me = client
            .call_tool(tool_call(
                "save",
                &json!({ "content": "beta to forget", "kind": "skill",
                        "origin": "s", "external": false }),
            ))
            .await
            .unwrap();
        let drop_id = drop_me.structured_content.unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        let forgotten = client
            .call_tool(tool_call("forget", &json!({ "id": drop_id.clone() })))
            .await
            .unwrap();
        assert_eq!(forgotten.structured_content.unwrap()["forgotten"], true);

        // A second forget of the same id is a distinct not-found error.
        let err = client
            .call_tool(tool_call("forget", &json!({ "id": drop_id })))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("[invalid_input]"), "got: {err}");
        assert!(err.to_string().contains("no memory with id"), "got: {err}");

        let _ = keep;
        client.cancel().await.unwrap();
        let _ = server.cancel().await;
    }

    // Reopen the store fresh — only the kept memory survives.
    let reopened = SqliteStorage::connect(&db_path).await.unwrap();
    let memories = reopened.load_memories().await.unwrap();
    assert_eq!(memories.len(), 1);
    assert!(memories[0].content.contains("alpha keeper"));
    drop(reopened);
    let _ = std::fs::remove_file(&db);
}

// ====== 004-research-layer ==================================================

use mcp_parallax::client::BraveClient;

const RESEARCH_CONTRACT: &str =
    include_str!("../specs/004-research-layer/contracts/research.tool.json");

/// Build the full server with research ON: a real `BraveClient` pointed at
/// wiremock (`/res/v1/web/search`), pages and `/v1/messages` served by the
/// same mock server. The fetcher is the real `HygieneFetcher` hitting
/// wiremock over localhost.
async fn serve_with_research(
    mock: &MockServer,
) -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    Arc<SqliteStorage>,
    rmcp::service::RunningService<rmcp::service::RoleServer, Parallax>,
) {
    let mut config = test_config(5_000);
    config.brave_api_key = Some("brave-test-key".into());
    config.fetch_timeout_ms = 3_000;
    // Integration pages are served by wiremock on localhost.
    config.fetch_allow_private = true;
    let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
    let anthropic =
        Arc::new(AnthropicClient::with_base_url(&config, &mock.uri()).with_backoff_base_ms(1));
    let brave = Arc::new(
        BraveClient::with_base_url(&config, &mock.uri())
            .unwrap()
            .with_backoff_base_ms(1),
    );
    let server = Parallax::with_capabilities(
        anthropic,
        storage.clone(),
        Arc::new(SystemClock),
        &config,
        None,
        Some(brave),
    )
    .unwrap();

    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move { server.serve(server_io).await });
    let client = ().serve(client_io).await.expect("client init");
    let running_server = server_task.await.expect("join").expect("server init");
    (client, storage, running_server)
}

/// Route the four research model hops by their prompt markers.
async fn mount_research_llm(mock: &MockServer, answer: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("scoping a web research run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "angles": ["first angle"],
            "sub_questions": ["does it hold?"]
        }))))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("extract falsifiable claims"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(
            &json!({ "claims": ["the documented claim holds"] }),
        )))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("adversarial fact-checker"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(end_turn(&json!({ "verdict": "supported", "findings": [] }))),
        )
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("executive synthesis"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(end_turn(&json!({ "answer": answer, "gaps": [] }))),
        )
        .mount(mock)
        .await;
}

async fn mount_search_and_page(mock: &MockServer) {
    let page_url = format!("{}/page1", mock.uri());
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "web": { "results": [
                { "url": page_url, "title": "The Documented Page", "description": "doc" }
            ]}
        })))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/page1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                "<html><head><title>The Documented Page</title></head><body><article>\
             <h1>Heading</h1><p>The documented claim holds, as this page explains at \
             length with enough running text for extraction to keep it as main \
             content.</p></article></body></html>"
                    .to_string(),
                "text/html; charset=utf-8",
            ),
        )
        .mount(mock)
        .await;
}

// ---- T017: catalog gating ---------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_a_brave_key_the_catalog_matches_the_research_contract() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve_with_research(&mock).await;

    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "check",
            "checkpoint_action",
            "checkpoint_batch",
            "checkpoint_turn",
            "decide",
            "diverge",
            "elicit",
            "research",
            "unstick",
            "verify"
        ]
    );

    let research = tools.iter().find(|t| t.name == "research").unwrap();
    let contract: Value = serde_json::from_str(RESEARCH_CONTRACT).unwrap();
    assert_eq!(
        research.description.as_deref().unwrap(),
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
    let input = serde_json::to_value(research.input_schema.as_ref()).unwrap();
    assert_eq!(props(&input), props(&contract["inputSchema"]));
    let output =
        serde_json::to_value(research.output_schema.as_ref().expect("outputSchema")).unwrap();
    assert_eq!(props(&output), props(&contract["outputSchema"]));

    client.cancel().await.unwrap();
}

// ---- T014: full round trip ----------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn research_round_trip_returns_grounded_citations_and_one_record() {
    let mock = MockServer::start().await;
    mount_search_and_page(&mock).await;
    mount_research_llm(&mock, "The claim holds [s1].").await;
    let (client, storage, _server) = serve_with_research(&mock).await;

    let result = client
        .call_tool(tool_call(
            "research",
            &json!({ "question": "does the documented claim hold?", "depth": "quick" }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();

    // The wire shape validates against the contract.
    let contract: Value = serde_json::from_str(RESEARCH_CONTRACT).unwrap();
    mcp_parallax::schema::validate(&contract["outputSchema"], structured).unwrap();

    // Every cited id resolves; sources carry identity, never bodies (FR-012).
    assert_eq!(structured["answer"], "The claim holds [s1].");
    let sources = structured["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["id"], "s1");
    assert!(sources[0]["url"].as_str().unwrap().ends_with("/page1"));
    let wire = structured.to_string();
    assert!(!wire.contains("running text for extraction"));

    let findings = structured["key_findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["support"], "unverified"); // single source — never fact
    assert_eq!(structured["stats"]["sources_fetched"], 1);
    assert_eq!(structured["stats"]["stopped_early"], false);

    // Exactly one record, attributed to research on the anthropic model.
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "research");
    assert_eq!(records[0].model, "claude-opus-4-8");
    assert_eq!(records[0].outcome, Outcome::Success);
    assert!(records[0].input_tokens > 0);

    client.cancel().await.unwrap();
}

// ---- T016 (integration): budget ceiling returns early, not an error ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tiny_budget_returns_a_well_formed_early_synthesized_result() {
    let mock = MockServer::start().await;
    mount_search_and_page(&mock).await;
    // The scope call alone consumes 2000 input tokens against a 1000 budget.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("scoping a web research run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{ "type": "text", "text": json!({
                "angles": ["first angle"], "sub_questions": ["q1"]
            }).to_string() }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 2000, "output_tokens": 10 }
        })))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_research(&mock).await;

    let result = client
        .call_tool(tool_call(
            "research",
            &json!({
                "question": "q?",
                "constraints": { "budget_tokens": 1000 }
            }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();

    let contract: Value = serde_json::from_str(RESEARCH_CONTRACT).unwrap();
    mcp_parallax::schema::validate(&contract["outputSchema"], structured).unwrap();
    assert_eq!(structured["stats"]["stopped_early"], true);
    assert_eq!(structured["stats"]["stop_reason"], "budget");
    assert!(!structured["answer"].as_str().unwrap().is_empty());

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::Success); // early stop is not an error

    client.cancel().await.unwrap();
}

// ---- T018: failure parity -----------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_provider_outage_surfaces_named_with_one_record() {
    let mock = MockServer::start().await;
    mount_research_llm(&mock, "n/a").await;
    // Terminal 422 from Brave (e.g. an invalid subscription token).
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(422).set_body_string("SUBSCRIPTION_TOKEN_INVALID"))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_research(&mock).await;

    let err = client
        .call_tool(tool_call("research", &json!({ "question": "q?" })))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("[search_provider]"), "got: {err}");

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::SearchProvider);

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn research_invalid_input_is_rejected_pre_provider_with_one_record() {
    let mock = MockServer::start().await;
    // Nothing mounted: any provider call would 404 and fail differently.
    let (client, storage, _server) = serve_with_research(&mock).await;

    let err = client
        .call_tool(tool_call("research", &json!({ "question": "   " })))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("[invalid_input]"), "got: {err}");

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::InvalidInput);

    client.cancel().await.unwrap();
}

// ====== 005-deterministic-layer =============================================

const CHECK_CONTRACT: &str =
    include_str!("../specs/005-deterministic-layer/contracts/check.tool.json");

/// Route the check translation hop by its prompt marker.
async fn mount_check_translation(mock: &MockServer, translation: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("deterministic checking"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&translation)))
        .mount(mock)
        .await;
}

// ---- T008: catalog — check is always on, no keys required (SC-005) ---------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_is_in_the_catalog_with_no_capability_keys_and_matches_the_contract() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve(&mock, 2_000).await;

    let tools = client.list_all_tools().await.unwrap();
    let check = tools
        .iter()
        .find(|t| t.name == "check")
        .expect("check listed");
    let contract: Value = serde_json::from_str(CHECK_CONTRACT).unwrap();
    assert_eq!(
        check.description.as_deref().unwrap(),
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
    let input = serde_json::to_value(check.input_schema.as_ref()).unwrap();
    assert_eq!(props(&input), props(&contract["inputSchema"]));
    let output = serde_json::to_value(check.output_schema.as_ref().expect("outputSchema")).unwrap();
    assert_eq!(props(&output), props(&contract["outputSchema"]));

    client.cancel().await.unwrap();
}

// ---- T008: ground-truth round trip — the REAL engines execute --------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_arithmetic_round_trip_returns_an_engine_decided_verdict() {
    let mock = MockServer::start().await;
    mount_check_translation(
        &mock,
        json!({
            "checkable": true, "reason": null, "engine": "arithmetic",
            "arithmetic_expression": "math::abs(1840 * 0.63 - 1159.2) <= 0.001",
            "smtlib_constraints": null, "asserted": null
        }),
    )
    .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(tool_call(
            "check",
            &json!({ "claim": "a 37% reduction from 1840 ms leaves 1159.2 ms" }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();

    let contract: Value = serde_json::from_str(CHECK_CONTRACT).unwrap();
    mcp_parallax::schema::validate(&contract["outputSchema"], structured).unwrap();
    assert_eq!(structured["verdict"], "supported");
    assert_eq!(structured["engine"], "arithmetic");
    assert!(structured["formal_form"]
        .as_str()
        .unwrap()
        .contains("math::abs"));
    assert_eq!(structured["engine_result"], "true");
    assert_eq!(structured["translation_attempts"], 1);

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "check");
    assert_eq!(records[0].model, "claude-opus-4-8");
    assert_eq!(records[0].outcome, Outcome::Success);

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_refuted_impossibility_carries_the_solver_witness() {
    let mock = MockServer::start().await;
    mount_check_translation(
        &mock,
        json!({
            "checkable": true, "reason": null, "engine": "constraints",
            "arithmetic_expression": null,
            "smtlib_constraints": "(declare-const a Int)\n(declare-const b Int)\n(declare-const c Int)\n(assert (< a b))\n(assert (< b c))\n(assert (< c a))",
            "asserted": "unsatisfiable"
        }),
    )
    .await;
    let (client, _storage, _server) = serve(&mock, 5_000).await;

    // The cyclic ordering really is unsatisfiable — the claim asserting
    // impossibility is SUPPORTED, proven by the solver.
    let result = client
        .call_tool(tool_call(
            "check",
            &json!({ "claim": "you cannot order a, b, c so each is less than the next cyclically" }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "supported");
    assert_eq!(structured["engine"], "constraints");
    assert_eq!(structured["engine_result"], "unsat");
    assert!(structured["explanation"]
        .as_str()
        .unwrap()
        .contains("no assignment exists"));

    client.cancel().await.unwrap();
}

// ---- T009: the honest decline path ------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_declines_uncheckable_claims_with_one_success_record() {
    let mock = MockServer::start().await;
    mount_check_translation(
        &mock,
        json!({
            "checkable": false, "reason": "elegance is a judgment call",
            "engine": null, "arithmetic_expression": null,
            "smtlib_constraints": null, "asserted": null
        }),
    )
    .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(tool_call(
            "check",
            &json!({ "claim": "Rust is more elegant than C++" }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "not_checkable");
    assert!(structured["reason"].as_str().unwrap().contains("judgment"));
    assert!(structured["engine"].is_null());
    assert!(structured["formal_form"].is_null());
    assert!(structured["engine_result"].is_null());
    assert!(structured["witness"].is_null());

    // The honest decline is a SUCCESS, not an error class.
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::Success);

    client.cancel().await.unwrap();
}

// ---- T010: the feedback loop end to end --------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_recovers_from_a_malformed_translation_via_one_violation_fed_retry() {
    let mock = MockServer::start().await;
    // First attempt: wrong dialect (abs is not whitelisted). The retry prompt
    // carries the engine violation; second attempt is valid.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("deterministic checking"))
        .and(body_string_contains("REJECTED"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "checkable": true, "reason": null, "engine": "arithmetic",
            "arithmetic_expression": "math::abs(2.0 - 2.0) <= 0.1",
            "smtlib_constraints": null, "asserted": null
        }))))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("deterministic checking"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "checkable": true, "reason": null, "engine": "arithmetic",
            "arithmetic_expression": "abs(2.0 - 2.0) <= 0.1",
            "smtlib_constraints": null, "asserted": null
        }))))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(tool_call("check", &json!({ "claim": "2 is about 2" })))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "supported");
    assert_eq!(structured["translation_attempts"], 2);

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::Success);

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_double_translation_failure_surfaces_as_validation_failure() {
    let mock = MockServer::start().await;
    mount_check_translation(
        &mock,
        json!({
            "checkable": true, "reason": null, "engine": "arithmetic",
            "arithmetic_expression": "((( not an expression",
            "smtlib_constraints": null, "asserted": null
        }),
    )
    .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let err = client
        .call_tool(tool_call("check", &json!({ "claim": "c" })))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("[validation_failure]"),
        "got: {err}"
    );
    assert!(err.to_string().contains("translation failed"), "got: {err}");

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::ValidationFailure);

    client.cancel().await.unwrap();
}

// ---- 006-checkpoint-layer ----------------------------------------------

const CHECKPOINT_ACTION_CONTRACT: &str =
    include_str!("../specs/006-checkpoint-layer/contracts/checkpoint_action.tool.json");
const CHECKPOINT_BATCH_CONTRACT: &str =
    include_str!("../specs/006-checkpoint-layer/contracts/checkpoint_batch.tool.json");
const CHECKPOINT_TURN_CONTRACT: &str =
    include_str!("../specs/006-checkpoint-layer/contracts/checkpoint_turn.tool.json");

/// Write a transcript fixture: `commands` as one bash `tool_use` line each
/// (failing when marked), for session `session`.
fn write_transcript(dir: &std::path::Path, session: &str, commands: &[(&str, bool)]) -> String {
    use std::io::Write as _;
    let path = dir.join("transcript.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    for (i, (command, failed)) in commands.iter().enumerate() {
        writeln!(
            file,
            "{}",
            json!({
                "type": "assistant",
                "sessionId": session,
                "message": { "role": "assistant", "content": [
                    { "type": "tool_use", "id": format!("t{i}"), "name": "Bash",
                      "input": { "command": command } }
                ]}
            })
        )
        .unwrap();
        if *failed {
            writeln!(
                file,
                "{}",
                json!({
                    "type": "user",
                    "sessionId": session,
                    "message": { "role": "user", "content": [
                        { "type": "tool_result", "tool_use_id": format!("t{i}"), "is_error": true }
                    ]}
                })
            )
            .unwrap();
        }
    }
    path.to_string_lossy().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_tools_match_their_contracts() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve(&mock, 2_000).await;

    let tools = client.list_all_tools().await.unwrap();
    for (name, contract_text) in [
        ("checkpoint_action", CHECKPOINT_ACTION_CONTRACT),
        ("checkpoint_batch", CHECKPOINT_BATCH_CONTRACT),
        ("checkpoint_turn", CHECKPOINT_TURN_CONTRACT),
    ] {
        let tool = tools.iter().find(|t| t.name == name).expect(name);
        let contract: Value = serde_json::from_str(contract_text).unwrap();
        assert_eq!(
            tool.description.as_deref().unwrap(),
            contract["description"],
            "{name}: description diverges from the contract"
        );
        let input = serde_json::to_value(tool.input_schema.as_ref()).unwrap();
        let props = |schema: &Value| -> Vec<String> {
            let mut names: Vec<String> = schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            names.sort();
            names
        };
        assert_eq!(props(&input), props(&contract["params"]), "{name}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_batch_flags_a_seeded_loop_with_both_records() {
    let mock = MockServer::start().await;
    let (client, storage, _server) = serve(&mock, 2_000).await;
    let dir = tempfile::tempdir().unwrap();
    let path = write_transcript(
        dir.path(),
        "cs-loop",
        &[
            ("cargo test", true),
            ("cargo test", true),
            ("cargo test", true),
            ("cargo test", true),
        ],
    );

    let result = client
        .call_tool(tool_call(
            "checkpoint_batch",
            &json!({ "session_id": "cs-loop", "transcript_path": path }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "flag");
    let message = structured["message"].as_str().unwrap();
    assert!(message.contains("cargo test"), "{message}");
    assert_eq!(structured["fail_open"], false);

    // Exactly one invocation record AND one checkpoint record (FR-006).
    let invocations = storage.list_invocations().await.unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].tool, "checkpoint_batch");
    assert_eq!(invocations[0].outcome, Outcome::Success);
    let checkpoints = storage.list_checkpoints().await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].verdict.as_str(), "flag");
    assert!(!checkpoints[0].signals_fired.is_empty());

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_batch_is_silent_on_a_benign_transcript() {
    let mock = MockServer::start().await;
    let (client, storage, _server) = serve(&mock, 2_000).await;
    let dir = tempfile::tempdir().unwrap();
    let path = write_transcript(
        dir.path(),
        "cs-benign",
        &[
            ("cargo build", false),
            ("cargo test", true),
            ("cargo fmt", false),
            ("cargo test", false),
        ],
    );

    let result = client
        .call_tool(tool_call(
            "checkpoint_batch",
            &json!({ "session_id": "cs-benign", "transcript_path": path }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "silence");
    assert_eq!(structured["message"], Value::Null);
    assert_eq!(structured["signals"], json!([]));

    let checkpoints = storage.list_checkpoints().await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].verdict.as_str(), "silence");

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_fails_open_when_the_transcript_is_unreadable() {
    let mock = MockServer::start().await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    // A missing transcript is an evaluation failure - the verdict is a
    // recorded fail-open silence, NOT an error (FR-008/SC-004).
    let result = client
        .call_tool(tool_call(
            "checkpoint_batch",
            &json!({ "session_id": "cs-x", "transcript_path": "missing/never.jsonl" }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "silence");
    assert_eq!(structured["fail_open"], true);

    let invocations = storage.list_invocations().await.unwrap();
    assert_eq!(invocations[0].outcome, Outcome::Success);
    let checkpoints = storage.list_checkpoints().await.unwrap();
    assert!(checkpoints[0].fail_open);

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_action_passes_non_risk_and_continuation_turns_are_silent() {
    let mock = MockServer::start().await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    // Non-risk-matched action: silence, no evaluation (FR-013) - and no
    // memory capability is configured, so nothing else could run anyway.
    let result = client
        .call_tool(tool_call(
            "checkpoint_action",
            &json!({
                "session_id": "cs-a",
                "transcript_path": "unused.jsonl",
                "tool_name": "Read",
                "tool_input": "src/main.rs"
            }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "silence");
    assert_eq!(structured["fail_open"], false);

    // A continuation turn end never reviews again (FR-014).
    let result = client
        .call_tool(tool_call(
            "checkpoint_turn",
            &json!({
                "session_id": "cs-a",
                "transcript_path": "unused.jsonl",
                "final_message": "Reconciled the two statements explicitly.",
                "continuation": true
            }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "silence");

    let checkpoints = storage.list_checkpoints().await.unwrap();
    assert_eq!(checkpoints.len(), 2);
    assert!(checkpoints.iter().all(|c| !c.review_ran));
    let invocations = storage.list_invocations().await.unwrap();
    assert_eq!(invocations.len(), 2);

    client.cancel().await.unwrap();
}

// ---- 015-preference-enforcement -------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_turn_flags_a_stored_preference_violation_end_to_end() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    // The single review hop confirms the violation (and no contradiction).
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "contradicts": false,
            "statement_a": "",
            "statement_b": "",
            "basis": "No contradiction among the pairs.",
            "violates": true,
            "violated_preference": "alpha rule: final messages must never contain the word delve",
            "violation_basis": "The final message contains the word delve.",
            "capture_worthy": false,
            "capture_kind": "none",
            "capture_content": "",
            "capture_basis": ""
        }))))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    // Seed the preference first-hand (kind fact, external=false ⇒ trusted).
    let saved = client
        .call_tool(tool_call(
            "save",
            &json!({ "content": "alpha rule: final messages must never contain the word delve",
                    "kind": "fact", "origin": "user", "external": false }),
        ))
        .await
        .unwrap();
    let memory_id = saved.structured_content.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let dir = tempfile::tempdir().unwrap();
    let path = write_transcript(dir.path(), "cs-pref", &[("cargo build", false)]);
    let result = client
        .call_tool(tool_call(
            "checkpoint_turn",
            &json!({
                "session_id": "cs-pref",
                "transcript_path": path,
                "final_message": "Let me delve into the details of this fix.",
                "continuation": false
            }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    // FR-003: flag, never hold — and the flag names preference + provenance.
    assert_eq!(structured["verdict"], "flag");
    let message = structured["message"].as_str().unwrap();
    assert!(
        message.contains("must never contain the word delve"),
        "{message}"
    );
    assert!(message.contains(&memory_id), "{message}");
    assert!(message.contains("first_hand provenance"), "{message}");
    assert_eq!(structured["signals"][0]["kind"], "preference_violation");

    // Exactly one audit row; enforcement evaluated and fired (SC-005).
    let checkpoints = storage.list_checkpoints().await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert!(checkpoints[0]
        .signals_evaluated
        .contains(&mcp_parallax::checkpoint::SignalKind::PreferenceViolation));
    assert!(checkpoints[0].signals_fired[0]
        .evidence
        .contains(&memory_id));
    assert!(checkpoints[0].review_ran);

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_turn_without_memory_still_lists_only_contradiction() {
    // SC-003: with memory unconfigured, the audit surface is unchanged —
    // the enforcement signal is never listed as evaluated.
    let mock = MockServer::start().await;
    let (client, storage, _server) = serve(&mock, 2_000).await;
    let dir = tempfile::tempdir().unwrap();
    let path = write_transcript(dir.path(), "cs-nomem", &[("cargo build", false)]);

    let result = client
        .call_tool(tool_call(
            "checkpoint_turn",
            &json!({
                "session_id": "cs-nomem",
                "transcript_path": path,
                "final_message": "Everything completed without surprises today.",
                "continuation": false
            }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["verdict"], "silence");

    let checkpoints = storage.list_checkpoints().await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(
        checkpoints[0].signals_evaluated,
        vec![mcp_parallax::checkpoint::SignalKind::SelfContradiction]
    );

    client.cancel().await.unwrap();
}

// ---- 016-push-memory -------------------------------------------------------

const SURFACE_CONTRACT: &str = include_str!("../specs/016-push-memory/contracts/surface.tool.json");

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn surface_matches_its_contract() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve_with_memory(&mock, ":memory:").await;

    let tools = client.list_all_tools().await.unwrap();
    let tool = tools.iter().find(|t| t.name == "surface").expect("surface");
    let contract: Value = serde_json::from_str(SURFACE_CONTRACT).unwrap();
    assert_eq!(
        tool.description.as_deref().unwrap(),
        contract["description"],
        "surface: description diverges from the contract"
    );
    let props = |schema: &Value| -> Vec<String> {
        let mut names: Vec<String> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    };
    let input = serde_json::to_value(tool.input_schema.as_ref()).unwrap();
    assert_eq!(props(&input), props(&contract["params"]));
    let output = serde_json::to_value(tool.output_schema.as_ref().expect("outputSchema")).unwrap();
    assert_eq!(props(&output), props(&contract["result"]));

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn surface_pushes_once_per_session_and_leaves_the_pull_surface_unchanged() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    // Seed one first-hand fact ("alpha" → [1,0]; surface queries embed [0.9,0.1]).
    let saved = client
        .call_tool(tool_call(
            "save",
            &json!({ "content": "alpha rule: clear the cache before staging deploys",
                    "kind": "fact", "origin": "test", "external": false }),
        ))
        .await
        .unwrap();
    let memory_id = saved.structured_content.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Turn 1: related prompt ⇒ surfaced with the full advisory label.
    let result = client
        .call_tool(tool_call(
            "surface",
            &json!({ "session_id": "ps-int", "prompt": "the staging deploy is failing again" }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["surfaced"][0]["id"], memory_id.as_str());
    assert_eq!(structured["fail_open"], false);
    let hook = &structured["hookSpecificOutput"];
    assert_eq!(hook["hookEventName"], "UserPromptSubmit");
    let context = hook["additionalContext"].as_str().unwrap();
    assert!(context.contains("advisory context"), "{context}");
    assert!(context.contains(&memory_id), "{context}");
    assert!(context.contains("first_hand"), "{context}");
    assert!(
        context.contains("alpha rule: clear the cache before staging deploys"),
        "{context}"
    );

    // Turn 2, same session ⇒ suppressed: silence, and no hook key at all.
    let result = client
        .call_tool(tool_call(
            "surface",
            &json!({ "session_id": "ps-int", "prompt": "still about the staging deploy" }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["surfaced"], json!([]));
    assert!(structured.get("hookSpecificOutput").is_none());

    // New session ⇒ suppression reset, surfaced again (FR-005).
    let result = client
        .call_tool(tool_call(
            "surface",
            &json!({ "session_id": "ps-int-2", "prompt": "staging deploy question" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.structured_content.unwrap()["surfaced"][0]["id"],
        memory_id.as_str()
    );

    // FR-009: a recall interleaved with surface calls returns the memory
    // unchanged — push reads never perturb the pull surface.
    let recalled = client
        .call_tool(tool_call("recall", &json!({ "query": "alpha rule" })))
        .await
        .unwrap();
    let memories = recalled.structured_content.unwrap()["memories"].clone();
    assert_eq!(memories[0]["id"], memory_id.as_str());
    assert_eq!(
        memories[0]["content"],
        "alpha rule: clear the cache before staging deploys"
    );

    // SC-005: one audit row per evaluation — surfaced, silent, surfaced.
    let pushes = storage.list_pushes().await.unwrap();
    assert_eq!(pushes.len(), 3);
    let surfacing: usize = pushes.iter().filter(|p| !p.surfaced_ids.is_empty()).count();
    assert_eq!(surfacing, 2);
    assert!(pushes.iter().all(|p| !p.fail_open));

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn surface_stays_silent_when_nothing_is_relevant() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    // "beta" content embeds [0,1]; the query embeds [0.9,0.1] → cosine ≈ 0.11.
    client
        .call_tool(tool_call(
            "save",
            &json!({ "content": "beta note: the marketing site uses a static generator",
                    "kind": "fact", "origin": "test", "external": false }),
        ))
        .await
        .unwrap();

    let result = client
        .call_tool(tool_call(
            "surface",
            &json!({ "session_id": "ps-quiet", "prompt": "an entirely unrelated question" }),
        ))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["surfaced"], json!([]));
    assert!(structured.get("hookSpecificOutput").is_none());
    assert_eq!(structured["fail_open"], false);

    let pushes = storage.list_pushes().await.unwrap();
    assert_eq!(pushes.len(), 1);
    assert!(pushes[0].surfaced_ids.is_empty());

    client.cancel().await.unwrap();
}

// ---- 017-memory-consolidation ----------------------------------------------

async fn mount_consolidation(mock: &MockServer, relation: &str, basis: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "relation": relation, "basis": basis
        }))))
        .mount(mock)
        .await;
}

async fn save_fact(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    content: &str,
) -> String {
    let saved = client
        .call_tool(tool_call(
            "save",
            &json!({ "content": content, "kind": "fact", "origin": "test", "external": false }),
        ))
        .await
        .unwrap();
    saved.structured_content.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_supersedes_and_retrieval_returns_only_the_update() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    mount_consolidation(&mock, "updates", "The pipeline moved providers.").await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    let old_id = save_fact(&client, "alpha fact: the pipeline runs on the old provider").await;
    let new_id = save_fact(
        &client,
        "alpha fact: the pipeline moved to the new provider",
    )
    .await;

    // Retrieval returns only the update (FR-001/FR-011).
    let recalled = client
        .call_tool(tool_call("recall", &json!({ "query": "alpha pipeline" })))
        .await
        .unwrap();
    let memories = recalled.structured_content.unwrap()["memories"].clone();
    assert_eq!(memories.as_array().unwrap().len(), 1);
    assert_eq!(memories[0]["id"], new_id.as_str());

    // The superseded original is present, attributed, byte-identical (US4).
    let all = storage.load_memories().await.unwrap();
    assert_eq!(all.len(), 2);
    let old = all.iter().find(|m| m.id == old_id).unwrap();
    assert_eq!(old.status, mcp_parallax::memory::Status::Superseded);
    assert_eq!(old.replaced_by.as_deref(), Some(new_id.as_str()));
    assert_eq!(
        old.content,
        "alpha fact: the pipeline runs on the old provider"
    );

    // Exactly one supersede audit row (FR-009).
    let audit = storage.list_consolidations().await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(
        audit[0].action,
        mcp_parallax::memory::consolidate::ConsolidationAction::Supersede
    );

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_context_specific_statement_leaves_the_standing_fact_active() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    mount_consolidation(&mock, "context_specific", "A this-week circumstance.").await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    save_fact(&client, "alpha fact: the team is based in Lisbon").await;
    save_fact(
        &client,
        "alpha note: working from the Berlin office this week",
    )
    .await;

    // Both stay active (FR-002 - the Berlin/Lisbon rule); no audit row.
    let all = storage.load_memories().await.unwrap();
    assert!(all
        .iter()
        .all(|m| m.status == mcp_parallax::memory::Status::Active));
    assert!(storage.list_consolidations().await.unwrap().is_empty());

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_near_duplicate_merges_to_a_byte_identical_canonical() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    mount_consolidation(&mock, "same_assertion", "Same knowledge, reworded.").await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    let first = save_fact(
        &client,
        "alpha rule: clear the cache before staging deploys",
    )
    .await;
    let second = save_fact(
        &client,
        "alpha rule: the cache must be cleared ahead of any staging deploy",
    )
    .await;

    let all = storage.load_memories().await.unwrap();
    let merged = all.iter().find(|m| m.id == first).unwrap();
    assert_eq!(merged.status, mcp_parallax::memory::Status::Merged);
    assert_eq!(merged.replaced_by.as_deref(), Some(second.as_str()));
    // Survivor content byte-identical to the admission (FR-004).
    let canonical = all.iter().find(|m| m.id == second).unwrap();
    assert_eq!(
        canonical.content,
        "alpha rule: the cache must be cleared ahead of any staging deploy"
    );

    let recalled = client
        .call_tool(tool_call("recall", &json!({ "query": "alpha cache" })))
        .await
        .unwrap();
    let memories = recalled.structured_content.unwrap()["memories"].clone();
    assert_eq!(memories.as_array().unwrap().len(), 1);
    assert_eq!(memories[0]["id"], second.as_str());

    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_of_turn_capture_stores_a_quarantined_candidate() {
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await;
    // The turn hop proposes a lesson; no contradiction, no violation.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "contradicts": false,
            "statement_a": "",
            "statement_b": "",
            "basis": "Nothing to compare.",
            "violates": false,
            "violated_preference": "",
            "violation_basis": "",
            "capture_worthy": true,
            "capture_kind": "lesson",
            "capture_content": "the staging deploy fails unless the cache is cleared first",
            "capture_basis": "The turn diagnosed and fixed exactly this failure."
        }))))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_memory(&mock, ":memory:").await;

    let dir = tempfile::tempdir().unwrap();
    let transcript = write_transcript(dir.path(), "cs-capture", &[("cargo test", false)]);
    let result = client
        .call_tool(tool_call(
            "checkpoint_turn",
            &json!({
                "session_id": "cs-capture",
                "transcript_path": transcript,
                "final_message": "Fixed: the cache had to be cleared before the deploy.",
                "continuation": false
            }),
        ))
        .await
        .unwrap();
    // Capture never affects the verdict (FR-008).
    assert_eq!(
        result.structured_content.as_ref().unwrap()["verdict"],
        "silence"
    );

    // The candidate is stored quarantined with its origin (FR-007).
    let all = storage.load_memories().await.unwrap();
    assert_eq!(all.len(), 1);
    let candidate = &all[0];
    assert_eq!(candidate.trust, mcp_parallax::memory::Trust::Untrusted);
    assert!(candidate.external);
    assert!(candidate
        .origin
        .contains("auto-capture: session cs-capture"));
    assert_eq!(
        candidate.content,
        "the staging deploy fails unless the cache is cleared first"
    );

    // Quarantine: push never surfaces it, even on a related prompt.
    let surfaced = client
        .call_tool(tool_call(
            "surface",
            &json!({ "session_id": "cs-capture", "prompt": "the staging deploy is failing" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        surfaced.structured_content.as_ref().unwrap()["surfaced"],
        json!([])
    );

    // Recall labels it untrusted; the audit row names the session.
    let recalled = client
        .call_tool(tool_call(
            "recall",
            &json!({ "query": "staging deploy cache" }),
        ))
        .await
        .unwrap();
    let memories = recalled.structured_content.unwrap()["memories"].clone();
    assert_eq!(memories[0]["trust"], "untrusted");
    let audit = storage.list_consolidations().await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(
        audit[0].action,
        mcp_parallax::memory::consolidate::ConsolidationAction::CaptureProposed
    );
    assert_eq!(audit[0].session_id.as_deref(), Some("cs-capture"));

    // The candidate is deletable like any memory.
    let forgotten = client
        .call_tool(tool_call(
            "forget",
            &json!({ "id": memories[0]["id"].as_str().unwrap() }),
        ))
        .await
        .unwrap();
    assert_eq!(forgotten.structured_content.unwrap()["forgotten"], true);

    client.cancel().await.unwrap();
}

// ---- 007-observability-layer ---------------------------------------------

/// T005 + T007: spans and metrics derived from the records, end to end
/// through the real server (in-memory exporters injected through the SDK's
/// own exporter abstraction). One process-global telemetry init — this is
/// the single enabled-path test; every other test's emissions land in the
/// same in-memory sink harmlessly, so assertions filter by this test's
/// session id. (Strict telemetry==records rate equality runs in the
/// acceptance example, which owns a clean process.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one process-global telemetry init = one comprehensive test
async fn telemetry_spans_match_the_stored_records() {
    use mcp_parallax::observability;
    use opentelemetry_sdk::metrics::in_memory_exporter::InMemoryMetricExporter;
    use opentelemetry_sdk::trace::InMemorySpanExporter;

    let span_exporter = InMemorySpanExporter::default();
    let metric_exporter = InMemoryMetricExporter::default();
    let guard = observability::init_with_exporters(
        span_exporter.clone(),
        metric_exporter.clone(),
        "itest-instance",
    );

    // A successful verify (3-pass ensemble via the wiremock Anthropic).
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "verdict": "supported", "findings": []
        }))))
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;
    client
        .call_tool(tool_call("verify", &json!({ "claim": "telemetry twin" })))
        .await
        .unwrap();

    // A checkpoint flag (seeded loop transcript).
    let dir = tempfile::tempdir().unwrap();
    let transcript = write_transcript(
        dir.path(),
        "otlp-loop",
        &[
            ("cargo test", true),
            ("cargo test", true),
            ("cargo test", true),
            ("cargo test", true),
        ],
    );
    client
        .call_tool(tool_call(
            "checkpoint_batch",
            &json!({ "session_id": "otlp-loop", "transcript_path": transcript }),
        ))
        .await
        .unwrap();

    guard.flush();

    // --- invocation span == stored record, field for field (SC-001) ---
    let invocation = storage
        .list_invocations()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.tool == "verify")
        .expect("verify record stored");
    let spans = span_exporter.get_finished_spans().unwrap();
    let attr = |attrs: &[opentelemetry::KeyValue], key: &str| -> Option<String> {
        attrs
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| kv.value.to_string())
    };
    let span = spans
        .iter()
        .find(|s| {
            s.name == "parallax.verify"
                && attr(&s.attributes, "parallax.session_id").as_deref()
                    == Some(invocation.session_id.as_str())
        })
        .expect("invocation span exported");
    assert_eq!(
        attr(&span.attributes, "gen_ai.request.model").as_deref(),
        Some(invocation.model.as_str())
    );
    assert_eq!(
        attr(&span.attributes, "gen_ai.usage.input_tokens").as_deref(),
        Some(invocation.input_tokens.to_string().as_str())
    );
    assert_eq!(
        attr(&span.attributes, "gen_ai.usage.output_tokens").as_deref(),
        Some(invocation.output_tokens.to_string().as_str())
    );
    assert_eq!(
        attr(&span.attributes, "parallax.outcome").as_deref(),
        Some("success")
    );
    assert_eq!(
        attr(&span.attributes, "parallax.cost_usd").as_deref(),
        Some(invocation.cost_usd.to_string().as_str())
    );
    // Retroactive timing equals the record's window.
    let end: std::time::SystemTime = invocation.created_at.into();
    assert_eq!(span.end_time, end);
    assert_eq!(
        span.start_time,
        end - std::time::Duration::from_millis(invocation.latency_ms)
    );

    // --- checkpoint span == stored audit row (FR-008: kinds, no evidence) ---
    let checkpoint = storage
        .list_checkpoints()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.session_id == "otlp-loop")
        .expect("checkpoint row stored");
    assert_eq!(checkpoint.verdict.as_str(), "flag");
    let cp_span = spans
        .iter()
        .find(|s| {
            s.name == "parallax.checkpoint.batch"
                && attr(&s.attributes, "parallax.session_id").as_deref() == Some("otlp-loop")
        })
        .expect("checkpoint span exported");
    assert_eq!(
        attr(&cp_span.attributes, "parallax.checkpoint.verdict").as_deref(),
        Some("flag")
    );
    for kv in &cp_span.attributes {
        assert!(
            !kv.value.to_string().contains("invoked 4 times"),
            "evidence leaked into checkpoint span attribute {}",
            kv.key
        );
    }

    // --- metric instruments present with our scenario's series ---
    let metrics = metric_exporter.get_finished_metrics().unwrap();
    let names: Vec<String> = metrics
        .iter()
        .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
        .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
        .map(|m| m.name().to_string())
        .collect();
    for expected in [
        "parallax.invocations",
        "parallax.invocation.duration",
        "parallax.cost",
        "gen_ai.client.token.usage",
        "parallax.checkpoint.evaluations",
        "parallax.checkpoint.duration",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "missing metric {expected}: {names:?}"
        );
    }

    // --- the cancelled exit emits too, not just the success path ---
    // An abandoned invocation records `cancelled` and must mirror to telemetry
    // with that outcome, twinning its SQLite row. Drive a real abandoned
    // verify (client torn down mid-flight) and assert the twin span. Guards
    // the cancellation exit against losing its telemetry pairing.
    let slow = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(end_turn(&json!({ "verdict": "supported", "findings": [] })))
                .set_delay(Duration::from_mins(1)),
        )
        .mount(&slow)
        .await;
    let (slow_client, slow_storage, slow_server) = serve(&slow, 120_000).await;
    let call_task = tokio::spawn(async move {
        slow_client
            .call_tool(tool_call("verify", &json!({ "claim": "abandoned twin" })))
            .await
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    call_task.abort();
    let _ = call_task.await;
    let _ = slow_server.cancel().await;

    // publish() fires synchronously at the exit, before the row is persisted —
    // so once the cancelled row is visible, the span is already queued.
    let cancelled = {
        let mut found = None;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Some(r) = slow_storage
                .list_invocations()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.outcome == Outcome::Cancelled)
            {
                found = Some(r);
                break;
            }
        }
        found.expect("abandoned invocation recorded cancelled")
    };

    guard.flush();
    let spans = span_exporter.get_finished_spans().unwrap();
    let cancelled_span = spans
        .iter()
        .find(|s| {
            s.name == "parallax.verify"
                && attr(&s.attributes, "parallax.session_id").as_deref()
                    == Some(cancelled.session_id.as_str())
        })
        .expect("cancelled invocation span exported — the exit must emit");
    assert_eq!(
        attr(&cancelled_span.attributes, "parallax.outcome").as_deref(),
        Some("cancelled")
    );
    // Retroactive timing twins the record, same as the finish() path.
    let cend: std::time::SystemTime = cancelled.created_at.into();
    assert_eq!(cancelled_span.end_time, cend);

    client.cancel().await.unwrap();
}

/// T008(a)(b)(c): the spawn-the-binary smoke test with telemetry enabled
/// against an UNREACHABLE collector — env-driven init for real, identical
/// protocol behavior, stdout carries only protocol frames, and the process
/// exits within the bounded flush window instead of hanging (FR-006/007/010,
/// SC-004's session-level half).
#[test]
#[allow(clippy::panic)] // deadline violation IS the test failure
fn stdio_smoke_with_unreachable_collector_stays_clean_and_exits_bounded() {
    use std::io::{BufRead, BufReader, Write};

    let db = std::env::temp_dir().join(format!("parallax-otlp-smoke-{}.db", uuid::Uuid::new_v4()));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_mcp-parallax"))
        .env("ANTHROPIC_API_KEY", "dummy-key-no-model-call-happens")
        .env_remove("VOYAGE_API_KEY")
        .env_remove("BRAVE_API_KEY")
        // Telemetry ON, collector unreachable (nothing listens on port 9).
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:9")
        .env("DATABASE_PATH", db.to_string_lossy().to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("binary spawns");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"smoke","version":"0"}}}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    stdout.read_line(&mut line).unwrap();
    let init: Value = serde_json::from_str(&line).expect("first stdout line is JSON-RPC");
    assert_eq!(init["jsonrpc"], "2.0");

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
    // Behavior identical to a telemetry-disabled run: full default catalog.
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 9);

    // Close stdin -> transport EOF -> graceful shutdown path runs the
    // telemetry flush against the dead collector. The process must exit
    // within the bounded window, not hang (FR-010).
    drop(stdin);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        match child.try_wait().expect("wait") {
            Some(status) => {
                assert!(status.success(), "clean exit, got {status:?}");
                break;
            }
            None if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                panic!("process hung past the bounded flush window with a dead collector");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    let _ = std::fs::remove_file(&db);
}

// ---- 008-grounded-verify --------------------------------------------------

/// Build the server with grounded-verify enabled against a real source root.
async fn serve_with_grounded(
    mock: &MockServer,
    root: &str,
) -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    Arc<SqliteStorage>,
    rmcp::service::RunningService<rmcp::service::RoleServer, Parallax>,
) {
    let mut config = test_config(2_000);
    config.grounded_verify_root = Some(root.to_string());
    let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
    let anthropic =
        Arc::new(AnthropicClient::with_base_url(&config, &mock.uri()).with_backoff_base_ms(1));
    let server = Parallax::new(anthropic, storage.clone(), Arc::new(SystemClock), &config).unwrap();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move { server.serve(server_io).await });
    let client = ().serve(client_io).await.expect("client init");
    let running_server = server_task.await.expect("join").expect("server init");
    (client, storage, running_server)
}

fn grounded_call(claim: &str, locators: Value) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new("grounded_verify");
    params.arguments = json!({ "claim": claim, "locators": locators })
        .as_object()
        .cloned();
    params
}

fn grounded_pass(verdict: &str, findings: Value, missing: Value) -> Value {
    // Ordinary judgment pass: the abstain flag is off (010).
    json!({ "verdict": verdict, "findings": findings, "missing_evidence": missing, "needs_computation": false })
}

/// A pass that self-reports the decisive fact is a computation it cannot perform
/// by reading (the abstain trigger — 010 FR-005/FR-006).
fn grounded_pass_computes(verdict: &str, findings: Value, missing: Value) -> Value {
    json!({ "verdict": verdict, "findings": findings, "missing_evidence": missing, "needs_computation": true })
}

async fn tool_names(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
) -> Vec<String> {
    client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

// US1 + SC-005: gating — absent root ⇒ tool absent; present ⇒ tool listed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_source_root_the_catalog_has_no_grounded_verify() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve(&mock, 2_000).await;
    assert!(!tool_names(&client)
        .await
        .contains(&"grounded_verify".to_string()));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_a_source_root_grounded_verify_joins_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockServer::start().await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;
    assert!(tool_names(&client)
        .await
        .contains(&"grounded_verify".to_string()));
    client.cancel().await.unwrap();
}

// US1/US2 + SC-001/SC-002 + FR-007/M2: the verbatim source reaches the pass
// (the mock matches only on the file's contents being in the request body), the
// verdict is returned faithfully, the manifest mirrors the locator, the
// completeness signal is surfaced, and exactly one record is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_judges_verbatim_source_with_manifest_and_one_record() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("pub.rs"),
        "pub fn publish(&self) { self.emit(); telemetry(); }\n",
    )
    .unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        // Only matches if the verbatim source reached the model — proves the
        // evidence (not caller prose) is what the pass judges.
        .and(body_string_contains("self.emit()"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&grounded_pass(
                "supported",
                json!([]),
                json!(["the definition of telemetry()"]),
            ))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let result = client
        .call_tool(grounded_call(
            "publish emits the tracing event",
            json!([{ "path": "pub.rs", "start_line": 1, "end_line": 1 }]),
        ))
        .await
        .unwrap();
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    assert_eq!(structured["verdict"], "supported");
    assert_eq!(structured["manifest"][0]["path"], "pub.rs");
    assert_eq!(structured["manifest"][0]["start_line"], 1);
    assert!(structured["manifest"][0]["bytes"].as_u64().unwrap() > 0);
    assert_eq!(
        structured["missing_evidence"],
        json!(["the definition of telemetry()"])
    );

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "grounded_verify");
    assert_eq!(records[0].outcome, Outcome::Success);

    client.cancel().await.unwrap();
}

// US1 + FR-009: an unresolvable locator aborts the whole call, named, with no
// verdict — and no model call happens (no mounts).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_unresolvable_locator_errors_named_with_no_verdict() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("present.rs"), "fn x() {}\n").unwrap();
    let mock = MockServer::start().await;
    let (client, storage, _server) = serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let err = client
        .call_tool(grounded_call(
            "claim",
            json!([{ "path": "present.rs" }, { "path": "gone.rs" }]),
        ))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("source not found: gone.rs"),
        "{err}"
    );

    // The aborted call still leaves exactly one record (invalid_input).
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::InvalidInput);

    client.cancel().await.unwrap();
}

// L1: a glob metacharacter is not interpreted (globs deferred) — the literal
// path simply does not resolve, and the error names it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_rejects_a_glob_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn x() {}\n").unwrap();
    let mock = MockServer::start().await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let err = client
        .call_tool(grounded_call("claim", json!([{ "path": "*.rs" }])))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("*.rs"), "{err}");

    client.cancel().await.unwrap();
}

// US2 (manifest fidelity over a mixed locator set, incl. a line range) + US3
// (the completeness signal is empty when the model reports nothing missing).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_manifest_covers_each_locator_and_completeness_can_be_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "line1\nline2\nline3\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "only\n").unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&grounded_pass(
                "supported",
                json!([]),
                json!([]),
            ))),
        )
        .mount(&mock)
        .await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let result = client
        .call_tool(grounded_call(
            "c",
            json!([
                { "path": "a.rs", "start_line": 2, "end_line": 3 },
                { "path": "b.rs" }
            ]),
        ))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    assert_eq!(s["manifest"].as_array().unwrap().len(), 2);
    assert_eq!(s["manifest"][0]["path"], "a.rs");
    assert_eq!(s["manifest"][0]["start_line"], 2);
    assert_eq!(s["manifest"][0]["end_line"], 3);
    assert_eq!(s["manifest"][1]["path"], "b.rs");
    assert!(s["manifest"][1]["start_line"].is_null());
    assert_eq!(s["missing_evidence"], json!([]));

    client.cancel().await.unwrap();
}

// ---- 009-glob-locators ----------------------------------------------------

// US1 + SC-001/002: a glob expands, server-side, to its sorted matching files,
// each a manifest entry; non-matching files are excluded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_glob_expands_to_the_matching_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.path().join("src/b.rs"), "fn b() {}\n").unwrap();
    std::fs::write(dir.path().join("src/notes.txt"), "ignore\n").unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&grounded_pass(
                "supported",
                json!([]),
                json!([]),
            ))),
        )
        .mount(&mock)
        .await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let result = client
        .call_tool(grounded_call(
            "the src files compile",
            json!([{ "glob": "src/*.rs" }]),
        ))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");
    assert_eq!(s["verdict"], "supported");
    let paths: Vec<&str> = s["manifest"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["src/a.rs", "src/b.rs"]); // sorted; .txt excluded

    client.cancel().await.unwrap();
}

// US2 + SC-004: a zero-match glob is a loud named error, no verdict.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_zero_match_glob_errors_named() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "x\n").unwrap();
    let mock = MockServer::start().await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;
    let err = client
        .call_tool(grounded_call("c", json!([{ "glob": "nope/*.rs" }])))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("matched no files"), "{err}");
    client.cancel().await.unwrap();
}

// US3 + FR-007: a glob carrying a line range is rejected, named.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_glob_with_a_line_range_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "x\n").unwrap();
    let mock = MockServer::start().await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;
    let err = client
        .call_tool(grounded_call(
            "c",
            json!([{ "glob": "*.rs", "start_line": 1, "end_line": 2 }]),
        ))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("a line range is not allowed with a glob"),
        "{err}"
    );
    client.cancel().await.unwrap();
}

// 010 US2 + FR-008 / SC-003: the dogfooded reproduction — a computable claim
// (a line count) whose passes self-report `needs_computation` returns the
// server-assembled `inconclusive` verdict routed to `check`, NEVER a confident
// refutation it did not compute.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_computable_claim_is_inconclusive_not_confidently_refuted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("server.rs"), "line\n".repeat(1224)).unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        // The passes estimate (~850) and self-report that an exact count is the
        // decisive fact they cannot compute by reading.
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&grounded_pass_computes(
                "refuted",
                json!(["estimated about 850 lines, fewer than 1000"]),
                json!(["an exact line count, e.g. wc -l"]),
            ))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let result = client
        .call_tool(grounded_call(
            "src/server.rs is over 1000 lines",
            json!([{ "path": "server.rs" }]),
        ))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    // Abstains and routes — never the confident refutation the bug produced.
    assert_eq!(s["verdict"], "inconclusive");
    assert_ne!(s["verdict"], "refuted");
    assert!(s["reason"].as_str().unwrap().contains("check"));

    // Still exactly one invocation record, recorded as success.
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "grounded_verify");
    assert_eq!(records[0].outcome, Outcome::Success);

    client.cancel().await.unwrap();
}

// 010 US2 + FR-007: the judgment path is unchanged — a pass that does NOT set
// `needs_computation` returns its confident verdict even while listing advisory
// missing evidence (no over-abstention).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_judgment_claim_keeps_its_confident_verdict() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("pub.rs"),
        "pub fn publish(&self) { self.emit(); }\n",
    )
    .unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&grounded_pass(
                "supported",
                json!([]),
                json!(["the definition of emit()"]),
            ))),
        )
        .mount(&mock)
        .await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let result = client
        .call_tool(grounded_call(
            "publish calls emit",
            json!([{ "path": "pub.rs" }]),
        ))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    // Confident verdict stands; advisory missing_evidence is surfaced, not abstained.
    assert_eq!(s["verdict"], "supported");
    assert!(s.get("reason").is_none() || s["reason"].is_null());
    assert_eq!(s["missing_evidence"], json!(["the definition of emit()"]));

    client.cancel().await.unwrap();
}

// 011 — a computable pass body: needs_computation set + the compute fields.
fn grounded_compute_pass(property: &str, op: &str, threshold: i64, literal: Value) -> Value {
    json!({
        "verdict": "supported", "findings": [], "missing_evidence": [], "needs_computation": true,
        "compute_property": property, "compute_match_literal": literal,
        "compute_operator": op, "compute_threshold": threshold,
    })
}

// 011 US1 / SC-001/SC-002/SC-005: a computable claim is *settled* over the real
// file's line count via the engine — supported with the executed form, not the
// 010 inconclusive abstain. The server counts the actual 1224-line file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_computable_claim_is_settled_supported_with_executed_form() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("server.rs"), "line\n".repeat(1224)).unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&grounded_compute_pass(
                "lines",
                ">",
                1000,
                Value::Null,
            ))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let s = client
        .call_tool(grounded_call(
            "src/server.rs is over 1000 lines",
            json!([{ "path": "server.rs" }]),
        ))
        .await
        .unwrap();
    let s = s.structured_content.as_ref().expect("structured_content");

    assert_eq!(s["verdict"], "supported");
    assert_eq!(s["executed_form"], "1224 > 1000");
    assert_eq!(s["engine_result"], "true");
    assert_eq!(s["findings"], json!(["counted 1224 lines"]));
    assert_ne!(s["verdict"], "inconclusive"); // the 010 behavior is superseded

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::Success);
    client.cancel().await.unwrap();
}

// 011 US1 / SC-002: the engine decides direction — a false comparison refutes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_computable_false_comparison_settles_refuted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("server.rs"), "line\n".repeat(1224)).unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&grounded_compute_pass(
                "lines",
                ">",
                5000,
                Value::Null,
            ))),
        )
        .mount(&mock)
        .await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let s = client
        .call_tool(grounded_call(
            "src/server.rs is over 5000 lines",
            json!([{ "path": "server.rs" }]),
        ))
        .await
        .unwrap();
    let s = s.structured_content.as_ref().expect("structured_content");
    assert_eq!(s["verdict"], "refuted");
    assert_eq!(s["executed_form"], "1224 > 5000");
    assert_eq!(s["engine_result"], "false");
    client.cancel().await.unwrap();
}

// 011 US2 / SC-003: a multi-locator computable claim is not single-source, so it
// abstains (inconclusive) — never a computed verdict over an aggregate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_multi_source_computable_abstains() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "line\n".repeat(600)).unwrap();
    std::fs::write(dir.path().join("b.rs"), "line\n".repeat(700)).unwrap();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&grounded_compute_pass(
                "lines",
                ">",
                1000,
                Value::Null,
            ))),
        )
        .mount(&mock)
        .await;
    let (client, _storage, _server) =
        serve_with_grounded(&mock, dir.path().to_str().unwrap()).await;

    let s = client
        .call_tool(grounded_call(
            "these files total over 1000 lines",
            json!([{ "path": "a.rs" }, { "path": "b.rs" }]),
        ))
        .await
        .unwrap();
    let s = s.structured_content.as_ref().expect("structured_content");
    assert_eq!(s["verdict"], "inconclusive");
    assert!(s.get("executed_form").is_none() || s["executed_form"].is_null());
    client.cancel().await.unwrap();
}

// ---- 012-diverge ----------------------------------------------------------

fn diverge_call(problem: &str, context: Option<&str>) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new("diverge");
    let args = context.map_or_else(
        || json!({ "problem": problem }),
        |c| json!({ "problem": problem, "context": c }),
    );
    params.arguments = args.as_object().cloned();
    params
}

fn diverge_pass(framing: &str, implication: &str) -> Value {
    json!({ "framing": framing, "implication": implication })
}

// 012 US1 + FR-009 / SC-002 / FR-007: diverge is always in the catalog; distinct
// per-lens framings come back lens-labeled and deduplicated, with no verdict or
// confidence field, and exactly one record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diverge_returns_distinct_lens_labeled_framings_and_one_record() {
    let mock = MockServer::start().await;
    // A distinct framing per lens, matched on the lens directive in the prompt body
    // (k=3 → invert / actor / horizon).
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("Flip the goal"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&diverge_pass(
                "What if more steps is the fix, each one earning trust?",
                "Reframes the goal from brevity to confidence.",
            ))),
        )
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("Change whose problem"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&diverge_pass(
                "Is this the user's problem, or the team's metric?",
                "If users aren't dropping off, step count is a vanity concern.",
            ))),
        )
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("Shift the time scale"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&diverge_pass(
                "At a one-year horizon, is onboarding length even the lever?",
                "The durable lever may be activation, not first-session speed.",
            ))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(diverge_call("We need to cut steps from onboarding.", None))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    // Always in the catalog.
    assert!(tool_names(&client).await.contains(&"diverge".to_string()));
    // Three distinct framings, each lens-labeled; no verdict/confidence.
    let perspectives = s["perspectives"].as_array().unwrap();
    assert_eq!(perspectives.len(), 3);
    let lenses: Vec<&str> = perspectives
        .iter()
        .map(|p| p["lens"].as_str().unwrap())
        .collect();
    assert_eq!(lenses, vec!["invert", "actor", "horizon"]);
    for p in perspectives {
        assert!(!p["framing"].as_str().unwrap().is_empty());
        assert!(!p["implication"].as_str().unwrap().is_empty());
    }
    assert_eq!(s["passes"], 3);
    assert!(s.get("verdict").is_none());
    assert!(s.get("confidence").is_none());

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "diverge");
    assert_eq!(records[0].outcome, Outcome::Success);

    client.cancel().await.unwrap();
}

// 012 US1 / FR-004: identical framings across passes collapse to one (the server
// deduplicates deterministically), keeping the earliest lens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diverge_deduplicates_identical_framings() {
    let mock = MockServer::start().await;
    // Every pass returns the same framing → collapses to one perspective.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&diverge_pass(
                "This is really a naming problem, not a flow problem.",
                "Renaming the steps may resolve the felt friction.",
            ))),
        )
        .mount(&mock)
        .await;
    let (client, _storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(diverge_call("Cut onboarding steps.", None))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");
    let perspectives = s["perspectives"].as_array().unwrap();
    assert_eq!(perspectives.len(), 1); // 3 identical → deduped to 1
    assert_eq!(perspectives[0]["lens"], "invert"); // earliest kept
    assert_eq!(s["passes"], 3); // all 3 completed; dedup is post-collection
    client.cancel().await.unwrap();
}

// 012 US2 / FR-005: a stated preference in `context` does not break the tool — it
// reaches a pass only as context (no extra stance slot); perspectives still come
// back and one record is written. (The "does not narrow" property is the live dogfood.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diverge_accepts_a_stance_in_context_and_stays_blind() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&diverge_pass(
                "What if the rewrite is the risk, not the fix?",
                "A rewrite resets hard-won edge-case knowledge.",
            ))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(diverge_call(
            "Our service is hard to maintain.",
            Some("I think we should just rewrite it."),
        ))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");
    assert!(!s["perspectives"].as_array().unwrap().is_empty());
    assert!(s.get("verdict").is_none());

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "diverge");
    client.cancel().await.unwrap();
}

// ---- 013-decide -----------------------------------------------------------

fn decide_call(decision: &str, options: Value) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new("decide");
    params.arguments = json!({ "decision": decision, "options": options })
        .as_object()
        .cloned();
    params
}

fn decide_assessment(methodology: &str, scores: Value, rationales: Value, factors: Value) -> Value {
    json!({
        "methodology": methodology,
        "option_scores": scores,
        "option_rationales": rationales,
        "deciding_factors": factors,
    })
}

// 013 US1 + FR-002/004/005/007 + SC-001/003: a scored single pass yields a
// server-derived recommendation with margin-calibrated confidence, the full
// breakdown, the surfaced methodology, no verdict/next_step, and one record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decide_returns_calibrated_recommendation_and_one_record() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&decide_assessment(
                "weigh",
                json!([85, 40]),
                json!([
                    "safe and reversible at each step",
                    "fast but no incremental rollback"
                ]),
                json!(["blast radius", "rollback speed"]),
            ))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(decide_call(
            "How should we ship the migration?",
            json!(["feature-flag ramp", "big-bang cutover"]),
        ))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    assert!(tool_names(&client).await.contains(&"decide".to_string()));
    assert_eq!(s["recommended"], "feature-flag ramp");
    assert_eq!(s["runner_up"], "big-bang cutover");
    assert!(s["runner_up_reason"].as_str().unwrap().contains("45 below"));
    assert!((s["confidence"].as_f64().unwrap() - 0.725).abs() < 1e-9); // margin 45
    assert_eq!(s["methodology"], "weigh");
    assert_eq!(
        s["deciding_factors"],
        json!(["blast radius", "rollback speed"])
    );
    assert_eq!(s["assessments"].as_array().unwrap().len(), 2);
    assert!(s.get("verdict").is_none());
    assert!(s.get("next_step").is_none());

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "decide");
    assert_eq!(records[0].outcome, Outcome::Success);
    assert_eq!(records[0].input_tokens, 100); // single pass, not x3
    client.cancel().await.unwrap();
}

// 013 SC-005 / FR-008: fewer than two options is rejected, no model call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decide_rejects_fewer_than_two_options() {
    let mock = MockServer::start().await;
    let (client, _storage, _server) = serve(&mock, 2_000).await;
    let err = client
        .call_tool(decide_call("pick", json!(["only one"])))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("at least two options"), "{err}");
    client.cancel().await.unwrap();
}

// ---- 014-elicit -----------------------------------------------------------

fn elicit_call(task: &str) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new("elicit");
    params.arguments = json!({ "task": task }).as_object().cloned();
    params
}

fn elicit_inference(objective: &str, prefs: Value, signals: Value, strengths: Value) -> Value {
    json!({
        "assumed_objective": objective,
        "preference_texts": prefs,
        "preference_signals": signals,
        "preference_strengths": strengths,
        "divergence_questions": [],
        "divergence_signals": [],
        "signal_level": "medium",
    })
}

// 014 US1 + FR-006/007/SC-005: without memory, elicit surfaces the objective and
// traced preferences, reports memory_consulted=false, carries no enforcement
// field, and writes exactly one record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elicit_surfaces_objective_without_memory_and_one_record() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&elicit_inference(
                "Add a cache to speed up the report endpoint",
                json!(["p99 latency, not average, is the target"]),
                json!(["the request mentions tail latency"]),
                json!(["stated"]),
            ))),
        )
        .mount(&mock)
        .await;
    let (client, storage, _server) = serve(&mock, 2_000).await;

    let result = client
        .call_tool(elicit_call("Speed up the report endpoint"))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");

    assert!(tool_names(&client).await.contains(&"elicit".to_string()));
    assert_eq!(
        s["assumed_objective"],
        "Add a cache to speed up the report endpoint"
    );
    assert_eq!(s["governing_preferences"][0]["strength"], "stated");
    assert_eq!(s["memory_consulted"], false);
    assert!(s.get("verdict").is_none());
    assert!(s.get("hold").is_none() && s.get("action").is_none()); // surface only

    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "elicit");
    assert_eq!(records[0].outcome, Outcome::Success);
    client.cancel().await.unwrap();
}

// 014 US1 + FR-003/SC-004 (mechanism): with memory configured, the server recalls
// a seeded TRUSTED preference and it reaches the inference prompt — the /v1/messages
// matcher only fires when the recalled content is in the request body. memory_consulted
// is true. This is the structural consultation guarantee (output-marking is the live T012).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elicit_recalls_a_trusted_preference_into_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let mock = MockServer::start().await;
    mount_embeddings(&mock).await; // "alpha" docs/query rank together
                                   // The elicit inference response is served ONLY if the recalled memory content
                                   // ("avoids adding new services") reached the prompt — proving the recall landed.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("avoids adding new services"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(end_turn(&elicit_inference(
                "Add a caching service",
                json!(["minimal new infrastructure"]),
                json!(["stored memory"]),
                json!(["revealed"]),
            ))),
        )
        .mount(&mock)
        .await;
    let (client, _storage, _server) = serve_with_memory(&mock, db.to_str().unwrap()).await;

    // Seed a trusted (first-hand) preference memory; "alpha" routes the embedding.
    client
        .call_tool(tool_call(
            "save",
            &json!({
                "content": "alpha: the user avoids adding new services",
                "kind": "lesson",
                "origin": "observed in past work",
                "external": false
            }),
        ))
        .await
        .unwrap();

    let result = client
        .call_tool(elicit_call("add a caching service to speed up reports"))
        .await
        .unwrap();
    let s = result
        .structured_content
        .as_ref()
        .expect("structured_content");
    assert_eq!(s["memory_consulted"], true);
    assert_eq!(s["assumed_objective"], "Add a caching service");
    client.cancel().await.unwrap();
}
