//! Live acceptance for 060-claude-cli-backend (quickstart §Live acceptance).
//!
//! Runs real `claude -p` calls on the operator's own signed-in Claude Code, so
//! it spends subscription usage; no Anthropic API key is used (the client
//! removes it from every call's environment).
//!
//! - Part A (SC-001, SC-002): every always-on tool once through the server,
//!   plus one quick `research` question when `EXA_API_KEY` is set. Each must
//!   succeed and record `cost_basis = cli_estimate`.
//! - Part B (SC-003, SC-004): 20 `decide`-schema calls straight through the
//!   CLI client. The CLI's overhead is wall time minus the reply's
//!   `duration_api_ms`; each reply's first attempt is checked by the local
//!   validator against decide's registered schema.
//! - Part C (SC-005): every request the runner saw carried the isolation
//!   flags and variables and removed the API-billing variables.
//!
//! Run: `cargo run --example acceptance_claude_cli`

#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use mcp_parallax::client::claude_cli::{create_private_folder, process_runner};
use mcp_parallax::client::{ClaudeCliClient, ExaClient};
use mcp_parallax::config::{Backend, Config};
use mcp_parallax::error::AppError;
use mcp_parallax::routing::{DefaultVar, RoutingTable};
use mcp_parallax::server::Parallax;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::telemetry::CostBasis;
use mcp_parallax::traits::client::ModelClient;
use mcp_parallax::traits::clock::SystemClock;
use mcp_parallax::traits::runner::{CliOutput, CliRequest, CliRunner};
use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Wraps the real runner, keeping every request with its wall time and output.
struct Recording {
    inner: Arc<dyn CliRunner>,
    log: Mutex<Vec<(CliRequest, u128, Option<CliOutput>)>>,
}

#[async_trait::async_trait]
impl CliRunner for Recording {
    async fn run(&self, request: &CliRequest) -> Result<CliOutput, AppError> {
        let started = Instant::now();
        let result = self.inner.run(request).await;
        let wall = started.elapsed().as_millis();
        self.log
            .lock()
            .unwrap()
            .push((request.clone(), wall, result.as_ref().ok().cloned()));
        result
    }
}

fn cli_config() -> Config {
    let base = common::config();
    let model = std::env::var("CLAUDE_CLI_MODEL").unwrap_or_else(|_| base.claude_cli_model.clone());
    Config {
        backend: Backend::ClaudeCli,
        anthropic_api_key: String::new(),
        claude_cli_model: model.clone(),
        routing: RoutingTable::single(&model, DefaultVar::ClaudeCliModel),
        request_timeout_ms: 180_000,
        max_retries: 1,
        exa_api_key: std::env::var("EXA_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty()),
        ..base
    }
}

fn call(name: &str, args: &Value) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new(name.to_string());
    params.arguments = args.as_object().cloned();
    params
}

/// Part A: each always-on tool once, and quick research when Exa is set.
async fn part_a(config: &Config, recording: &Arc<Recording>, root: &std::path::Path) -> bool {
    let folder = create_private_folder(&std::env::temp_dir()).unwrap();
    let model = Arc::new(ClaudeCliClient::with_runner(
        config,
        recording.clone(),
        &folder,
        &config.claude_cli_model,
        None,
    ));
    let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
    let exa = config
        .exa_api_key
        .as_ref()
        .map(|_| Arc::new(ExaClient::new(config).unwrap()) as Arc<_>);
    let server = Parallax::with_capabilities(
        model,
        storage.clone(),
        Arc::new(SystemClock),
        config,
        None,
        exa,
    )
    .unwrap();
    let (server_io, client_io) = tokio::io::duplex(256 * 1024);
    let server_task = tokio::spawn(async move { server.serve(server_io).await });
    let client = ().serve(client_io).await.unwrap();
    let _server = server_task.await.unwrap().unwrap();

    let mut calls = vec![
        (
            "decide",
            json!({"question": "Which format for a small config file?", "options": ["TOML file", "SQLite database"]}),
        ),
        (
            "verify",
            json!({"claim": "Water boils at 100 degrees Celsius at sea level."}),
        ),
        (
            "unstick",
            json!({"goal": "find the bad config key", "blocked": "the service crashes on boot", "tried": ["reading the file top to bottom"]}),
        ),
        (
            "diverge",
            json!({"problem": "API requests time out every day at noon"}),
        ),
        ("elicit", json!({"task": "speed up the API"})),
        ("check", json!({"claim": "17 multiplied by 23 equals 391"})),
        (
            "grounded_verify",
            json!({"claim": "publish emits an event", "locators": [{"path": "pub.rs", "start_line": 1, "end_line": 1}]}),
        ),
    ];
    if config.exa_api_key.is_some() {
        calls.push(("research", json!({"question": "What year was the Rust programming language's 1.0 release?", "depth": "quick"})));
    } else {
        println!("research: skipped (EXA_API_KEY not set) — SC-002 not measured");
    }
    let _ = root; // the grounded_verify root is set on `config`

    let mut ok = true;
    for (name, args) in calls {
        let started = Instant::now();
        let result = client.call_tool(call(name, &args)).await;
        let secs = started.elapsed().as_secs_f64();
        let records = storage.list_invocations().await.unwrap();
        let record = records.iter().find(|r| r.tool == name);
        let passed = matches!(&result, Ok(r) if r.is_error != Some(true))
            && record.is_some_and(|r| r.cost_basis == Some(CostBasis::CliEstimate));
        ok &= passed;
        println!(
            "{name:<16} {}  {secs:>6.1} s  outcome={}  cost_basis={}  cost=${:.4}",
            if passed { "PASS" } else { "FAIL" },
            record.map_or("-", |r| r.outcome.as_str()),
            record
                .and_then(|r| r.cost_basis)
                .map_or("-", CostBasis::as_str),
            record.map_or(0.0, |r| r.cost_usd),
        );
        if let Ok(r) = &result {
            if r.is_error == Some(true) {
                println!("    error: {:?}", r.content);
            }
        }
    }
    client.cancel().await.unwrap();
    ok
}

