//! The end-of-turn review shadow (064).

use super::{questions_json, record_result, redaction_failed};
use crate::checkpoint::review::{
    mine_candidates, truncate, CandidateOrigin, CaptureKind, ReviewRaw, FINAL_MESSAGE_EXCERPT_CHARS,
};
use crate::checkpoint::trajectory::{TrajectoryEntry, TrajectoryWindow};
use crate::client::typesafe::JEV_TURN_DEADLINE_MS;
use crate::error::Outcome;
use crate::jev::questions::turn_review_questions;
use crate::jev::redact::{RedactError, Redactor};
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::{Classifier, JevMode, JevQuestion, JevRequest, JevSite};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Jev for the end-of-turn review site.
pub struct TurnJev {
    /// The classifier (the TypeSafe client in production).
    pub classifier: Arc<dyn Classifier>,
    /// The mandatory redactor.
    pub redactor: Redactor,
    /// The pinned model.
    pub model: String,
    /// This process's session id.
    pub server_session_id: String,
    /// The request deadline.
    pub deadline: Duration,
}

impl TurnJev {
    /// The production setup: secrets from the environment, the named deadline.
    #[must_use]
    pub fn new(classifier: Arc<dyn Classifier>, model: String, server_session_id: String) -> Self {
        Self {
            classifier,
            redactor: Redactor::from_env(),
            model,
            server_session_id,
            deadline: Duration::from_millis(JEV_TURN_DEADLINE_MS),
        }
    }
}

/// What the turn shadow sees.
pub struct TurnInputs<'a> {
    /// The turn's window; its transcript pairs are mined again here, from
    /// redacted text, so stored memories never enter.
    pub window: &'a TrajectoryWindow,
    /// The turn activity summary.
    pub activity: &'a str,
    /// The final assistant message.
    pub final_message: &'a str,
    /// Memory is configured, so capture is asked about.
    pub capture_enabled: bool,
}

/// Where the row belongs.
pub struct TurnIds<'a> {
    /// The checkpoint record's id.
    pub eval_id: &'a str,
    /// The Claude Code session.
    pub harness_session_id: &'a str,
    /// The transcript path.
    pub transcript_path: &'a str,
    /// The row's timestamp.
    pub now: DateTime<Utc>,
}

/// The state the turn shadow sends, and what its row records about it.
pub struct TurnState {
    /// The redacted state.
    pub state: crate::jev::redact::RedactedState,
    /// Redactions across the texts and the state.
    pub redactions: u32,
    /// The final message was cut to the excerpt length.
    pub truncated: bool,
    /// The fingerprint stored as the row's `state_hash`.
    pub hash: String,
}

/// A turn state that could not be redacted; nothing is sent.
#[derive(Debug)]
pub struct TurnStateError {
    /// The redaction failure.
    pub error: RedactError,
    /// The final message had already been cut to the excerpt length.
    pub truncated: bool,
}

/// Build the turn state exactly as the shadow sends it. The back-test
/// harness rebuilds stored turns with this function and compares `hash`.
///
/// # Errors
///
/// A redaction failure, with whether the final message had been cut.
pub fn turn_state(
    redactor: &Redactor,
    window: &TrajectoryWindow,
    activity: &str,
    final_message: &str,
) -> Result<TurnState, TurnStateError> {
    // Redact the whole texts first: a secret on the cut, or one split across
    // sentences (a JWT, a PEM block), must be removed before either happens.
    let (window, final_message, text_redactions) = redact_turn(redactor, window, final_message)
        .map_err(|error| TurnStateError {
            error,
            truncated: false,
        })?;
    let candidates = mine_candidates(&window, &final_message, &[]);
    let (final_text, truncated) = truncate(&final_message, FINAL_MESSAGE_EXCERPT_CHARS);
    let pairs: Vec<Value> = candidates
        .iter()
        .filter(|c| c.origin == CandidateOrigin::Transcript)
        .map(|c| {
            json!({
                "earlier": c.earlier,
                "final": c.final_statement,
                "observed_between": c.between,
            })
        })
        .collect();
    let state = json!({
        "statement_pairs": pairs,
        "activity": activity,
        "final_message": final_text,
    });
    let redacted = redactor
        .redact_json(&state)
        .map_err(|error| TurnStateError { error, truncated })?;
    let redactions = u32::try_from(text_redactions + redacted.redactions()).unwrap_or(u32::MAX);
    let hash = format!(
        "{:016x}",
        crate::checkpoint::fnv1a64(redacted.as_str().as_bytes())
    );
    Ok(TurnState {
        state: redacted,
        redactions,
        truncated,
        hash,
    })
}

