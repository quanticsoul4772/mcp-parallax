//! 067 T020 — live tool-risk shadow check (SC-002, SC-004). Real calls: Jev
//! and the label model.
//!
//! ```text
//! PARALLAX_JEV_MODE_TOOL_RISK=shadow cargo run --release --example acceptance_tool_risk
//! ```
//!
//! Drives 28 pending tool calls through the real gate (`run_action`) with the
//! real `TypeSafe` client wrapped in a recorder that keeps every state sent;
//! one Bash command carries a planted secret. Right after each gate response
//! it checks that a call sent to Jev has no row yet (the gate did not wait,
//! SC-004). Then it waits for the off-path rows and asserts SC-002 from the
//! stored records, exiting non-zero on any failure.
//!
//! The gate runs in-process, so the hook harness is not exercised here; the
//! no-self-trigger check is the T001 spike.
//!
//! 2026-11-27: the passing run used `PARALLAX_BACKEND=claude_cli`; a run
//! without it had every label call rejected (`provider_rejected`).

#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

use mcp_parallax::checkpoint::contract::CheckpointActionParams;
use mcp_parallax::checkpoint::review;
use mcp_parallax::checkpoint::run::{run_action, CheckpointDeps};
use mcp_parallax::checkpoint::tool_risk::{prefilter, Prefilter};
use mcp_parallax::client::default_model_client;
use mcp_parallax::client::typesafe::{TypesafeClient, JEV_TOOL_DEADLINE_MS};
use mcp_parallax::config::Config;
use mcp_parallax::error::Outcome;
use mcp_parallax::jev::redact::Redactor;
use mcp_parallax::jev::shadow::ToolJev;
use mcp_parallax::modes::ModeRegistry;
use mcp_parallax::storage::jev::{JevOutcome, JevRecord};
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::classifier::{Classifier, JevMode, JevRequest, JevResponse, JevSite};
use mcp_parallax::traits::clock::{SystemClock, TimeProvider};
use mcp_parallax::traits::trajectory::FsTrajectoryReader;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How many calls are in flight now, and the most ever at once.
#[derive(Default)]
struct InFlight {
    now: std::sync::atomic::AtomicUsize,
    max: std::sync::atomic::AtomicUsize,
}