/// Part B: 20 decide-schema calls; overhead and first-attempt schema passes.
async fn part_b(config: &Config, recording: &Arc<Recording>) -> bool {
    let mut registry = mcp_parallax::modes::ModeRegistry::new();
    mcp_parallax::modes::decide::register(&mut registry).unwrap();
    let mode = registry.get("decide").unwrap();
    let (output_schema, sanitized) = mode.schemas_for(2).unwrap();
    let folder = create_private_folder(&std::env::temp_dir()).unwrap();
    let client = ClaudeCliClient::with_runner(
        config,
        recording.clone(),
        &folder,
        &config.claude_cli_model,
        None,
    );
    let questions = [
        (
            "Which storage for a small CLI's settings?",
            "TOML file",
            "SQLite database",
        ),
        (
            "How to cut a 50-minute CI run?",
            "Cache the build directory",
            "Split tests across jobs",
        ),
        (
            "First step when a server's API credit runs out?",
            "Wait for credit",
            "Add a subscription backend",
        ),
        (
            "Which branch strategy for a solo project?",
            "Trunk-based",
            "Git flow",
        ),
        ("Where to log in a stdio server?", "stderr", "stdout"),
    ];
    let before = recording.log.lock().unwrap().len();
    let mut first_attempt_ok = 0;
    for i in 0..20 {
        let (q, a, b) = questions[i % questions.len()];
        let prompt = format!(
            "Score each option from 0 to 100 and give one sentence per option.\nQuestion: {q}\nOptions:\n1. {a}\n2. {b}"
        );
        match client.complete(&prompt, sanitized).await {
            Ok(c) if mcp_parallax::schema::validate(output_schema, &c.value).is_ok() => {
                first_attempt_ok += 1;
            }
            Ok(c) => println!("  call {i}: schema check failed: {}", c.value),
            Err(e) => println!("  call {i}: error: {e}"),
        }
    }
    let entries: Vec<_> = recording.log.lock().unwrap()[before..].to_vec();
    let mut overheads: Vec<i128> = entries
        .iter()
        .filter_map(|(_, wall, out)| {
            let reply: Value = serde_json::from_str(out.as_ref()?.stdout.trim()).ok()?;
            let api = i128::from(reply.get("duration_api_ms")?.as_u64()?);
            Some(i128::try_from(*wall).ok()? - api)
        })
        .collect();
    overheads.sort_unstable();
    let median = overheads
        .get(overheads.len() / 2)
        .copied()
        .unwrap_or(i128::MAX);
    let sc3 = median <= 3_000;
    let sc4 = first_attempt_ok >= 19;
    println!(
        "SC-003 {}  median CLI overhead {median} ms over {} calls (target <= 3000)",
        if sc3 { "PASS" } else { "FAIL" },
        overheads.len()
    );
    println!(
        "SC-004 {}  {first_attempt_ok}/20 schema-valid on the first attempt (target >= 19)",
        if sc4 { "PASS" } else { "FAIL" }
    );
    sc3 && sc4
}

/// Part C: every request carried the isolation flags and variables.
fn part_c(recording: &Recording) -> bool {
    let log = recording.log.lock().unwrap();
    let isolated = |r: &CliRequest| {
        let has = |flag: &str| r.args.iter().any(|a| a == flag);
        has("--strict-mcp-config")
            && r.args.iter().any(|a| a == r#"{"mcpServers":{}}"#)
            && has("--safe-mode")
            && has("--no-session-persistence")
            && r.env_remove.iter().any(|v| v == "ANTHROPIC_API_KEY")
            && r.env_set
                .contains(&("CLAUDE_CODE_DISABLE_CLAUDE_MDS".into(), "1".into()))
    };
    let bad = log.iter().filter(|(r, _, _)| !isolated(r)).count();
    let ok = bad == 0 && !log.is_empty();
    println!(
        "SC-005 {}  {} of {} requests isolated",
        if ok { "PASS" } else { "FAIL" },
        log.len() - bad,
        log.len()
    );
    ok
}

#[tokio::main]
async fn main() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("pub.rs"),
        "pub fn publish(&self) { self.emit(); }\n",
    )
    .unwrap();
    let mut config = cli_config();
    config.grounded_verify_root = Some(root.path().to_string_lossy().to_string());
    println!(
        "claude_cli live acceptance — model {} — {}",
        config.claude_cli_model,
        mcp_parallax::client::claude_cli::check_cli(&config, process_runner(&config).as_ref())
            .await
            .expect("the claude CLI must run")
    );
    let recording = Arc::new(Recording {
        inner: process_runner(&config),
        log: Mutex::new(Vec::new()),
    });

    let a = part_a(&config, &recording, root.path()).await;
    let b = part_b(&config, &recording).await;
    let c = part_c(&recording);
    let all = a && b && c;
    println!("\nOVERALL {}", if all { "PASS" } else { "FAIL" });
    if !all {
        std::process::exit(1);
    }
}
