//! 066 T022: the end-of-turn review skip inside `run_turn`, enabled only by
//! injected thresholds (production has none in 066).

use super::*;
use crate::checkpoint::ReviewSkippedBy;
use crate::jev::policy::skip::{in_audit_share, ThresholdEntry};
use crate::jev::redact::Redactor;
use crate::jev::shadow::TurnJev;
use crate::storage::jev::JevRecord;
use crate::traits::classifier::{Classifier, JevAnswer, JevRequest, JevResponse};
use std::sync::Mutex;
use std::time::Duration as StdDuration;

const PINNED: &str = "jev-1.14.0";
const FINAL: &str = "Everything completed without surprises today.";

type Shared<T> = Arc<Mutex<Vec<T>>>;

/// A Jev that answers every question with `p`.
struct Answers(f64);

#[async_trait::async_trait]
impl Classifier for Answers {
    async fn classify(&self, request: &JevRequest) -> Result<JevResponse, AppError> {
        Ok(JevResponse {
            model_answered: PINNED.into(),
            answers: request
                .questions
                .iter()
                .map(|q| (q.id.clone(), JevAnswer::Noul { p: self.0 }))
                .collect(),
            input_tokens: Some(200),
            request_id: Some("req-1".into()),
            latency_ms: 90,
        })
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

/// A Jev that fails every call.
struct Failing;

#[async_trait::async_trait]
impl Classifier for Failing {
    async fn classify(&self, _: &JevRequest) -> Result<JevResponse, AppError> {
        Err(AppError::ClassifierProvider("typesafe 529".into()))
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

/// A Jev that never answers.
struct Hanging;

#[async_trait::async_trait]
impl Classifier for Hanging {
    async fn classify(&self, _: &JevRequest) -> Result<JevResponse, AppError> {
        tokio::time::sleep(StdDuration::from_secs(3600)).await;
        Err(AppError::ClassifierProvider("unreachable".into()))
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

fn thresholds() -> ThresholdEntry {
    ThresholdEntry {
        jev_model: PINNED,
        big_model: "claude-opus-4-8",
        question_version: 2,
        contradiction_no: 0.1,
        capture_worthy_no: 0.1,
    }
}

fn eval_id(audited: bool) -> String {
    (0..10_000)
        .map(|n| format!("eval-{n}"))
        .find(|id| in_audit_share(id) == audited)
        .unwrap()
}

fn cleared() -> Completion {
    Completion {
        value: json!({
            "contradicts": false, "statement_a": "", "statement_b": "",
            "basis": "No conflict.", "violates": false, "violated_preference": "",
            "violation_basis": "", "capture_worthy": false, "capture_kind": "none",
            "capture_content": "", "capture_basis": ""
        }),
        input_tokens: 80,
        output_tokens: 30,
        reported_cost_micros: None,
    }
}

struct Turn {
    checkpoints: Shared<CheckpointRecord>,
    rows: Shared<JevRecord>,
}

/// A memory-configured turn with no candidates, Jev beside it, and the given
/// classifier, thresholds and evaluation id.
fn turn(
    classifier: Arc<dyn Classifier>,
    thresholds: Option<ThresholdEntry>,
    eval_id: &str,
    reviews: usize,
    memory: bool,
) -> (DepsBuilder, Turn) {
    let mut b = DepsBuilder::new();
    b.reader
        .expect_read()
        .returning(|_, _| Ok((empty_window(), vec![])));
    if memory {
        b.embedder = Some(Arc::new(matching_embedder()));
        b.storage.expect_load_memories().returning(|| Ok(vec![]));
    }
    b.client
        .expect_complete()
        .times(reviews)
        .returning(|_, _| Ok(cleared()));
    b.storage
        .expect_delivered_signal_keys_since()
        .returning(|_, _| Ok(vec![]));
    let written = Turn {
        checkpoints: Arc::default(),
        rows: Arc::default(),
    };
    let checkpoints = Arc::clone(&written.checkpoints);
    b.storage.expect_record_checkpoint().returning(move |r| {
        checkpoints.lock().unwrap().push(r.clone());
        Ok(())
    });
    let rows = Arc::clone(&written.rows);
    b.storage.expect_record_jev().returning(move |batch| {
        rows.lock().unwrap().extend(batch.iter().cloned());
        Ok(())
    });
    b.jev = Some(TurnJev {
        classifier,
        redactor: Redactor::new(vec![]),
        model: PINNED.into(),
        server_session_id: "server-1".into(),
        deadline: StdDuration::from_millis(50),
    });
    b.skip_thresholds = thresholds;
    b.fixed_eval_id = Some(eval_id.to_string());
    (b, written)
}

async fn run(b: DepsBuilder) {
    run_turn(&b.build(), &turn_params(FINAL, false), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_confident_no_outside_the_audit_share_skips_the_review() {
    let (b, w) = turn(
        Arc::new(Answers(0.01)),
        Some(thresholds()),
        &eval_id(false),
        0,
        true,
    );
    run(b).await;
    let record = w.checkpoints.lock().unwrap()[0].clone();
    assert_eq!(record.review_skipped_by, Some(ReviewSkippedBy::Jev));
    assert!(!record.review_ran);
    assert_eq!(record.verdict, Verdict::Silence);
    let rows = w.rows.lock().unwrap().clone();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].applied, "a skipped turn's Jev row is applied");
}

#[tokio::test]
async fn a_turn_in_the_audit_share_gets_the_review_anyway() {
    let (b, w) = turn(
        Arc::new(Answers(0.01)),
        Some(thresholds()),
        &eval_id(true),
        1,
        true,
    );
    run(b).await;
    let record = w.checkpoints.lock().unwrap()[0].clone();
    assert_eq!(record.review_skipped_by, Some(ReviewSkippedBy::Audit));
    assert!(record.review_ran);
    assert!(!w.rows.lock().unwrap()[0].applied);
}

#[tokio::test]
async fn an_answer_above_its_threshold_runs_the_review() {
    let (b, w) = turn(
        Arc::new(Answers(0.9)),
        Some(thresholds()),
        &eval_id(false),
        1,
        true,
    );
    run(b).await;
    let record = w.checkpoints.lock().unwrap()[0].clone();
    assert_eq!(record.review_skipped_by, None);
    assert!(record.review_ran);
}

#[tokio::test]
async fn a_failed_jev_call_runs_the_review() {
    let (b, w) = turn(
        Arc::new(Failing),
        Some(thresholds()),
        &eval_id(false),
        1,
        true,
    );
    run(b).await;
    assert_eq!(w.checkpoints.lock().unwrap()[0].review_skipped_by, None);
}

#[tokio::test]
async fn without_memory_the_review_is_never_skipped() {
    // No memory means no capture question and the 006-era gate: with no
    // candidates the review does not run at all, and nothing is skipped.
    let (b, w) = turn(
        Arc::new(Answers(0.01)),
        Some(thresholds()),
        &eval_id(false),
        0,
        false,
    );
    run(b).await;
    let record = w.checkpoints.lock().unwrap()[0].clone();
    assert_eq!(record.review_skipped_by, None);
    assert!(!record.review_ran);
}

#[tokio::test]
async fn without_thresholds_the_review_runs_beside_jev_as_before() {
    let (b, w) = turn(Arc::new(Answers(0.01)), None, &eval_id(false), 1, true);
    run(b).await;
    let record = w.checkpoints.lock().unwrap()[0].clone();
    assert_eq!(record.review_skipped_by, None);
    assert!(record.review_ran);
    assert!(!w.rows.lock().unwrap()[0].applied);
}

// SC-005: with the skip enabled, the time Jev adds before a review it does
// not skip is at most the turn deadline.
#[tokio::test]
async fn a_hanging_jev_delays_the_review_by_at_most_its_deadline() {
    let (b, w) = turn(
        Arc::new(Hanging),
        Some(thresholds()),
        &eval_id(false),
        1,
        true,
    );
    let started = std::time::Instant::now();
    run(b).await;
    assert!(
        started.elapsed() < StdDuration::from_millis(crate::client::typesafe::JEV_TURN_DEADLINE_MS)
    );
    assert!(w.checkpoints.lock().unwrap()[0].review_ran);
}
