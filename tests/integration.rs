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
        input_max_chars: 50_000,
        voyage_api_key: None,
        voyage_model: "voyage-4".into(),
        memory_recall_limit: 5,
        brave_api_key: None,
        fetch_timeout_ms: 10_000,
        research_concurrency: 8,
        fetch_allow_private: false,
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
    assert_eq!(names, vec!["check", "unstick", "verify"]);

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
    assert_eq!(names, ["check", "unstick", "verify"]);

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
        ["check", "forget", "recall", "save", "unstick", "verify"]
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
    assert_eq!(names, ["check", "research", "unstick", "verify"]);

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
