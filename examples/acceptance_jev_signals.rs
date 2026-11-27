//! 068 T019 — live turn-signal shadow check (SC-005: asserts SC-001 to
//! SC-003). Real Jev calls; the review does not run (no statement pairs,
//! memory off), so no big-model call is made.
//!
//! ```text
//! PARALLAX_JEV_MODE_TURN_SIGNALS=shadow cargo run --release --example acceptance_jev_signals
//! ```
//!
//! Each fixture turn runs through the real `run_turn` twice, with the site
//! off and in shadow, against a fresh scratch database and scratch
//! transcripts. The real `TypeSafe` client is wrapped in a recorder that keeps
//! every request's question and state. Exits non-zero on any failure.

#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

use mcp_parallax::checkpoint::contract::CheckpointTurnParams;
use mcp_parallax::checkpoint::review;
use mcp_parallax::checkpoint::run::{run_turn, CheckpointDeps};
use mcp_parallax::checkpoint::CheckpointRecord;
use mcp_parallax::client::default_model_client;
use mcp_parallax::client::typesafe::{TypesafeClient, JEV_TURN_DEADLINE_MS};
use mcp_parallax::config::Config;
use mcp_parallax::jev::redact::Redactor;
use mcp_parallax::jev::shadow::{claim_exclusion, SignalsJev};
use mcp_parallax::modes::ModeRegistry;
use mcp_parallax::storage::jev::JevRecord;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::classifier::{Classifier, JevMode, JevRequest, JevResponse, JevSite};
use mcp_parallax::traits::clock::SystemClock;
use mcp_parallax::traits::trajectory::FsTrajectoryReader;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Planted in every user prompt; it must reach no request without the opt-in.
const MARKER: &str = "PUSHBACK-MARKER-068";