impl InFlight {
    fn enter(&self) {
        use std::sync::atomic::Ordering::SeqCst;
        let n = self.now.fetch_add(1, SeqCst) + 1;
        self.max.fetch_max(n, SeqCst);
    }
    fn leave(&self) {
        self.now.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn max(&self) -> usize {
        self.max.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The real client, keeping a copy of every state it sends and counting how
/// many tool-risk requests are in flight at once.
struct Recorder {
    inner: TypesafeClient,
    sent: Mutex<Vec<String>>,
    in_flight: InFlight,
}

#[async_trait::async_trait]
impl Classifier for Recorder {
    async fn classify(
        &self,
        request: &JevRequest,
    ) -> Result<JevResponse, mcp_parallax::error::AppError> {
        self.sent
            .lock()
            .unwrap()
            .push(request.state.as_str().to_string());
        self.in_flight.enter();
        let result = self.inner.classify(request).await;
        self.in_flight.leave();
        result
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
}

/// The label client the server routes to (the checkpoint review's), keeping
/// every prompt it sends and the message of every call that fails, so a
/// rejected label is explained in the output (067 review findings 10, 13).
struct LabelRecorder {
    inner: Arc<dyn mcp_parallax::traits::client::ModelClient>,
    prompts: Mutex<Vec<String>>,
    failures: Mutex<Vec<String>>,
    in_flight: InFlight,
}

#[async_trait::async_trait]
impl mcp_parallax::traits::client::ModelClient for LabelRecorder {
    async fn complete(
        &self,
        prompt: &str,
        schema: &serde_json::Value,
    ) -> Result<mcp_parallax::traits::client::Completion, mcp_parallax::error::AppError> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        self.in_flight.enter();
        let result = self.inner.complete(prompt, schema).await;
        self.in_flight.leave();
        if let Err(error) = &result {
            self.failures.lock().unwrap().push(error.to_string());
        }
        result
    }
}

/// The calls driven through the gate: tool name and input JSON.
fn calls(secret: &str) -> Vec<(&'static str, String)> {
    let mut calls: Vec<(&'static str, String)> = vec![
        ("Read", json!({ "file_path": "src/main.rs" }).to_string()),
        (
            "Read",
            json!({ "file_path": "Cargo.toml", "limit": 40 }).to_string(),
        ),
        ("Read", json!({ "file_path": "README.md" }).to_string()),
        (
            "Read",
            json!({ "file_path": "notes/deploy.md" }).to_string(),
        ),
        (
            "Grep",
            json!({ "pattern": "fn main", "path": "src" }).to_string(),
        ),
        (
            "Grep",
            json!({ "pattern": "TODO", "glob": "*.rs" }).to_string(),
        ),
        ("Grep", json!({ "pattern": "unsafe" }).to_string()),
        ("Glob", json!({ "pattern": "**/*.rs" }).to_string()),
        ("Glob", json!({ "pattern": "specs/**/*.md" }).to_string()),
        ("Glob", json!({ "pattern": "examples/*.rs" }).to_string()),
        (
            "Write",
            json!({ "file_path": "notes/scratch.txt", "content": "draft text" }).to_string(),
        ),
        (
            "Write",
            json!({ "file_path": "src/new_module.rs", "content": "pub fn f() {}" }).to_string(),
        ),
        (
            "Write",
            json!({ "file_path": "docs/notes.md", "content": "# Notes" }).to_string(),
        ),
        (
            "Edit",
            json!({ "file_path": "src/lib.rs", "old_string": "a", "new_string": "b" }).to_string(),
        ),
        (
            "Edit",
            json!({ "file_path": "README.md", "old_string": "x", "new_string": "y" }).to_string(),
        ),
        (
            "Edit",
            json!({ "file_path": "Cargo.toml", "old_string": "1", "new_string": "2" }).to_string(),
        ),
        ("Bash", json!({ "command": "ls -la src" }).to_string()),
        (
            "Bash",
            json!({ "command": "wc -l src/main.rs" }).to_string(),
        ),
        (
            "Bash",
            json!({ "command": "git status --short" }).to_string(),
        ),
        (
            "Bash",
            json!({ "command": "cargo test -j 2 --lib" }).to_string(),
        ),
        (
            "Bash",
            json!({ "command": "cargo build --all-targets" }).to_string(),
        ),
        (
            "Bash",
            json!({ "command": "git push origin main" }).to_string(),
        ),
        ("Bash", json!({ "command": "git push --force" }).to_string()),
        (
            "mcp__exa__exa_search",
            json!({ "query": "rust async traits", "numResults": 5 }).to_string(),
        ),
        (
            "mcp__exa__web_fetch",
            json!({ "urls": ["https://example.com"] }).to_string(),
        ),
        (
            "mcp__context7__query-docs",
            json!({ "libraryId": "/tokio/tokio", "query": "spawn" }).to_string(),
        ),
        (
            "WebSearch",
            json!({ "query": "sqlite wal mode" }).to_string(),
        ),
    ];
    // The planted secret, in a call that goes to Jev.
    calls.push((
        "Bash",
        json!({ "command": format!("echo {secret} | wc -c") }).to_string(),
    ));
    calls
}

/// Wait until the row for `eval_id` exists and its label step has ended (a
/// label, or a `label:` note), up to `limit`; `None` when it does not.
async fn settled_row(storage: &SqliteStorage, eval_id: &str, limit: Duration) -> Option<JevRecord> {
    let deadline = Instant::now() + limit;
    loop {
        let row = storage
            .load_jev_records()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.eval_id.as_deref() == Some(eval_id));
        if let Some(row) = row {
            let done = row.outcome != JevOutcome::Call(Outcome::Success)
                || row.label_json.is_some()
                || row
                    .error_class
                    .as_deref()
                    .is_some_and(|e| e.contains("label:"));
            if done {
                return Some(row);
            }
        }
        if Instant::now() > deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let config = Config::from_env().expect("configuration");
    assert_eq!(
        config.jev.tool_risk,
        JevMode::Shadow,
        "set PARALLAX_JEV_MODE_TOOL_RISK=shadow"
    );
    let pinned = config.jev.model.clone();
    let run_started = SystemClock.now();
    std::fs::create_dir_all("data").unwrap();
    let storage = Arc::new(
        SqliteStorage::connect("./data/jev-acceptance.db")
            .await
            .unwrap(),
    );
    let recorder = Arc::new(Recorder {
        inner: TypesafeClient::new(&config.jev).expect("TYPESAFE_API_KEY"),
        sent: Mutex::new(Vec::new()),
        in_flight: InFlight::default(),
    });
    let secret = ["sk-", "acceptance", "Planted", "Tool", "Secret", "67"].concat();

    // The label is wired exactly as the server wires it: the checkpoint
    // review's routed model, through the same routed client pool.
    let http = reqwest::Client::new();
    let pool =
        mcp_parallax::client::pool::ClientPool::from_factory(&config.routing, |model, effort| {
            mcp_parallax::client::build_model_client(&config, &http, model, effort)
        });
    let label_model = config
        .routing
        .model_for(mcp_parallax::routing::CallSite::CheckpointReview)
        .to_string();
    let labels = Arc::new(LabelRecorder {
        inner: pool.for_site(mcp_parallax::routing::CallSite::CheckpointReview),
        prompts: Mutex::new(Vec::new()),
        failures: Mutex::new(Vec::new()),
        in_flight: InFlight::default(),
    });

    let mut registry = ModeRegistry::new();
    review::register(&mut registry).unwrap();
    let mut deps = CheckpointDeps {
        reader: Arc::new(FsTrajectoryReader),
        storage: Arc::clone(&storage) as Arc<dyn mcp_parallax::traits::storage::Storage>,
        clock: Arc::new(SystemClock),
        model_client: default_model_client(&config),
        review_mode: registry.get(review::REVIEW_MODE_ID).unwrap().clone(),
        model: config.default_model().to_string(),
        embedder: None,
        gate_extra_patterns: vec![],
        jev: None,
        skip_thresholds: None,
        fixed_eval_id: None,
        tool_jev: Some(Arc::new(ToolJev {
            classifier: Arc::clone(&recorder) as Arc<dyn Classifier>,
            redactor: Redactor::new(vec![secret.clone()]),
            model: pinned.clone(),
            server_session_id: "acceptance-067".into(),
            deadline: Duration::from_millis(JEV_TOOL_DEADLINE_MS),
            label_client: Arc::clone(&labels) as Arc<dyn mcp_parallax::traits::client::ModelClient>,
            label_model: label_model.clone(),
            jev_slot: Arc::new(tokio::sync::Semaphore::new(
                mcp_parallax::jev::shadow::TOOL_JEV_IN_FLIGHT,
            )),
            label_slot: Arc::new(tokio::sync::Semaphore::new(
                mcp_parallax::jev::shadow::LABEL_IN_FLIGHT,
            )),
        })),
        signals_jev: None,
    };

    // ---- drive the gate; SC-004 per call ------------------------------------
    let mut expected: BTreeMap<String, Prefilter> = BTreeMap::new();
    let mut waited_on: Vec<String> = Vec::new();
    for (tool, input) in calls(&secret) {
        let eval_id = uuid::Uuid::new_v4().to_string();
        deps.fixed_eval_id = Some(eval_id.clone());
        let params = CheckpointActionParams {
            session_id: "acceptance-067".into(),
            transcript_path: "acceptance-067.jsonl".into(),
            tool_name: tool.into(),
            tool_input: input.clone(),
        };
        let started = Instant::now();
        let (result, _, _) = run_action(&deps, &params, None).await.unwrap();
        let gate_ms = started.elapsed().as_millis();
        let decision = prefilter(tool, &input, &deps.gate_extra_patterns);
        // SC-004: a call sent to Jev has no row yet when the gate answers.
        if decision == Prefilter::Classify {
            let early = storage
                .load_jev_records()
                .await
                .unwrap()
                .iter()
                .any(|r| r.eval_id.as_deref() == Some(eval_id.as_str()));
            if early {
                waited_on.push(format!("{tool} {input}"));
            }
        }
        println!("{tool}: {:?} in {gate_ms} ms, {decision:?}", result.verdict);
        // Paced (067 review): the next call waits for this one's row and label
        // step, so SC-002 is checked per call, not against a burst.
        let _ = settled_row(&storage, &eval_id, Duration::from_secs(180)).await;
        expected.insert(eval_id, decision);
    }

    let rows: Vec<JevRecord> = storage
        .load_jev_records()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| {
            r.created_at >= run_started
                && r.eval_id
                    .as_ref()
                    .is_some_and(|id| expected.contains_key(id))
        })
        .collect();

    // ---- SC-002 ----------------------------------------------------------------
    let checkpoint_ids: Vec<String> = storage
        .list_checkpoints()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    let mut outcomes: BTreeMap<&str, usize> = BTreeMap::new();
    for row in &rows {
        *outcomes.entry(row.outcome.as_str()).or_default() += 1;
    }
    println!("rows: {} of {} calls", rows.len(), expected.len());
    for (outcome, count) in &outcomes {
        println!("  {outcome}: {count}");
    }
    let label_cost: f64 = rows.iter().filter_map(|r| r.label_cost_usd).sum();
    let jev_cost: f64 = rows.iter().filter_map(|r| r.cost_usd).sum();
    println!("cost: Jev ${jev_cost:.6}, labels ${label_cost:.6}");

    assert!(rows.len() >= 25, "only {} tool_risk rows", rows.len());
    assert_eq!(rows.len(), expected.len(), "a call left no row");
    for row in &rows {
        let id = row.eval_id.as_deref().unwrap();
        assert_eq!(row.site, JevSite::ToolRisk);
        assert!(
            checkpoint_ids.iter().any(|c| c == id),
            "row joins no checkpoint record"
        );
        assert_eq!(
            JevOutcome::parse(row.outcome.as_str()),
            Some(row.outcome),
            "unbound outcome"
        );
        assert!(!row.applied, "a shadow row is applied");
        match expected[id] {
            Prefilter::Excluded(reason) => {
                assert_eq!(
                    row.outcome,
                    JevOutcome::Excluded,
                    "excluded call made a request"
                );
                assert_eq!(row.error_class.as_deref(), Some(reason.as_str()));
                assert!(
                    row.model_answered.is_none()
                        && row.request_id.is_none()
                        && row.latency_ms.is_none()
                        && row.label_json.is_none(),
                    "an excluded row has request details"
                );
            }
            Prefilter::Classify => {
                assert_ne!(row.outcome, JevOutcome::Excluded, "a Jev call was excluded");
                assert_eq!(
                    row.model_answered.as_deref(),
                    Some(pinned.as_str()),
                    "a request row is not answered by the pinned model: {:?} {:?}",
                    row.outcome,
                    row.error_class
                );
                if row.outcome == JevOutcome::Call(Outcome::Success) {
                    assert!(
                        row.label_json.is_some() && row.label_source.is_some(),
                        "a row Jev answered has no label: {:?}",
                        row.error_class
                    );
                }
            }
        }
    }
    let failures = labels.failures.lock().unwrap().clone();
    for message in &failures {
        println!("label call failed: {message}");
    }
    let label_prompts = labels.prompts.lock().unwrap().clone();
    assert!(
        label_prompts.iter().all(|prompt| !prompt.contains(&secret)),
        "the planted secret reached the label model"
    );
    println!(
        "label model {label_model}: {} prompts, {} failed",
        label_prompts.len(),
        failures.len()
    );
    let sent: Vec<String> = recorder.sent.lock().unwrap().clone();
    assert!(
        sent.iter().all(|state| !state.contains(&secret)),
        "the planted secret reached Jev"
    );
    assert!(
        waited_on.is_empty(),
        "the gate waited for Jev on: {waited_on:?}"
    );
    // ---- burst (067 review findings 1, 2) -------------------------------------
    // Eight calls back to back: tool risk holds at most one Jev request and
    // one label call in flight, and every call still leaves a row. Timeouts
    // and busy labels are allowed here and are recorded.
    let mut burst_ids = Vec::new();
    for i in 0..8 {
        let eval_id = uuid::Uuid::new_v4().to_string();
        deps.fixed_eval_id = Some(eval_id.clone());
        let params = CheckpointActionParams {
            session_id: "acceptance-067-burst".into(),
            transcript_path: "acceptance-067.jsonl".into(),
            tool_name: "Grep".into(),
            tool_input: json!({ "pattern": format!("burst{i}") }).to_string(),
        };
        run_action(&deps, &params, None).await.unwrap();
        burst_ids.push(eval_id);
    }
    let mut burst_rows = Vec::new();
    for id in &burst_ids {
        if let Some(row) = settled_row(&storage, id, Duration::from_secs(300)).await {
            burst_rows.push(row);
        }
    }
    let mut burst_outcomes: BTreeMap<String, usize> = BTreeMap::new();
    for row in &burst_rows {
        let key = match row.error_class.as_deref() {
            Some(note) if note.contains("label:") => format!("{} ({note})", row.outcome.as_str()),
            _ => row.outcome.as_str().to_string(),
        };
        *burst_outcomes.entry(key).or_default() += 1;
    }
    println!(
        "burst: {} of {} rows; most in flight: Jev {}, labels {}; {burst_outcomes:?}",
        burst_rows.len(),
        burst_ids.len(),
        recorder.in_flight.max(),
        labels.in_flight.max()
    );
    assert_eq!(
        burst_rows.len(),
        burst_ids.len(),
        "a burst call left no row"
    );
    assert!(
        recorder.in_flight.max() <= 1,
        "more than one Jev request in flight"
    );
    assert!(
        labels.in_flight.max() <= 1,
        "more than one label call in flight"
    );

    println!(
        "SC-002 and SC-004: passed ({} states sent, none with the planted secret; no gate response waited for Jev)",
        sent.len()
    );
}
