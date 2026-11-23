//! 060 T012 (spec US1): the always-on tools run end to end on the
//! `claude_cli` backend, over MCP, with a stand-in CLI replaying canned
//! replies. No test here starts a real process (FR-014).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use mcp_parallax::client::ClaudeCliClient;
use mcp_parallax::config::{Backend, Config, DEFAULT_CLAUDE_CLI_MODEL};
use mcp_parallax::error::{AppError, Outcome};
use mcp_parallax::routing::{DefaultVar, RoutingTable};
use mcp_parallax::server::Parallax;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::clock::SystemClock;
use mcp_parallax::traits::runner::{CliOutput, CliRequest, CliRunner};
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// A stand-in CLI, recording every request it receives. It either replays
/// `structured_output` values in order (the last one repeats), or, when
/// routed, answers each prompt by the first marker its stdin contains.
struct FakeCli {
    replies: Mutex<VecDeque<Value>>,
    by_marker: Vec<(&'static str, Value)>,
    seen: Mutex<Vec<CliRequest>>,
}

impl FakeCli {
    fn replying(values: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(values.into()),
            by_marker: Vec::new(),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn routed(by_marker: Vec<(&'static str, Value)>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::new()),
            by_marker,
            seen: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl CliRunner for FakeCli {
    async fn run(&self, request: &CliRequest) -> Result<CliOutput, AppError> {
        self.seen.lock().unwrap().push(request.clone());
        let value = if self.by_marker.is_empty() {
            let mut replies = self.replies.lock().unwrap();
            if replies.len() > 1 {
                replies.pop_front().unwrap()
            } else {
                replies.front().cloned().unwrap()
            }
        } else {
            self.by_marker
                .iter()
                .find(|(marker, _)| request.stdin.contains(marker))
                .map(|(_, value)| value.clone())
                .ok_or_else(|| AppError::Client("no canned reply for this prompt".into()))?
        };
        Ok(CliOutput {
            exit_code: Some(0),
            stdout: json!({
                "subtype": "success",
                "is_error": false,
                "structured_output": value,
                "usage": {"input_tokens": 100, "output_tokens": 10},
                "total_cost_usd": 0.001
            })
            .to_string(),
            stderr: String::new(),
        })
    }
}

/// The `claude_cli` configuration: no API key anywhere.
fn cli_config() -> Config {
    let mut config = test_config(2_000);
    config.backend = Backend::ClaudeCli;
    config.anthropic_api_key = String::new();
    config.routing = RoutingTable::single(DEFAULT_CLAUDE_CLI_MODEL, DefaultVar::ClaudeCliModel);
    config
}

async fn serve_cli(
    config: &Config,
    cli: Arc<FakeCli>,
) -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    Arc<SqliteStorage>,
    rmcp::service::RunningService<rmcp::service::RoleServer, Parallax>,
) {
    let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
    let folder = std::env::temp_dir().join("parallax-claude-cli-test");
    let client = Arc::new(ClaudeCliClient::with_runner(
        config,
        cli,
        &folder,
        DEFAULT_CLAUDE_CLI_MODEL,
        None,
    ));
    let server = Parallax::new(client, storage.clone(), Arc::new(SystemClock), config).unwrap();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move { server.serve(server_io).await });
    let client = ().serve(client_io).await.expect("client init");
    let running_server = server_task.await.expect("join").expect("server init");
    (client, storage, running_server)
}

async fn tool_names(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
) -> Vec<String> {
    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    names
}

/// Call one tool against the stand-in and return its structured result,
/// asserting one successful invocation record.
async fn run_tool(config: &Config, cli: Arc<FakeCli>, name: &str, args: &Value) -> Value {
    let (client, storage, _server) = serve_cli(config, cli).await;
    let result = client.call_tool(tool_call(name, args)).await.unwrap();
    assert_ne!(result.is_error, Some(true), "{name}: {result:?}");
    let structured = result
        .structured_content
        .clone()
        .expect("structured_content");
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1, "{name}");
    assert_eq!(records[0].outcome, Outcome::Success, "{name}");
    client.cancel().await.unwrap();
    structured
}

/// The requests the stand-in received, copied out so no lock is held.
fn seen(cli: &FakeCli) -> Vec<CliRequest> {
    cli.seen.lock().unwrap().clone()
}

/// 060 T012 (US1 scenario 1): with no API key, the `claude_cli` server lists
/// the same always-on tools as the anthropic server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_an_api_key_the_catalog_matches_the_anthropic_backend() {
    let (cli_client, _, _cli_server) =
        serve_cli(&cli_config(), FakeCli::replying(vec![json!({})])).await;
    let mock = wiremock::MockServer::start().await;
    let (api_client, _, _api_server) = serve(&mock, 2_000).await;
    assert_eq!(tool_names(&cli_client).await, tool_names(&api_client).await);
    cli_client.cancel().await.unwrap();
    api_client.cancel().await.unwrap();
}

/// 060 T012 (US1): `verify` runs its three passes through the CLI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_runs_through_the_cli() {
    let cli = FakeCli::replying(vec![
        json!({"verdict": "refuted", "findings": ["1066, not 1067"]}),
    ]);
    let out = run_tool(
        &cli_config(),
        cli.clone(),
        "verify",
        &json!({"claim": "Hastings was 1067"}),
    )
    .await;
    assert_eq!(out["verdict"], "refuted");
    assert_eq!(seen(&cli).len(), 3, "verify runs three passes");
}

