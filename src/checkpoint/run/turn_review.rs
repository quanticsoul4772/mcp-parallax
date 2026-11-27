//! The end-of-turn review hop beside Jev (064) and the 066 skip path, moved
//! out of `run.rs` (066 review finding 15).

use super::CheckpointDeps;
use crate::checkpoint::contract::CheckpointTurnParams;
use crate::checkpoint::{preference, review, ReviewSkippedBy, Signal, SignalKind};
use crate::error::AppError;
use crate::jev::policy::skip::{skip_decision, SkipDecision, SkipInput};
use std::time::Instant;

/// What one review hop returns: flags, capture proposal, tokens in and out,
/// and the raw answers.
pub(super) type ReviewOutput = (
    Vec<(Signal, String)>,
    Option<review::CaptureProposal>,
    u64,
    u64,
    review::ReviewRaw,
);

/// 064: run the review hop and, when the turn site is in shadow, Jev beside
/// it (joined, never instead of it). The Jev row is built whatever either
/// does; its label comes from the review's raw answers when the review
/// succeeded.
pub(super) async fn review_beside_jev(
    deps: &CheckpointDeps,
    params: &CheckpointTurnParams,
    eval_id: &str,
    candidates: &[review::Candidate],
    preferences: &[preference::PreferenceCandidate],
    window: &crate::checkpoint::trajectory::TrajectoryWindow,
    capture_enabled: bool,
) -> (Result<ReviewOutput, AppError>, Option<TurnJevOutcome>) {
    let activity = review::turn_activity(window);
    let review = review::review_once(
        deps.model_client.as_ref(),
        &deps.review_mode,
        candidates,
        preferences,
        &params.final_message,
        &activity,
        capture_enabled,
    );
    let Some(jev) = &deps.jev else {
        return (review.await, None);
    };
    let shadow = Box::pin(crate::jev::shadow::shadow_turn(
        jev,
        crate::jev::shadow::TurnInputs {
            window,
            activity: &activity,
            final_message: &params.final_message,
            capture_enabled,
        },
        crate::jev::shadow::TurnIds {
            eval_id,
            harness_session_id: &params.session_id,
            transcript_path: &params.transcript_path,
            now: deps.clock.now(),
        },
    ));
    // Each side notes when it finished, so the wait beyond the review is known.
    let review = async {
        let result = review.await;
        (result, Instant::now())
    };
    let shadow = async {
        let row = shadow.await;
        (row, Instant::now())
    };
    let ((review, review_done), (mut row, jev_done)) = tokio::join!(review, shadow);
    let wait_ms = u64::try_from(jev_done.saturating_duration_since(review_done).as_millis())
        .unwrap_or(u64::MAX);
    label_turn_row(deps, &mut row, &review, candidates, capture_enabled);
    (
        review,
        Some(TurnJevOutcome {
            row: Box::new(row),
            wait_ms,
        }),
    )
}

/// The stored P(yes) for question `id` in a Jev row's questions.
fn stored_answer(questions: &serde_json::Value, id: &str) -> Option<f64> {
    questions.as_array()?.iter().find(|q| q["id"] == id)?["answer"].as_f64()
}

/// The 066 skip path, used only when thresholds are pinned.
///
/// Jev answers first, under its own deadline; then the review runs unless
/// the skip decision says skip. The review future is built once and only
/// awaited when it runs, so a skipped turn sends no review request.
pub(super) async fn review_after_jev(
    deps: &CheckpointDeps,
    params: &CheckpointTurnParams,
    eval_id: &str,
    candidates: &[review::Candidate],
    preferences: &[preference::PreferenceCandidate],
    window: &crate::checkpoint::trajectory::TrajectoryWindow,
    capture_enabled: bool,
) -> (
    Option<Result<ReviewOutput, AppError>>,
    Option<TurnJevOutcome>,
    Option<ReviewSkippedBy>,
) {
    let activity = review::turn_activity(window);
    let review = review::review_once(
        deps.model_client.as_ref(),
        &deps.review_mode,
        candidates,
        preferences,
        &params.final_message,
        &activity,
        capture_enabled,
    );
    let Some(jev) = &deps.jev else {
        return (Some(review.await), None, None);
    };
    let asked = Instant::now();
    let mut row = Box::pin(crate::jev::shadow::shadow_turn(
        jev,
        crate::jev::shadow::TurnInputs {
            window,
            activity: &activity,
            final_message: &params.final_message,
            capture_enabled,
        },
        crate::jev::shadow::TurnIds {
            eval_id,
            harness_session_id: &params.session_id,
            transcript_path: &params.transcript_path,
            now: deps.clock.now(),
        },
    ))
    .await;
    let wait_ms = u64::try_from(asked.elapsed().as_millis()).unwrap_or(u64::MAX);
    let decision = skip_decision(&SkipInput {
        thresholds: deps.skip_thresholds,
        memory_configured: deps.embedder.is_some(),
        contradiction: stored_answer(&row.questions_json, "contradiction"),
        capture_worthy: stored_answer(&row.questions_json, "capture_worthy"),
        candidates: candidates.len() + preferences.len(),
        truncated: row.truncated,
        eval_id,
    });
    if decision == SkipDecision::Skip {
        row.applied = true;
        let outcome = TurnJevOutcome {
            row: Box::new(row),
            wait_ms,
        };
        return (None, Some(outcome), Some(ReviewSkippedBy::Jev));
    }
    let review = review.await;
    label_turn_row(deps, &mut row, &review, candidates, capture_enabled);
    let by = (decision == SkipDecision::Audit).then_some(ReviewSkippedBy::Audit);
    (
        Some(review),
        Some(TurnJevOutcome {
            row: Box::new(row),
            wait_ms,
        }),
        by,
    )
}

/// The big model's label on the turn's Jev row, from the review's answers.
fn label_turn_row(
    deps: &CheckpointDeps,
    row: &mut crate::storage::jev::JevRecord,
    review: &Result<ReviewOutput, AppError>,
    candidates: &[review::Candidate],
    capture_enabled: bool,
) {
    let Ok((flags, _, _, _, raw)) = review else {
        return;
    };
    let memory_pairs = candidates
        .iter()
        .any(|c| c.origin == review::CandidateOrigin::Memory);
    let flagged = |kind: SignalKind| flags.iter().any(|(signal, _)| signal.kind == kind);
    row.label_json = Some(crate::jev::shadow::turn_label(
        raw,
        memory_pairs,
        capture_enabled,
        crate::jev::shadow::ReviewOutcome {
            contradiction_flagged: flagged(SignalKind::SelfContradiction),
            violation_flagged: flagged(SignalKind::PreferenceViolation),
            // Whether a capture was stored is known only after the store
            // step in `run_turn`, which sets it there.
            captured: false,
        },
    ));
    row.label_source = Some(format!("{}/{}", deps.model, review::REVIEW_MODE_ID));
}

/// The turn's Jev row and how long the turn waited on Jev after the review
/// finished (064).
pub(super) struct TurnJevOutcome {
    // Boxed: the row is large, and carrying it by value through the record
    // step pushed the turn future past the large-future lint (064).
    pub(super) row: Box<crate::storage::jev::JevRecord>,
    pub(super) wait_ms: u64,
}
