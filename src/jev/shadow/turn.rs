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

/// Ask Jev about one turn and build its row (label still empty).
pub async fn shadow_turn(jev: &TurnJev, inputs: TurnInputs<'_>, ids: TurnIds<'_>) -> JevRecord {
    let questions: Vec<JevQuestion> = turn_review_questions()
        .into_iter()
        .filter(|q| inputs.capture_enabled || q.id != "capture_worthy")
        .collect();
    let mut row = turn_row(jev, &questions, &ids);
    // Redact the whole texts first: a secret on the cut, or one split across
    // sentences (a JWT, a PEM block), must be removed before either happens.
    let (window, final_message, text_redactions) =
        match redact_turn(&jev.redactor, inputs.window, inputs.final_message) {
            Ok(redacted) => redacted,
            Err(error) => return redaction_failed(row, &error),
        };
    let candidates = mine_candidates(&window, &final_message, &[]);
    let (final_text, truncated) = truncate(&final_message, FINAL_MESSAGE_EXCERPT_CHARS);
    row.truncated = truncated;
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
        "activity": inputs.activity,
        "final_message": final_text,
    });
    let redacted = match jev.redactor.redact_json(&state) {
        Ok(redacted) => redacted,
        Err(error) => return redaction_failed(row, &error),
    };
    row.redaction_count =
        u32::try_from(text_redactions + redacted.redactions()).unwrap_or(u32::MAX);
    row.state_hash = Some(format!(
        "{:016x}",
        crate::checkpoint::fnv1a64(redacted.as_str().as_bytes())
    ));
    let request = JevRequest {
        model: jev.model.clone(),
        state: redacted,
        questions,
        timeout: jev.deadline,
    };
    let started = Instant::now();
    let result = tokio::time::timeout(jev.deadline, jev.classifier.classify(&request)).await;
    row.latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_result(&mut row, result, &request, &jev.model);
    row
}

/// The big model's label for a turn, enumerated fields only.
///
/// `contradicts` is empty when memory pairs were among the candidates (Jev
/// never sees those); `capture_kind` is empty when memory is off (capture
/// isn't asked).
#[must_use]
pub fn turn_label(raw: &ReviewRaw, memory_pairs: bool, capture_enabled: bool) -> Value {
    let capture_kind = match raw.capture_kind {
        CaptureKind::Skill => "skill",
        CaptureKind::Lesson => "lesson",
        CaptureKind::None => "none",
    };
    json!({
        "contradicts": if memory_pairs { Value::Null } else { Value::Bool(raw.contradicts) },
        "violates": raw.violates,
        "capture_kind": if capture_enabled { Value::from(capture_kind) } else { Value::Null },
    })
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