/// The real client, keeping every request's first question id and state.
struct Recorder {
    inner: TypesafeClient,
    sent: Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl Classifier for Recorder {
    async fn classify(
        &self,
        request: &JevRequest,
    ) -> Result<JevResponse, mcp_parallax::error::AppError> {
        self.sent.lock().unwrap().push((
            request.questions[0].id.clone(),
            request.state.as_str().to_string(),
        ));
        self.inner.classify(request).await
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
}

/// A fixture turn: the user's prompt, an earlier assistant statement, and
/// the final message (with number and date claims in the first).
struct Turn {
    prompt: String,
    earlier: &'static str,
    final_message: &'static str,
}

fn turns() -> Vec<Turn> {
    vec![
        Turn {
            prompt: format!("{MARKER} That is wrong; it failed on 12 March."),
            earlier: "I ran the test suite.",
            final_message: "The tests pass now. It has 4096 entries. I checked it on Tuesday.",
        },
        Turn {
            prompt: format!("{MARKER} Did you read the docs?"),
            earlier: "I read the README.",
            final_message:
                "The README describes the configuration. The parser handles empty lines.",
        },
        Turn {
            prompt: format!("{MARKER} Please summarize the change."),
            earlier: "I updated the plan.",
            final_message: "The plan is updated. Three tasks remain. Nothing else changed.",
        },
    ]
}

/// Write a scratch transcript: the user's prompt, then the earlier statement.
fn transcript(dir: &std::path::Path, session: &str, turn: &Turn) -> String {
    let lines = [
        json!({ "type": "user", "sessionId": session, "origin": { "kind": "human" },
                "message": { "role": "user", "content": turn.prompt } }),
        json!({ "type": "assistant", "sessionId": session,
                "message": { "role": "assistant",
                             "content": [{ "type": "text", "text": turn.earlier }] } }),
    ];
    let path = dir.join(format!("{session}.jsonl"));
    let mut text = String::new();
    for line in &lines {
        std::fmt::Write::write_fmt(&mut text, format_args!("{line}\n")).unwrap();
    }
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().to_string()
}

/// One `run_turn`; returns the result (minus its latency) and the eval id.
async fn turn_once(
    deps: &mut CheckpointDeps,
    session: &str,
    path: &str,
    final_message: &str,
    continuation: bool,
) -> (Value, String) {
    let eval_id = uuid::Uuid::new_v4().to_string();
    deps.fixed_eval_id = Some(eval_id.clone());
    let params = CheckpointTurnParams {
        session_id: session.into(),
        transcript_path: path.into(),
        final_message: final_message.into(),
        continuation,
    };
    let (result, _, _) = run_turn(deps, &params, None).await.unwrap();
    let mut result = serde_json::to_value(&result).unwrap();
    result.as_object_mut().unwrap().remove("latency_ms");
    (result, eval_id)
}

/// The checkpoint record for `eval_id`, minus id, latency, timestamp and Jev
/// cost (the cost is checked on its own, plan D7a).
async fn record(storage: &SqliteStorage, eval_id: &str) -> CheckpointRecord {
    let mut r = storage
        .list_checkpoints()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == eval_id)
        .expect("checkpoint record");
    r.id = String::new();
    r.latency_ms = 0;
    r.created_at = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
    r.jev_cost_usd = None;
    r
}

/// The turn-signal rows for `eval_id`, waiting up to `limit` for `n`.
async fn rows_for(
    storage: &SqliteStorage,
    eval_id: &str,
    n: usize,
    limit: Duration,
) -> Vec<JevRecord> {
    let deadline = Instant::now() + limit;
    loop {
        let rows: Vec<JevRecord> = storage
            .load_jev_records()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.site == JevSite::TurnSignals && r.eval_id.as_deref() == Some(eval_id))
            .collect();
        if rows.len() >= n || Instant::now() > deadline {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn question_id(row: &JevRecord) -> String {
    row.questions_json[0]["id"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let config = Config::from_env().expect("configuration");
    assert_eq!(
        config.jev.turn_signals,
        JevMode::Shadow,
        "set PARALLAX_JEV_MODE_TURN_SIGNALS=shadow"
    );
    let pinned = config.jev.model.clone();
    let dir = std::env::temp_dir().join(format!("acceptance-068-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("scratch.db").to_string_lossy().to_string();
    let storage = Arc::new(SqliteStorage::connect(&db).await.unwrap());
    let recorder = Arc::new(Recorder {
        inner: TypesafeClient::new(&config.jev).expect("TYPESAFE_API_KEY"),
        sent: Mutex::new(Vec::new()),
    });
    let signals = |send_user_text: bool| {
        Arc::new(SignalsJev {
            classifier: Arc::clone(&recorder) as Arc<dyn Classifier>,
            redactor: Redactor::from_env(),
            model: pinned.clone(),
            server_session_id: "acceptance-068".into(),
            deadline: Duration::from_millis(JEV_TURN_DEADLINE_MS),
            send_user_text,
            grounded_verify: false,
            research: false,
        })
    };
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
        tool_jev: None,
        signals_jev: None,
    };
    let wait = Duration::from_millis(JEV_TURN_DEADLINE_MS * 10);

    // ---- SC-001: off versus shadow, one row per question --------------------
    let mut failures: Vec<String> = Vec::new();
    for (i, turn) in turns().iter().enumerate() {
        let session = format!("acceptance-068-{i}");
        let path = transcript(&dir, &session, turn);
        deps.signals_jev = None;
        let (off_result, off_id) =
            turn_once(&mut deps, &session, &path, turn.final_message, false).await;
        deps.signals_jev = Some(signals(false));
        let (on_result, on_id) =
            turn_once(&mut deps, &session, &path, turn.final_message, false).await;
        let rows = rows_for(&storage, &on_id, 2, wait).await;
        let mut ids: Vec<String> = rows.iter().map(question_id).collect();
        ids.sort();
        println!(
            "turn {i}: {} rows {:?}; outcomes {:?}",
            rows.len(),
            ids,
            rows.iter().map(|r| r.outcome.as_str()).collect::<Vec<_>>()
        );
        if ids != ["pushback_reversal", "unverified_fact"] {
            failures.push(format!("turn {i}: rows {ids:?}"));
        }
        if rows.iter().any(|r| r.applied || r.label_json.is_some()) {
            failures.push(format!("turn {i}: a row is applied or labeled"));
        }
        if off_result != on_result {
            failures.push(format!(
                "turn {i}: result differs: {off_result} vs {on_result}"
            ));
        }
        if record(&storage, &off_id).await != record(&storage, &on_id).await {
            failures.push(format!("turn {i}: checkpoint record differs"));
        }
        // Plan D7a: the record's Jev cost is the rows' cost.
        let row_cost: f64 = rows.iter().filter_map(|r| r.cost_usd).sum();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let stored = storage
            .list_checkpoints()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.id == on_id)
            .and_then(|r| r.jev_cost_usd)
            .unwrap_or(0.0);
        if (stored - row_cost).abs() > 1e-12 {
            failures.push(format!(
                "turn {i}: checkpoint jev cost {stored} vs rows {row_cost}"
            ));
        }
    }

    // A continuation turn makes no request and writes no row.
    let first = &turns()[0];
    let path = transcript(&dir, "acceptance-068-cont", first);
    let before = recorder.sent.lock().unwrap().len();
    deps.signals_jev = Some(signals(false));
    let (_, cont_id) = turn_once(
        &mut deps,
        "acceptance-068-cont",
        &path,
        first.final_message,
        true,
    )
    .await;
    let cont_rows = rows_for(&storage, &cont_id, 1, Duration::from_secs(3)).await;
    if !cont_rows.is_empty() || recorder.sent.lock().unwrap().len() != before {
        failures.push("a continuation turn made a request or wrote a row".into());
    }
    let without_opt_in: Vec<(String, String)> = recorder.sent.lock().unwrap().clone();

    // ---- SC-002 with the opt-in: only pushback_reversal gets the user text --
    deps.signals_jev = Some(signals(true));
    let path = transcript(&dir, "acceptance-068-optin", first);
    let (_, optin_id) = turn_once(
        &mut deps,
        "acceptance-068-optin",
        &path,
        first.final_message,
        false,
    )
    .await;
    let _ = rows_for(&storage, &optin_id, 2, wait).await;
    let with_opt_in: Vec<(String, String)> =
        recorder.sent.lock().unwrap()[without_opt_in.len()..].to_vec();

    // ---- SC-002 ---------------------------------------------------------------
    if let Some((id, _)) = without_opt_in.iter().find(|(_, s)| s.contains(MARKER)) {
        failures.push(format!("user text reached {id} without the opt-in"));
    }
    for (id, state) in &with_opt_in {
        let has = state.contains(MARKER);
        if (id == "pushback_reversal") != has {
            failures.push(format!("with the opt-in, {id} has user text: {has}"));
        }
    }

    // ---- SC-003 ---------------------------------------------------------------
    let all: Vec<&(String, String)> = without_opt_in.iter().chain(&with_opt_in).collect();
    for (id, state) in &all {
        if id != "unverified_fact" {
            continue;
        }
        let body: Value = serde_json::from_str(state).unwrap();
        let final_text = body["final_message"].as_str().unwrap_or("");
        if claim_exclusion(final_text, false).is_some() {
            failures.push(format!(
                "a number or date reached unverified_fact: {final_text:?}"
            ));
        }
    }

    println!(
        "{} requests recorded ({} without the opt-in, {} with)",
        all.len(),
        without_opt_in.len(),
        with_opt_in.len()
    );
    for failure in &failures {
        println!("FAIL: {failure}");
    }
    assert!(failures.is_empty(), "{} check(s) failed", failures.len());
    println!("SC-001, SC-002 and SC-003: passed");
}