/// 060 T012 (US1): `unstick` runs through the CLI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unstick_runs_through_the_cli() {
    let cli = FakeCli::replying(vec![json!({
        "next_step": "Bisect the overrides file",
        "rationale": "Halving isolates the bad key.",
        "watch_for": "Keys that only fail together"
    })]);
    let out = run_tool(
        &cli_config(),
        cli,
        "unstick",
        &json!({"goal": "find the bad key", "blocked": "crash on boot", "tried": ["reading it"]}),
    )
    .await;
    assert_eq!(out["next_step"], "Bisect the overrides file");
}

/// 060 T012 (US1): `diverge` runs through the CLI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diverge_runs_through_the_cli() {
    let cli = FakeCli::replying(vec![
        json!({"framing": "a capacity problem", "implication": "add a queue"}),
    ]);
    let out = run_tool(
        &cli_config(),
        cli,
        "diverge",
        &json!({"problem": "requests time out at noon"}),
    )
    .await;
    assert!(out.to_string().contains("a capacity problem"), "{out}");
}

/// 060 T012 (US1): `decide` runs through the CLI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decide_runs_through_the_cli() {
    let cli = FakeCli::replying(vec![json!({
        "methodology": "weigh",
        "deciding_factors": ["cost"],
        "option_01_score": 80,
        "option_01_rationale": "cheap",
        "option_02_score": 20,
        "option_02_rationale": "costly"
    })]);
    let out = run_tool(
        &cli_config(),
        cli,
        "decide",
        &json!({"question": "which store?", "options": ["toml file", "sqlite"]}),
    )
    .await;
    assert_eq!(out["recommended"], "toml file");
}

/// 060 T012 (US1): `elicit` runs through the CLI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elicit_runs_through_the_cli() {
    let cli = FakeCli::replying(vec![json!({
        "assumed_objective": "ship the fix",
        "preference_texts": ["p99 latency target"],
        "preference_signals": ["the request"],
        "preference_strengths": ["stated"],
        "divergence_questions": ["which region?"],
        "divergence_signals": ["no region given"],
        "signal_level": "medium"
    })]);
    let out = run_tool(
        &cli_config(),
        cli,
        "elicit",
        &json!({"task": "speed up the API"}),
    )
    .await;
    assert_eq!(out["assumed_objective"], "ship the fix");
}

/// 060 T012 (US1): `check` runs through the CLI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_runs_through_the_cli() {
    let cli = FakeCli::replying(vec![check_arithmetic("1 + 1 == 2")]);
    let out = run_tool(
        &cli_config(),
        cli,
        "check",
        &json!({"claim": "one plus one is two"}),
    )
    .await;
    assert_eq!(out["verdict"], "supported");
}

/// 060 T012 (US1): `grounded_verify` runs through the CLI, and the verbatim
/// source reaches it on stdin.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grounded_verify_runs_through_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("pub.rs"),
        "pub fn publish(&self) { self.emit(); }\n",
    )
    .unwrap();
    let mut config = cli_config();
    config.grounded_verify_root = Some(dir.path().to_str().unwrap().to_string());
    let cli = FakeCli::replying(vec![json!({
        "verdict": "supported",
        "findings": [],
        "missing_evidence": [],
        "needs_computation": false
    })]);
    let out = run_tool(
        &config,
        cli.clone(),
        "grounded_verify",
        &json!({"claim": "publish emits", "locators": [{"path": "pub.rs", "start_line": 1, "end_line": 1}]}),
    )
    .await;
    assert_eq!(out["verdict"], "supported");
    assert!(
        seen(&cli).iter().all(|r| r.stdin.contains("self.emit()")),
        "the verbatim source reached the CLI on stdin"
    );
}

