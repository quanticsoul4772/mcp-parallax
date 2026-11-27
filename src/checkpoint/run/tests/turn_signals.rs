//! 066 T016: the turn signals inside `run_turn`.

use super::*;
use crate::checkpoint::signals::{PreviousFinals, TurnSignalsDeps};
use crate::storage::turn_signals::TurnSignalRecord;
use std::sync::Mutex;

const PROMPT_SENTINEL: &str = "SENTINEL-USER-PROMPT-7f3a";
const TOOL_SENTINEL: &str = "SENTINEL-TOOL-RESULT-91c2";
const FIXTURE_FINAL: &str = "Version 1.2.3 is in src/lib.rs; the test failed at line 4096.";

fn fixture_lines() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transcripts/turn_string_prompt.jsonl");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn empty_window() -> TrajectoryWindow {
    TrajectoryWindow {
        session_id: "s1".into(),
        entries: vec![],
    }
}

fn signals_on(finals: &Arc<PreviousFinals>) -> TurnSignalsDeps {
    TurnSignalsDeps {
        previous_finals: Arc::clone(finals),
        grounded_verify: true,
        research: false,
    }
}

/// What a test captured from the storage mock.
type Captured<T> = Arc<Mutex<Vec<T>>>;

/// Capture every checkpoint record and every turn-signal row written.
fn capture(b: &mut DepsBuilder) -> (Captured<CheckpointRecord>, Captured<TurnSignalRecord>) {
    let records = Arc::new(Mutex::new(Vec::new()));
    let rows = Arc::new(Mutex::new(Vec::new()));
    let r = Arc::clone(&records);
    b.storage.expect_record_checkpoint().returning(move |rec| {
        r.lock().unwrap().push(rec.clone());
        Ok(())
    });
    let w = Arc::clone(&rows);
    b.storage
        .expect_record_turn_signals()
        .returning(move |row| {
            w.lock().unwrap().push(row.clone());
            Ok(())
        });
    (records, rows)
}

#[tokio::test]
async fn a_turn_without_review_writes_one_row_joined_to_its_checkpoint_record() {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((empty_window(), fixture_lines())));
    b.client.expect_complete().times(0);
    b.turn_signals = Some(signals_on(&Arc::new(PreviousFinals::default())));
    let (records, rows) = capture(&mut b);

    run_turn(&b.build(), &turn_params(FIXTURE_FINAL, false), None)
        .await
        .unwrap();

    let records = records.lock().unwrap();
    let rows = rows.lock().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].eval_id, records[0].id);
    assert_eq!(rows[0].turn_start_line, Some(2));
    assert_eq!(rows[0].final_line, Some(19));
    assert_eq!(rows[0].pushback_result, "no previous final message");
    // 1.2.3 is not in the turn's evidence (the tool saw 1.2.4); the path
    // and the number are.
    assert_eq!(rows[0].unverified_result, "fired");
    assert_eq!(
        rows[0].unmatched_entities.as_array().unwrap().len(),
        1,
        "{:?}",
        rows[0].unmatched_entities
    );
    assert_eq!(rows[0].unmatched_entities[0]["kind"], "version");
    assert_eq!(
        rows[0].recommendations["unverified_fact"],
        "grounded_verify"
    );
    assert_eq!(rows[0].harness_session_id.as_deref(), Some("s1"));
    assert_eq!(rows[0].transcript_ref.as_deref(), Some("t.jsonl"));
}

