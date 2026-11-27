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
