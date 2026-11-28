//! 068 T012: `run_turn` with the turn-signal site (SC-001, User Story 1
//! scenarios 1, 3 and 4; plan D1a, D7a).

use super::*;
use crate::client::typesafe::JEV_TURN_DEADLINE_MS;
use crate::jev::redact::Redactor;
use crate::jev::shadow::SignalsJev;
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::{Classifier, JevAnswer, JevRequest, JevResponse, JevSite};
use std::sync::Mutex;
use std::time::Duration as StdDuration;

const PINNED: &str = "jev-1.14.0";
/// A final message that mines no statement pair from `reversal_window`, so
/// the review does not run (memory off) and the turn stays silent.
const FINAL: &str = "The tests pass. It has 4096 entries.";

/// Jev answering every question yes at 0.9, as `model`.
struct Answers(&'static str, Arc<Mutex<Vec<JevRequest>>>);

#[async_trait::async_trait]
impl Classifier for Answers {
    async fn classify(&self, request: &JevRequest) -> Result<JevResponse, AppError> {
        self.1.lock().unwrap().push(request.clone());
        Ok(JevResponse {
            model_answered: self.0.into(),
            answers: request
                .questions
                .iter()
                .map(|q| (q.id.clone(), JevAnswer::Noul { p: 0.9 }))
                .collect(),
            input_tokens: Some(40),
            request_id: Some("req-1".into()),
            latency_ms: 90,
        })
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

/// Jev that never answers.
struct Hanging;

#[async_trait::async_trait]
impl Classifier for Hanging {
    async fn classify(&self, _: &JevRequest) -> Result<JevResponse, AppError> {
        std::future::pending().await
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

/// Jev that fails every request.
struct Failing(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait::async_trait]
impl Classifier for Failing {
    async fn classify(&self, _: &JevRequest) -> Result<JevResponse, AppError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(AppError::ClassifierProvider("test".into()))
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

fn signals_jev(classifier: Arc<dyn Classifier>, deadline: StdDuration) -> Arc<SignalsJev> {
    Arc::new(SignalsJev {
        classifier,
        redactor: Redactor::new(vec![]),
        model: PINNED.into(),
        server_session_id: "server-1".into(),
        deadline,
        send_user_text: false,
        grounded_verify: false,
        research: false,
    })
}

type Rows = Arc<Mutex<Vec<JevRecord>>>;
type Costs = Arc<Mutex<Vec<(String, f64)>>>;
type Records = Arc<Mutex<Vec<CheckpointRecord>>>;

/// A builder that reads `reversal_window`, records every checkpoint record,
/// Jev row and checkpoint Jev-cost addition.
fn capturing(rows: &Rows, costs: &Costs, records: &Records) -> DepsBuilder {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((reversal_window(), vec![])));
    let records = Arc::clone(records);
    b.storage.expect_record_checkpoint().returning(move |r| {
        records.lock().unwrap().push(r.clone());
        Ok(())
    });
    let written = Arc::clone(rows);
    b.storage.expect_record_jev().returning(move |r| {
        written.lock().unwrap().extend_from_slice(r);
        Ok(())
    });
    b.storage
        .expect_turn_signal_yes_kinds_since()
        .returning(|_, _| Ok(vec![]));
    let costs = Arc::clone(costs);
    b.storage
        .expect_add_checkpoint_jev_cost()
        .returning(move |id, cost| {
            costs.lock().unwrap().push((id.to_string(), cost));
            Ok(())
        });
    b
}

/// Wait for the off-path task to write `n` rows.
async fn rows_after(rows: &Rows, n: usize) -> Vec<JevRecord> {
    for _ in 0..200 {
        if rows.lock().unwrap().len() >= n {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    rows.lock().unwrap().clone()
}

#[tokio::test]
async fn each_non_continuation_turn_end_writes_one_row_per_question() {
    let (rows, costs, records) = (Rows::default(), Costs::default(), Records::default());
    let requests = Arc::new(Mutex::new(vec![]));
    let mut b = capturing(&rows, &costs, &records);
    b.signals_jev = Some(signals_jev(
        Arc::new(Answers(PINNED, Arc::clone(&requests))),
        StdDuration::from_millis(JEV_TURN_DEADLINE_MS),
    ));
    let (result, _, _) = run_turn(&b.build(), &turn_params(FINAL, false), None)
        .await
        .unwrap();
    assert_eq!(result.verdict, Verdict::Silence);
    let rows = rows_after(&rows, 2).await;
    assert_eq!(rows.len(), 2);
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r.questions_json[0]["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"pushback_reversal") && ids.contains(&"unverified_fact"));
    for row in &rows {
        assert_eq!(row.site, JevSite::TurnSignals);
        assert_eq!(
            row.outcome,
            JevOutcome::Call(crate::error::Outcome::Success)
        );
        assert_eq!(
            row.eval_id.as_deref(),
            Some(records.lock().unwrap()[0].id.as_str())
        );
        assert!(!row.applied);
        assert!(row.label_json.is_none());
    }
    // One request per question, one attempt, the named deadline.
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.questions.len(), 1);
        assert!(!request.retry);
        assert_eq!(
            request.timeout,
            StdDuration::from_millis(JEV_TURN_DEADLINE_MS)
        );
    }
}

#[tokio::test]
async fn a_never_returning_jev_does_not_delay_the_turn_response() {
    let (rows, costs, records) = (Rows::default(), Costs::default(), Records::default());
    let mut b = capturing(&rows, &costs, &records);
    b.signals_jev = Some(signals_jev(
        Arc::new(Hanging),
        StdDuration::from_millis(JEV_TURN_DEADLINE_MS),
    ));
    let deps = b.build();
    let params = turn_params(FINAL, false);
    // The Stop hook's response never awaits the requests (plan D1a).
    tokio::time::timeout(
        StdDuration::from_millis(JEV_TURN_DEADLINE_MS / 2),
        run_turn(&deps, &params, None),
    )
    .await
    .expect("the turn answered without waiting for Jev")
    .unwrap();
}

#[tokio::test]
async fn timeouts_errors_and_wrong_model_answers_are_recorded_only() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cases: [(Arc<dyn Classifier>, crate::error::Outcome); 3] = [
        (Arc::new(Hanging), crate::error::Outcome::Timeout),
        (
            Arc::new(Failing(Arc::clone(&calls))),
            crate::error::Outcome::ClassifierProvider,
        ),
        (
            Arc::new(Answers("jev-9.9.9", Arc::default())),
            crate::error::Outcome::ValidationFailure,
        ),
    ];
    for (classifier, outcome) in cases {
        let (rows, costs, records) = (Rows::default(), Costs::default(), Records::default());
        let mut b = capturing(&rows, &costs, &records);
        b.signals_jev = Some(signals_jev(classifier, StdDuration::from_millis(50)));
        let (result, _, _) = run_turn(&b.build(), &turn_params(FINAL, false), None)
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::Silence, "{outcome:?}");
        let rows = rows_after(&rows, 2).await;
        assert_eq!(rows.len(), 2, "{outcome:?}");
        assert!(
            rows.iter().all(|r| r.outcome == JevOutcome::Call(outcome)),
            "{outcome:?}: {rows:?}"
        );
    }
}

#[tokio::test]
async fn continuation_turns_make_no_request_and_write_no_row() {
    let (rows, costs, records) = (Rows::default(), Costs::default(), Records::default());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut b = capturing(&rows, &costs, &records);
    b.signals_jev = Some(signals_jev(
        Arc::new(Failing(Arc::clone(&calls))),
        StdDuration::from_millis(JEV_TURN_DEADLINE_MS),
    ));
    let (result, _, _) = run_turn(&b.build(), &turn_params(FINAL, true), None)
        .await
        .unwrap();
    assert_eq!(result.verdict, Verdict::Silence);
    tokio::time::sleep(StdDuration::from_millis(100)).await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn with_the_site_off_no_request_is_made_and_no_row_is_written() {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((reversal_window(), vec![])));
    b.storage.expect_record_checkpoint().returning(|_| Ok(()));
    b.storage.expect_record_jev().times(0);
    b.storage.expect_add_checkpoint_jev_cost().times(0);
    run_turn(&b.build(), &turn_params(FINAL, false), None)
        .await
        .unwrap();
    tokio::time::sleep(StdDuration::from_millis(100)).await;
}

#[tokio::test]
async fn the_checkpoint_jev_cost_gains_each_rows_cost() {
    let (rows, costs, records) = (Rows::default(), Costs::default(), Records::default());
    let mut b = capturing(&rows, &costs, &records);
    b.fixed_eval_id = Some("eval-68".into());
    b.signals_jev = Some(signals_jev(
        Arc::new(Answers(PINNED, Arc::default())),
        StdDuration::from_millis(JEV_TURN_DEADLINE_MS),
    ));
    run_turn(&b.build(), &turn_params(FINAL, false), None)
        .await
        .unwrap();
    let rows = rows_after(&rows, 2).await;
    for _ in 0..200 {
        if costs.lock().unwrap().len() >= 2 {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    let costs = costs.lock().unwrap().clone();
    assert_eq!(costs.len(), 2);
    for (row, (eval_id, cost)) in rows.iter().zip(&costs) {
        assert_eq!(eval_id, "eval-68");
        assert!((cost - row.cost_usd.unwrap()).abs() < 1e-12);
    }
}

/// The turn's result (minus its latency) and checkpoint record (minus id,
/// latency and timestamp), with the site off or in shadow.
async fn turn_outcome(
    shadow: Option<Arc<dyn Classifier>>,
) -> (serde_json::Value, CheckpointRecord) {
    let (rows, costs, records) = (Rows::default(), Costs::default(), Records::default());
    let mut b = capturing(&rows, &costs, &records);
    b.signals_jev = shadow.map(|c| signals_jev(c, StdDuration::from_millis(50)));
    let (result, _, _) = run_turn(&b.build(), &turn_params(FINAL, false), None)
        .await
        .unwrap();
    let mut result = serde_json::to_value(&result).unwrap();
    result.as_object_mut().unwrap().remove("latency_ms");
    let mut record = records.lock().unwrap()[0].clone();
    record.id = String::new();
    record.latency_ms = 0;
    record.created_at = fixed_now();
    (result, record)
}

#[tokio::test]
async fn the_checkpoint_result_and_record_equal_the_site_off_run() {
    let off = turn_outcome(None).await;
    let shadows: [Arc<dyn Classifier>; 3] = [
        Arc::new(Answers(PINNED, Arc::default())),
        Arc::new(Hanging),
        Arc::new(Failing(Arc::default())),
    ];
    for classifier in shadows {
        assert_eq!(turn_outcome(Some(classifier)).await, off);
    }
}

/// Jev answering every question at probability `0`.
struct AnswersAt(f64);

#[async_trait::async_trait]
impl Classifier for AnswersAt {
    async fn classify(&self, request: &JevRequest) -> Result<JevResponse, AppError> {
        Ok(JevResponse {
            model_answered: PINNED.into(),
            answers: request
                .questions
                .iter()
                .map(|q| (q.id.clone(), JevAnswer::Noul { p: self.0 }))
                .collect(),
            input_tokens: Some(40),
            request_id: Some("req-1".into()),
            latency_ms: 90,
        })
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

/// The two rows' `cooldown_suppressed`, by question id, when the storage
/// lookup reports `earlier` yes kinds for session `for_session` (and none
/// for any other), with Jev answering at `p`.
async fn suppressed(
    earlier: &'static [&'static str],
    for_session: &'static str,
    p: f64,
) -> Vec<(String, Option<Vec<String>>)> {
    let rows = Rows::default();
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((reversal_window(), vec![])));
    b.storage.expect_record_checkpoint().returning(|_| Ok(()));
    let written = Arc::clone(&rows);
    b.storage.expect_record_jev().returning(move |r| {
        written.lock().unwrap().extend_from_slice(r);
        Ok(())
    });
    b.storage
        .expect_add_checkpoint_jev_cost()
        .returning(|_, _| Ok(()));
    b.storage
        .expect_turn_signal_yes_kinds_since()
        .returning(move |session, since| {
            assert_eq!(
                since,
                fixed_now() - Duration::milliseconds(COOLDOWN_WINDOW_MS)
            );
            Ok(if session == for_session {
                earlier.iter().map(|k| (*k).to_string()).collect()
            } else {
                vec![]
            })
        });
    b.signals_jev = Some(signals_jev(
        Arc::new(AnswersAt(p)),
        StdDuration::from_millis(JEV_TURN_DEADLINE_MS),
    ));
    run_turn(&b.build(), &turn_params(FINAL, false), None)
        .await
        .unwrap();
    rows_after(&rows, 2)
        .await
        .into_iter()
        .map(|r| {
            (
                r.questions_json[0]["id"].as_str().unwrap().to_string(),
                r.cooldown_suppressed,
            )
        })
        .collect()
}

fn kinds_of(rows: &[(String, Option<Vec<String>>)], id: &str) -> Option<Vec<String>> {
    rows.iter().find(|(q, _)| q == id).unwrap().1.clone()
}

// 068 T014 (FR-007, plan D3): a yes after a yes for the same session and
// kind inside the window lists that kind; the other kind lists nothing.
#[tokio::test]
async fn a_second_yes_for_the_same_session_and_kind_lists_that_kind() {
    let rows = suppressed(&["pushback_reversal"], "s1", 0.9).await;
    assert_eq!(
        kinds_of(&rows, "pushback_reversal"),
        Some(vec!["pushback_reversal".to_string()])
    );
    assert_eq!(kinds_of(&rows, "unverified_fact"), Some(vec![]));
}

// 068 T014: an earlier yes in a different session lists nothing.
#[tokio::test]
async fn an_earlier_yes_in_another_session_lists_nothing() {
    let rows = suppressed(&["pushback_reversal", "unverified_fact"], "other", 0.9).await;
    assert_eq!(kinds_of(&rows, "pushback_reversal"), Some(vec![]));
    assert_eq!(kinds_of(&rows, "unverified_fact"), Some(vec![]));
}

// 068 T014 (plan D3 note): an answer below 0.5 is not a yes, so it would not
// have been suppressed.
#[tokio::test]
async fn an_answer_below_one_half_is_not_listed() {
    let rows = suppressed(&["pushback_reversal", "unverified_fact"], "s1", 0.3).await;
    assert_eq!(kinds_of(&rows, "pushback_reversal"), Some(vec![]));
    assert_eq!(kinds_of(&rows, "unverified_fact"), Some(vec![]));
}

/// Jev that records each request's first question id and state, then fails.
struct RecordingStates(Arc<Mutex<Vec<(String, String)>>>);

#[async_trait::async_trait]
impl Classifier for RecordingStates {
    async fn classify(&self, request: &JevRequest) -> Result<JevResponse, AppError> {
        self.0.lock().unwrap().push((
            request.questions[0].id.clone(),
            request.state.as_str().to_string(),
        ));
        Err(AppError::ClassifierProvider("test".into()))
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

/// The states sent when the transcript lines hold a human prompt carrying
/// `marker`, with the opt-in on or off.
async fn states_with_prompt(marker: &'static str, send_user_text: bool) -> Vec<(String, String)> {
    let rows = Rows::default();
    let sent = Arc::new(Mutex::new(vec![]));
    let mut b = DepsBuilder::new();
    let prompt = json!({
        "type": "user", "sessionId": "s1", "origin": { "kind": "human" },
        "message": { "role": "user", "content": format!("{marker} that is wrong") }
    })
    .to_string();
    b.reader
        .expect_read()
        .returning(move |_, _| Ok((reversal_window(), vec![prompt.clone()])));
    b.storage.expect_record_checkpoint().returning(|_| Ok(()));
    let written = Arc::clone(&rows);
    b.storage.expect_record_jev().returning(move |r| {
        written.lock().unwrap().extend_from_slice(r);
        Ok(())
    });
    b.storage
        .expect_turn_signal_yes_kinds_since()
        .returning(|_, _| Ok(vec![]));
    b.storage
        .expect_add_checkpoint_jev_cost()
        .returning(|_, _| Ok(()));
    b.signals_jev = Some(Arc::new(SignalsJev {
        classifier: Arc::new(RecordingStates(Arc::clone(&sent))),
        redactor: Redactor::new(vec![]),
        model: PINNED.into(),
        server_session_id: "server-1".into(),
        deadline: StdDuration::from_millis(JEV_TURN_DEADLINE_MS),
        send_user_text,
        grounded_verify: false,
        research: false,
    }));
    run_turn(&b.build(), &turn_params(FINAL, false), None)
        .await
        .unwrap();
    rows_after(&rows, 2).await;
    let sent = sent.lock().unwrap().clone();
    sent
}

// 068 (T019 follow-up): run_turn reads that turn's user prompt from the
// transcript lines; with the opt-in it reaches pushback_reversal only, and
// without it neither request.
#[tokio::test]
async fn the_turns_user_prompt_from_the_transcript_reaches_pushback_reversal_only_with_the_opt_in()
{
    // Under 20 characters, so the redactor's high-entropy rule leaves it.
    const MARKER: &str = "PROMPT-MARK-68";
    let with = states_with_prompt(MARKER, true).await;
    assert_eq!(with.len(), 2);
    for (id, state) in &with {
        assert_eq!(
            state.contains(MARKER),
            id == "pushback_reversal",
            "{id}: {state}"
        );
    }
    let without = states_with_prompt(MARKER, false).await;
    assert_eq!(without.len(), 2);
    assert!(without.iter().all(|(_, state)| !state.contains(MARKER)));
}

/// A review model that never answers, so the turn's evaluation is still
/// running when the call is cancelled.
struct PendingReview;

#[async_trait::async_trait]
impl ModelClient for PendingReview {
    async fn complete(&self, _: &str, _: &serde_json::Value) -> Result<Completion, AppError> {
        std::future::pending().await
    }
}

// The turn-signal requests start right after the transcript read, so a
// turn call cancelled during the slow review (Claude Code cancels the hook
// when the next message arrives) still writes both rows.
#[tokio::test]
async fn turn_signal_rows_are_written_even_when_the_turn_call_is_cancelled() {
    let (rows, costs, records) = (Rows::default(), Costs::default(), Records::default());
    let mut b = capturing(&rows, &costs, &records);
    b.signals_jev = Some(signals_jev(
        Arc::new(Answers(PINNED, Arc::default())),
        StdDuration::from_millis(JEV_TURN_DEADLINE_MS),
    ));
    let mut deps = b.build();
    deps.model_client = Arc::new(PendingReview);
    // Contradicts the window's statement, so the review runs and hangs.
    let params = turn_params("The database migration is not reversible.", false);
    let cancelled = tokio::time::timeout(
        StdDuration::from_millis(300),
        run_turn(&deps, &params, None),
    )
    .await;
    assert!(
        cancelled.is_err(),
        "the review was expected to still be running"
    );
    let rows = rows_after(&rows, 2).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(records.lock().unwrap().is_empty());
    // No record was written, so no cost was added to one (068 review finding 5).
    assert!(costs.lock().unwrap().is_empty());
}

/// A review model that fails after a delay, so the turn-signal rows land
/// before the (fail-open) checkpoint record is written.
struct SlowFailingReview;

#[async_trait::async_trait]
impl ModelClient for SlowFailingReview {
    async fn complete(&self, _: &str, _: &serde_json::Value) -> Result<Completion, AppError> {
        tokio::time::sleep(StdDuration::from_millis(200)).await;
        Err(AppError::Client("review failed".into()))
    }
}

// 068 review finding 5: when the rows land before the record, run_turn adds
// their costs right after writing the record, once each.
#[tokio::test]
async fn costs_of_rows_that_land_before_the_record_are_added_after_it() {
    let (rows, costs, records) = (Rows::default(), Costs::default(), Records::default());
    let mut b = capturing(&rows, &costs, &records);
    b.fixed_eval_id = Some("eval-68".into());
    b.signals_jev = Some(signals_jev(
        Arc::new(Answers(PINNED, Arc::default())),
        StdDuration::from_millis(JEV_TURN_DEADLINE_MS),
    ));
    let mut deps = b.build();
    deps.model_client = Arc::new(SlowFailingReview);
    let params = turn_params("The database migration is not reversible.", false);
    let landed = Arc::clone(&rows);
    let early = tokio::spawn(async move { rows_after(&landed, 2).await });
    run_turn(&deps, &params, None).await.unwrap();
    let rows = early.await.unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(records.lock().unwrap().len(), 1);
    let costs = costs.lock().unwrap().clone();
    assert_eq!(costs.len(), 2, "{costs:?}");
    for (row, (eval_id, cost)) in rows.iter().zip(&costs) {
        assert_eq!(eval_id, "eval-68");
        assert!((cost - row.cost_usd.unwrap()).abs() < 1e-12);
    }
}