#[tokio::test]
async fn a_turn_whose_review_runs_also_writes_one_row() {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((reversal_window(), fixture_lines())));
    b.client.expect_complete().times(1).returning(|_, _| {
        Ok(Completion {
            value: cleared_review(),
            input_tokens: 80,
            output_tokens: 30,
            reported_cost_micros: None,
        })
    });
    b.storage
        .expect_delivered_signal_keys_since()
        .returning(|_, _| Ok(vec![]));
    b.turn_signals = Some(signals_on(&Arc::new(PreviousFinals::default())));
    let (records, rows) = capture(&mut b);

    run_turn(
        &b.build(),
        &turn_params(
            "After checking, the database migration is not reversible after all.",
            false,
        ),
        None,
    )
    .await
    .unwrap();

    assert!(records.lock().unwrap()[0].review_ran);
    assert_eq!(rows.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_continuation_turn_writes_no_row_but_updates_the_previous_final() {
    let finals = Arc::new(PreviousFinals::default());

    let mut first = DepsBuilder::new();
    first.reader.expect_read().times(0);
    first.client.expect_complete().times(0);
    first.turn_signals = Some(signals_on(&finals));
    let (_, rows) = capture(&mut first);
    run_turn(
        &first.build(),
        &turn_params(
            "The database migration is fully reversible and safe to run.",
            true,
        ),
        None,
    )
    .await
    .unwrap();
    assert!(rows.lock().unwrap().is_empty());

    // The next turn: a real prompt, no tool call, and a reversal of the
    // message the continuation turn ended with.
    let prompt = serde_json::json!({
        "type": "user", "sessionId": "s1", "isSidechain": false,
        "origin": { "kind": "human" },
        "message": { "role": "user", "content": "are you sure?" }
    })
    .to_string();
    let mut second = DepsBuilder::new();
    second
        .reader
        .expect_read()
        .returning(move |_, _| Ok((empty_window(), vec![prompt.clone()])));
    second.client.expect_complete().times(0);
    second.turn_signals = Some(signals_on(&finals));
    let (_, rows) = capture(&mut second);
    run_turn(
        &second.build(),
        &turn_params(
            "You are right, the database migration is not reversible after all.",
            false,
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(rows.lock().unwrap()[0].pushback_result, "fired");
}

#[tokio::test]
async fn an_unreadable_transcript_records_both_signals_as_unreadable() {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Err(AppError::ValidationFailure("no transcript".into())));
    b.turn_signals = Some(signals_on(&Arc::new(PreviousFinals::default())));
    let (records, rows) = capture(&mut b);

    run_turn(&b.build(), &turn_params(FIXTURE_FINAL, false), None)
        .await
        .unwrap();

    let rows = rows.lock().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].pushback_result, "transcript unreadable");
    assert_eq!(rows[0].unverified_result, "transcript unreadable");
    assert_eq!(rows[0].turn_start_line, None);
    assert_eq!(rows[0].eval_id, records.lock().unwrap()[0].id);
}

#[tokio::test]
async fn an_empty_final_message_writes_no_row() {
    let mut b = DepsBuilder::new();
    b.reader.expect_read().times(0);
    b.turn_signals = Some(signals_on(&Arc::new(PreviousFinals::default())));
    let (_, rows) = capture(&mut b);
    run_turn(&b.build(), &turn_params("   ", false), None)
        .await
        .unwrap();
    assert!(rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn with_the_switch_off_no_row_is_written() {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((empty_window(), fixture_lines())));
    b.client.expect_complete().times(0);
    b.storage.expect_record_turn_signals().times(0);
    b.storage
        .expect_record_checkpoint()
        .times(1)
        .returning(|_| Ok(()));
    run_turn(&b.build(), &turn_params(FIXTURE_FINAL, false), None)
        .await
        .unwrap();
}

// SC-001: the checkpoint result and record are identical with the signals on
// and off, except latency (and the record id, which is fresh per run).
#[tokio::test]
async fn the_result_and_record_are_identical_with_the_signals_on_and_off() {
    let mut outcomes = Vec::new();
    for on in [false, true] {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((reversal_window(), fixture_lines())));
        b.client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: cleared_review(),
                input_tokens: 80,
                output_tokens: 30,
                reported_cost_micros: None,
            })
        });
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(|_, _| Ok(vec![]));
        if on {
            b.turn_signals = Some(signals_on(&Arc::new(PreviousFinals::default())));
        }
        let (records, _) = capture(&mut b);
        let (mut result, inp, out) = run_turn(
            &b.build(),
            &turn_params(
                "After checking, the database migration is not reversible after all.",
                false,
            ),
            None,
        )
        .await
        .unwrap();
        result.latency_ms = 0;
        let mut record = records.lock().unwrap()[0].clone();
        record.id = String::new();
        record.latency_ms = 0;
        outcomes.push((serde_json::to_value(&result).unwrap(), record, inp, out));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

