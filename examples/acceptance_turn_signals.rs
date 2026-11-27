//! Live acceptance for 066 (T029, SC-004): the turn signals on real code
//! paths, in-process, with no network.
//!
//! Each scrubbed fixture in `tests/fixtures/transcripts/` is read from disk by
//! the production transcript reader and run through `run_turn` with
//! `CHECKPOINT_TURN_SIGNALS` on, against a fresh scratch database. Asserts
//! one turn-signal row per turn end, joined to its checkpoint record by
//! `eval_id`, with the expected detector results, and that no sentinel string
//! from a fixture's prompt or tool results reaches any model request. A
//! failed assertion exits non-zero. The model client is a recorder that
//! refuses every call, so nothing leaves the machine.

#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

use mcp_parallax::checkpoint::contract::CheckpointTurnParams;
use mcp_parallax::checkpoint::review;
use mcp_parallax::checkpoint::run::{run_turn, CheckpointDeps};
use mcp_parallax::checkpoint::signals::TurnSignalsDeps;
use mcp_parallax::error::AppError;
use mcp_parallax::modes::ModeRegistry;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::client::{Completion, ModelClient};
use mcp_parallax::traits::clock::SystemClock;
use mcp_parallax::traits::trajectory::FsTrajectoryReader;
use serde_json::Value;
use std::sync::{Arc, Mutex};

const SESSION: &str = "fixture-session";
const SENTINELS: [&str; 2] = ["SENTINEL-USER-PROMPT-7f3a", "SENTINEL-TOOL-RESULT-91c2"];

/// Records every prompt and refuses the call, so a review that runs fails
/// open and nothing is sent.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

#[async_trait::async_trait]
impl ModelClient for Recorder {
    async fn complete(&self, prompt: &str, _schema: &Value) -> Result<Completion, AppError> {
        self.0.lock().unwrap().push(prompt.to_string());
        Err(AppError::Client(
            "the acceptance recorder sends nothing".into(),
        ))
    }
}

/// One fixture's expected row.
struct Expected {
    file: &'static str,
    turn_start_uuid: Option<&'static str>,
    final_uuid: Option<&'static str>,
    pushback: &'static str,
    unverified: &'static str,
}

const EXPECTED: [Expected; 5] = [
    Expected {
        file: "turn_string_prompt.jsonl",
        turn_start_uuid: Some("u01"),
        final_uuid: Some("a03"),
        pushback: "no previous final message",
        unverified: "fired",
    },
    Expected {
        file: "turn_block_prompt.jsonl",
        turn_start_uuid: Some("u01"),
        final_uuid: Some("a01"),
        pushback: "no previous final message",
        unverified: "not_fired",
    },
    Expected {
        file: "turn_slash_command.jsonl",
        turn_start_uuid: Some("u01"),
        final_uuid: Some("a01"),
        pushback: "no previous final message",
        unverified: "not_fired",
    },
    Expected {
        file: "turn_compacted.jsonl",
        turn_start_uuid: None,
        final_uuid: Some("a01"),
        pushback: "turn start not found",
        unverified: "turn start not found",
    },
    // 066 review finding 4: a reversal pair, so the review runs through the
    // real reader and the sentinel check has a request to inspect.
    Expected {
        file: "turn_reversal.jsonl",
        turn_start_uuid: Some("u01"),
        final_uuid: Some("a02"),
        pushback: "no previous final message",
        unverified: "not_fired",
    },
];

/// The last assistant text in a fixture: the turn's final message.
fn final_message(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v["type"] == "assistant")
        .filter_map(|v| {
            v["message"]["content"]
                .as_array()?
                .iter()
                .find(|b| b["type"] == "text")?["text"]
                .as_str()
                .map(str::to_string)
        })
        .next_back()
        .unwrap()
}

#[tokio::main]
async fn main() {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts");
    let scratch = tempfile::tempdir().unwrap();
    let db = scratch.path().join("turn-signals-acceptance.db");
    let storage = Arc::new(SqliteStorage::connect(&db.to_string_lossy()).await.unwrap());
    let recorder = Arc::new(Recorder::default());
    let mut registry = ModeRegistry::new();
    review::register(&mut registry).unwrap();

    for expected in &EXPECTED {
        // A fresh previous-final map per fixture: each is the first turn end
        // of its session.
        let deps = CheckpointDeps {
            reader: Arc::new(FsTrajectoryReader),
            storage: Arc::clone(&storage) as Arc<dyn mcp_parallax::traits::storage::Storage>,
            clock: Arc::new(SystemClock),
            model_client: Arc::clone(&recorder) as Arc<dyn ModelClient>,
            review_mode: registry.get(review::REVIEW_MODE_ID).unwrap().clone(),
            model: "claude-opus-4-8".into(),
            embedder: None,
            gate_extra_patterns: vec![],
            jev: None,
            turn_signals: Some(TurnSignalsDeps {
                previous_finals: Arc::default(),
                grounded_verify: true,
                research: false,
            }),
            skip_thresholds: None,
            fixed_eval_id: None,
            tool_jev: None,
        };
        let path = fixtures.join(expected.file);
        let params = CheckpointTurnParams {
            session_id: SESSION.into(),
            transcript_path: path.to_string_lossy().to_string(),
            final_message: final_message(&path),
            continuation: false,
        };
        run_turn(&deps, &params, None).await.unwrap();
    }

    let rows = storage.load_turn_signal_records().await.unwrap();
    let records = storage.list_checkpoints().await.unwrap();
    assert_eq!(rows.len(), EXPECTED.len(), "one row per turn end");
    assert_eq!(
        records.len(),
        EXPECTED.len(),
        "one checkpoint record per turn end"
    );
    for expected in &EXPECTED {
        let row = rows.iter().find(|r| {
            r.transcript_ref
                .as_deref()
                .is_some_and(|t| t.ends_with(expected.file))
        });
        assert!(row.is_some(), "no row for {}", expected.file);
        let row = row.unwrap();
        assert!(
            records.iter().any(|c| c.id == row.eval_id),
            "{}: row not joined to a checkpoint record",
            expected.file
        );
        assert_eq!(
            row.turn_start_uuid.as_deref(),
            expected.turn_start_uuid,
            "{}",
            expected.file
        );
        assert_eq!(
            row.final_uuid.as_deref(),
            expected.final_uuid,
            "{}",
            expected.file
        );
        assert_eq!(row.pushback_result, expected.pushback, "{}", expected.file);
        assert_eq!(
            row.unverified_result, expected.unverified,
            "{}",
            expected.file
        );
        println!(
            "{}: start {:?}, final {:?}, pushback {}, unverified {} ({} unmatched)",
            expected.file,
            row.turn_start_uuid,
            row.final_uuid,
            row.pushback_result,
            row.unverified_result,
            row.unmatched_entities.as_array().map_or(0, Vec::len)
        );
    }
    let prompts = recorder.0.lock().unwrap().clone();
    // The reversal fixture makes the review run, so the sentinel check below
    // inspects at least one real request (066 review finding 4).
    assert!(!prompts.is_empty(), "no model request was made");
    for prompt in &prompts {
        for sentinel in SENTINELS {
            assert!(
                !prompt.contains(sentinel),
                "{sentinel} reached a model request"
            );
        }
    }
    println!(
        "PASS: {} turn ends, {} joined rows, {} model requests attempted, no sentinel sent",
        EXPECTED.len(),
        rows.len(),
        prompts.len()
    );
}
