//! Helpers shared by the integration test binaries: the test config, the
//! server builders over a duplex transport, and the wiremock response and
//! fixture builders. Each `tests/*.rs` file is its own process and compiles
//! this module separately, so a helper one binary does not use is not dead.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::{AnthropicClient, BraveClient, VoyageClient};
use mcp_parallax::config::Config;
use mcp_parallax::server::Parallax;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::clock::SystemClock;
use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

pub fn test_config(timeout_ms: u64) -> Config {
    Config {
        backend: mcp_parallax::config::Backend::Anthropic,
        anthropic_api_key: "test-key".into(),
        openai_api_key: String::new(),
        openai_api_base: "http://127.0.0.1:1".into(),
        openai_model: String::new(),
        openai_structured_output: mcp_parallax::config::StructuredOutput::Auto,
        anthropic_model: "claude-opus-4-8".into(),
        routing: mcp_parallax::routing::RoutingTable::single(
            "claude-opus-4-8",
            mcp_parallax::routing::DefaultVar::AnthropicModel,
        ),
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
        anthropic_api_base: "http://127.0.0.1:1".into(),
    }
}

pub fn end_turn(value: &Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 100, "output_tokens": 10 }
    })
}

/// Build the full server against a wiremock-backed Anthropic endpoint and an
/// in-memory store; serve it over a duplex pipe; return the connected client
/// plus the storage handle for record assertions.
pub async fn serve(
    mock: &MockServer,
    timeout_ms: u64,
) -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    Arc<SqliteStorage>,
    rmcp::service::RunningService<rmcp::service::RoleServer, Parallax>,
) {
    let mut config = test_config(timeout_ms);
    // 028: the whole pool — including the per-effort variants — resolves
    // against the mock. Without this only the injected client was redirected,
    // so any call carrying an effort left the seam for the live endpoint.
    config.anthropic_api_base = mock.uri();
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

/// Build the full server with the memory capability ON: a real `VoyageClient`
/// pointed at the same wiremock server (`/v1/embeddings` beside
/// `/v1/messages`), persisted to `db_path`.
pub async fn serve_with_memory(
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

pub fn tool_call(name: &str, arguments: &Value) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new(name.to_string());
    params.arguments = arguments.as_object().cloned();
    params
}

/// Deterministic per-content embeddings: distinct contents get nearly
/// orthogonal vectors; the query lands closest to the "alpha" document.
pub fn mount_embeddings(mock: &MockServer) -> impl std::future::Future<Output = ()> + '_ {
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

/// Build the full server with research ON: a real `BraveClient` pointed at
/// wiremock (`/res/v1/web/search`), pages and `/v1/messages` served by the
/// same mock server. The fetcher is the real `HygieneFetcher` hitting
/// wiremock over localhost.
pub async fn serve_with_research(
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
pub async fn mount_research_llm(mock: &MockServer, answer: &str) {
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
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(
            &json!({ "answer": answer, "gaps": [], "gap_targets": [] }),
        )))
        .mount(mock)
        .await;
}

pub async fn mount_search_and_page(mock: &MockServer) {
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

/// Route the check translation hop by its prompt marker.
pub async fn mount_check_translation(mock: &MockServer, translation: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("deterministic checking"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&translation)))
        .mount(mock)
        .await;
}

/// A translator reply in its index-aligned wire shape, one tuple per part:
/// (text, checkable, reason, engine, form, polarity — sent as `stated`).
pub fn check_parts(parts: &[(&str, bool, &str, &str, &str, &str)]) -> Value {
    json!({
        "parts": parts.iter().map(|p| p.0).collect::<Vec<_>>(),
        "checkable": parts.iter().map(|p| p.1).collect::<Vec<_>>(),
        "reasons": parts.iter().map(|p| p.2).collect::<Vec<_>>(),
        "engines": parts.iter().map(|p| p.3).collect::<Vec<_>>(),
        "forms": parts.iter().map(|p| p.4).collect::<Vec<_>>(),
        "stated": parts.iter().map(|p| p.5).collect::<Vec<_>>(),
    })
}

/// A one-part arithmetic reply.
pub fn check_arithmetic(expression: &str) -> Value {
    check_parts(&[("the claim", true, "", "arithmetic", expression, "")])
}

/// Write a transcript fixture: `commands` as one bash `tool_use` line each
/// (failing when marked), for session `session`.
pub fn write_transcript(dir: &std::path::Path, session: &str, commands: &[(&str, bool)]) -> String {
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

/// Build the server with grounded-verify enabled against a real source root.
pub async fn serve_with_grounded(
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