// SC-002: sentinel strings planted in the transcript's user prompt and tool
// results reach no request body.
#[tokio::test]
async fn no_sentinel_from_the_transcript_reaches_a_request_body() {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((reversal_window(), fixture_lines())));
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&prompts);
    b.client.expect_complete().returning(move |prompt, _| {
        seen.lock().unwrap().push(prompt.to_string());
        Ok(Completion {
            value: cleared_review(),
            input_tokens: 80,
            output_tokens: 30,
            reported_cost_micros: None,
        })
    });
    b.storage
        .expect_delivered_signal_keys_since()
        .returning(|_, _| Ok(vec![]));
    b.turn_signals = Some(signals_on(&Arc::new(PreviousFinals::default())));
    let _ = capture(&mut b);
    run_turn(
        &b.build(),
        &turn_params(
            "After checking, the database migration is not reversible after all.",
            false,
        ),
        None,
    )
    .await
    .unwrap();
    let prompts = prompts.lock().unwrap();
    assert!(!prompts.is_empty());
    for prompt in prompts.iter() {
        assert!(!prompt.contains(PROMPT_SENTINEL) && !prompt.contains(TOOL_SENTINEL));
    }
}

fn cleared_review() -> serde_json::Value {
    json!({
        "contradicts": false, "statement_a": "", "statement_b": "",
        "basis": "No conflict.", "violates": false, "violated_preference": "",
        "violation_basis": "", "capture_worthy": false, "capture_kind": "none",
        "capture_content": "", "capture_basis": ""
    })
}

/// Records every Jev request and answers every question at 0.5.
struct RecordingJev(Arc<Mutex<Vec<crate::traits::classifier::JevRequest>>>);

#[async_trait::async_trait]
impl crate::traits::classifier::Classifier for RecordingJev {
    async fn classify(
        &self,
        request: &crate::traits::classifier::JevRequest,
    ) -> Result<crate::traits::classifier::JevResponse, AppError> {
        self.0.lock().unwrap().push(request.clone());
        Ok(crate::traits::classifier::JevResponse {
            model_answered: "jev-1.14.0".into(),
            answers: request
                .questions
                .iter()
                .map(|q| {
                    (
                        q.id.clone(),
                        crate::traits::classifier::JevAnswer::Noul { p: 0.5 },
                    )
                })
                .collect(),
            input_tokens: Some(100),
            request_id: None,
            latency_ms: 10,
        })
    }
    fn model_id(&self) -> &'static str {
        "jev-1.14.0"
    }
}

// SC-002 in Jev mode (066 analyze finding 1): with the signals on and the
// 064 turn shadow running, no fixture sentinel reaches the Jev request.
#[tokio::test]
async fn no_sentinel_from_the_transcript_reaches_the_jev_request() {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((reversal_window(), fixture_lines())));
    b.client.expect_complete().returning(|_, _| {
        Ok(Completion {
            value: cleared_review(),
            input_tokens: 80,
            output_tokens: 30,
            reported_cost_micros: None,
        })
    });
    b.storage
        .expect_delivered_signal_keys_since()
        .returning(|_, _| Ok(vec![]));
    b.storage.expect_record_jev().returning(|_| Ok(()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    b.jev = Some(crate::jev::shadow::TurnJev {
        classifier: Arc::new(RecordingJev(Arc::clone(&requests))),
        redactor: crate::jev::redact::Redactor::new(vec![]),
        model: "jev-1.14.0".into(),
        server_session_id: "server-1".into(),
        deadline: std::time::Duration::from_millis(500),
    });
    b.turn_signals = Some(signals_on(&Arc::new(PreviousFinals::default())));
    let _ = capture(&mut b);
    run_turn(
        &b.build(),
        &turn_params(
            "After checking, the database migration is not reversible after all.",
            false,
        ),
        None,
    )
    .await
    .unwrap();
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "the turn shadow asked Jev once");
    let body = format!("{} {:?}", requests[0].state.as_str(), requests[0].questions);
    assert!(
        !body.contains(PROMPT_SENTINEL) && !body.contains(TOOL_SENTINEL),
        "{body}"
    );
}