/// 060 T012 (US1 scenario 4): a first reply the local check rejects, then a
/// valid one, completes through the existing single violation-fed retry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_first_reply_recovers_through_the_single_retry() {
    let cli = FakeCli::replying(vec![
        // The engine's dialect rejects bare `abs`; the retry's form is valid.
        check_arithmetic("abs(2.0 - 2.0) <= 0.1"),
        check_arithmetic("math::abs(2.0 - 2.0) <= 0.1"),
    ]);
    let out = run_tool(
        &cli_config(),
        cli.clone(),
        "check",
        &json!({"claim": "2 is about 2"}),
    )
    .await;
    assert_eq!(out["verdict"], "supported");
    assert_eq!(out["translation_attempts"], 2);
    let seen = seen(&cli);
    assert_eq!(seen.len(), 2);
    assert!(
        seen[1].stdin.contains("REJECTED"),
        "the retry carried the violation"
    );
}

/// 060 T015 (US2 scenario 1): `research` completes on `claude_cli`, every
/// model hop through the CLI, sources from an Exa double.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn research_runs_through_the_cli() {
    let mock = wiremock::MockServer::start().await;
    mount_search_and_page(&mock).await;
    let mut config = cli_config();
    config.request_timeout_ms = 5_000;
    config.exa_api_key = Some("exa-test-key".into());
    let cli = FakeCli::routed(vec![
        (
            "scoping a web research run",
            json!({"angles": ["first angle"], "sub_questions": ["does it hold?"]}),
        ),
        (
            "extract falsifiable claims",
            json!({"claims": ["the documented claim holds"]}),
        ),
        (
            "adversarial fact-checker",
            json!({"verdict": "supported", "findings": []}),
        ),
        (
            "executive synthesis",
            json!({"answer": "The claim holds [s1].", "gaps": [], "gap_targets": []}),
        ),
    ]);
    let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
    let model = Arc::new(ClaudeCliClient::with_runner(
        &config,
        cli.clone(),
        &std::env::temp_dir().join("parallax-claude-cli-test"),
        DEFAULT_CLAUDE_CLI_MODEL,
        None,
    ));
    let exa = Arc::new(mcp_parallax::client::ExaClient::with_base_url(
        &config,
        "exa-test-key",
        &mock.uri(),
    ));
    let server = Parallax::with_capabilities(
        model,
        storage.clone(),
        Arc::new(SystemClock),
        &config,
        None,
        Some(exa),
    )
    .unwrap();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move { server.serve(server_io).await });
    let client = ().serve(client_io).await.expect("client init");
    let _server = server_task.await.expect("join").expect("server init");

    let result = client
        .call_tool(tool_call(
            "research",
            &json!({"question": "does the documented claim hold?", "depth": "quick"}),
        ))
        .await
        .unwrap();
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content");
    assert_eq!(structured["answer"], "The claim holds [s1].");
    let prompts: Vec<String> = seen(&cli).into_iter().map(|r| r.stdin).collect();
    for marker in [
        "scoping",
        "extract falsifiable",
        "adversarial",
        "executive synthesis",
    ] {
        assert!(
            prompts.iter().any(|p| p.contains(marker)),
            "no research hop reached the CLI with `{marker}`"
        );
    }
    let records = storage.list_invocations().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::Success);
    client.cancel().await.unwrap();
}

/// 060 T015 (US2): the `checkpoint_turn` review runs through the CLI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_checkpoint_review_runs_through_the_cli() {
    let cli = FakeCli::replying(vec![json!({
        "contradicts": false,
        "statement_a": "",
        "statement_b": "",
        "basis": "Nothing to compare.",
        "violates": false,
        "violated_preference": "",
        "violation_basis": "",
        "capture_worthy": false,
        "capture_kind": "none",
        "capture_content": "",
        "capture_basis": ""
    })]);
    let (client, storage, _server) = serve_cli(&cli_config(), cli.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    // An earlier statement the final message may contradict: without one the
    // review has no candidate and, by design, makes no model call.
    let path = dir.path().join("transcript.jsonl");
    std::fs::write(
        &path,
        format!(
            "{}
",
            json!({
                "type": "assistant",
                "sessionId": "cs-cli",
                "message": {"role": "assistant", "content": [
                    {"type": "text", "text": "The database migration is fully reversible and safe to run."}
                ]}
            })
        ),
    )
    .unwrap();
    let path = path.to_string_lossy().to_string();
    let result = client
        .call_tool(tool_call(
            "checkpoint_turn",
            &json!({
                "session_id": "cs-cli",
                "transcript_path": path,
                "final_message": "Correction: the database migration is not reversible.",
                "continuation": false
            }),
        ))
        .await
        .unwrap();
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content");
    assert_eq!(structured["verdict"], "silence");
    assert_eq!(
        seen(&cli).len(),
        1,
        "the review made its one model call through the CLI"
    );
    assert_eq!(storage.list_checkpoints().await.unwrap().len(), 1);
    client.cancel().await.unwrap();
}