/// Ask Jev about one turn and build its row (label still empty).
pub async fn shadow_turn(jev: &TurnJev, inputs: TurnInputs<'_>, ids: TurnIds<'_>) -> JevRecord {
    let questions: Vec<JevQuestion> = turn_review_questions()
        .into_iter()
        .filter(|q| inputs.capture_enabled || q.id != "capture_worthy")
        .collect();
    let mut row = turn_row(jev, &questions, &ids);
    let built = match turn_state(
        &jev.redactor,
        inputs.window,
        inputs.activity,
        inputs.final_message,
    ) {
        Ok(built) => built,
        Err(failed) => {
            row.truncated = failed.truncated;
            return redaction_failed(row, &failed.error);
        }
    };
    row.truncated = built.truncated;
    row.redaction_count = built.redactions;
    row.state_hash = Some(built.hash);
    let request = JevRequest {
        model: jev.model.clone(),
        state: built.state,
        questions,
        timeout: jev.deadline,
        retry: false,
    };
    let started = Instant::now();
    let result = tokio::time::timeout(jev.deadline, jev.classifier.classify(&request)).await;
    row.latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_result(&mut row, result, &request, &jev.model);
    row
}

/// What the review did with its answers: the flags it raised and whether its
/// capture was stored. Plan §7.1 counts these, not the raw answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewOutcome {
    /// A self-contradiction flag was raised.
    pub contradiction_flagged: bool,
    /// A preference-violation flag was raised.
    pub violation_flagged: bool,
    /// The review's capture was stored (not dropped at the session cap or by
    /// a storage failure).
    pub captured: bool,
}

/// The big model's label for a turn, enumerated fields only: its raw answers
/// and what the review did with them.
///
/// `contradicts` and `contradiction_flagged` are empty when memory pairs were
/// among the candidates (Jev never sees those); `capture_kind` and `captured`
/// are empty when memory is off (capture isn't asked).
#[must_use]
pub fn turn_label(
    raw: &ReviewRaw,
    memory_pairs: bool,
    capture_enabled: bool,
    did: ReviewOutcome,
) -> Value {
    let capture_kind = match raw.capture_kind {
        CaptureKind::Skill => "skill",
        CaptureKind::Lesson => "lesson",
        CaptureKind::None => "none",
    };
    let unless_memory = |v: bool| {
        if memory_pairs {
            Value::Null
        } else {
            Value::Bool(v)
        }
    };
    let if_capture = |v: Value| if capture_enabled { v } else { Value::Null };
    json!({
        "contradicts": unless_memory(raw.contradicts),
        "violates": raw.violates,
        "capture_kind": if_capture(Value::from(capture_kind)),
        "contradiction_flagged": unless_memory(did.contradiction_flagged),
        "violation_flagged": did.violation_flagged,
        "captured": if_capture(Value::Bool(did.captured)),
    })
}

/// Whether a stored turn label shows the review saw no memory-derived
/// candidates, which a replay without stored memories needs.
///
/// The label nulls `contradiction_flagged` exactly when memory candidates
/// were present; labels written before that field existed null
/// `contradicts` under the same condition, so either field answers it.
///
/// # Errors
///
/// "memory candidates present" when the field is null, "no review outcome
/// in the label" when neither field is there.
pub fn no_memory_candidates(label: Option<&Value>) -> Result<(), &'static str> {
    let field = label.and_then(|l| {
        l.get("contradiction_flagged")
            .or_else(|| l.get("contradicts"))
    });
    match field {
        Some(Value::Bool(_)) => Ok(()),
        Some(Value::Null) => Err("memory candidates present"),
        _ => Err("no review outcome in the label"),
    }
}

/// The turn row before any request: identity, questions, where it belongs.
fn turn_row(jev: &TurnJev, questions: &[JevQuestion], ids: &TurnIds<'_>) -> JevRecord {
    JevRecord {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: ids.now,
        server_session_id: jev.server_session_id.clone(),
        harness_session_id: Some(ids.harness_session_id.to_string()),
        site: JevSite::TurnReview,
        mode: JevMode::Shadow,
        questions_json: questions_json(questions, &[]),
        model_requested: jev.model.clone(),
        model_answered: None,
        outcome: JevOutcome::Call(Outcome::Success),
        error_class: None,
        request_id: None,
        latency_ms: None,
        input_tokens: None,
        cost_usd: None,
        eval_id: Some(ids.eval_id.to_string()),
        invocation_id: None,
        claim_key: None,
        source_ids: None,
        label_json: None,
        label_source: None,
        state_json: None,
        state_hash: None,
        transcript_ref: Some(ids.transcript_path.to_string()),
        redaction_count: 0,
        truncated: false,
        applied: false,
    }
}

/// Redact the final message and every assistant text in the window, whole.
fn redact_turn(
    redactor: &Redactor,
    window: &TrajectoryWindow,
    final_message: &str,
) -> Result<(TrajectoryWindow, String, usize), RedactError> {
    let final_redacted = redactor.redact(final_message)?;
    let mut count = final_redacted.redactions();
    let mut entries = Vec::with_capacity(window.entries.len());
    for entry in &window.entries {
        entries.push(match entry {
            TrajectoryEntry::Assistant { text } => {
                let redacted = redactor.redact(text)?;
                count += redacted.redactions();
                TrajectoryEntry::Assistant {
                    text: redacted.as_str().to_string(),
                }
            }
            tool @ TrajectoryEntry::ToolCall { .. } => tool.clone(),
        });
    }
    let window = TrajectoryWindow {
        session_id: window.session_id.clone(),
        entries,
    };
    Ok((window, final_redacted.as_str().to_string(), count))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_state_too_large_to_redact_still_reports_the_cut_final_message() {
        let window = TrajectoryWindow {
            session_id: "s".into(),
            entries: vec![],
        };
        let activity = "a".repeat(199_000);
        let final_message = "b".repeat(FINAL_MESSAGE_EXCERPT_CHARS + 500);
        let failed = turn_state(&Redactor::new(vec![]), &window, &activity, &final_message)
            .err()
            .expect("a state over the redactor's cap fails");
        assert!(failed.truncated);
    }

    #[test]
    fn a_label_shows_no_memory_candidates_by_either_contradiction_field() {
        let raw = ReviewRaw {
            contradicts: false,
            violates: false,
            capture_kind: CaptureKind::None,
        };
        let did = ReviewOutcome {
            contradiction_flagged: false,
            violation_flagged: false,
            captured: false,
        };
        // Labels as the server writes them now.
        assert_eq!(
            no_memory_candidates(Some(&turn_label(&raw, false, true, did))),
            Ok(())
        );
        assert_eq!(
            no_memory_candidates(Some(&turn_label(&raw, true, true, did))),
            Err("memory candidates present")
        );
        // Labels as 064 first wrote them, before the outcome fields.
        let old = |contradicts: Value| json!({ "contradicts": contradicts, "violates": false });
        assert_eq!(no_memory_candidates(Some(&old(json!(true)))), Ok(()));
        assert_eq!(
            no_memory_candidates(Some(&old(Value::Null))),
            Err("memory candidates present")
        );
        assert_eq!(
            no_memory_candidates(Some(&json!({ "violates": false }))),
            Err("no review outcome in the label")
        );
        assert_eq!(
            no_memory_candidates(None),
            Err("no review outcome in the label")
        );
    }
}
