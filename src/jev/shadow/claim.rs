//! The research claim shadow (064).

use super::{questions_json, record_result};
use crate::client::typesafe::JEV_CLAIM_DEADLINE_MS;
use crate::error::Outcome;
use crate::jev::exclusion::claim_exclusion;
use crate::jev::questions::research_claim_questions;
use crate::jev::redact::{RedactError, Redactor};
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::{Classifier, JevMode, JevRequest, JevSite};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Jev for research claim verification.
pub struct ClaimJev {
    /// The classifier (the TypeSafe client in production).
    pub classifier: Arc<dyn Classifier>,
    /// The mandatory redactor.
    pub redactor: Redactor,
    /// The pinned model.
    pub model: String,
    /// This process's session id.
    pub server_session_id: String,
    /// The request deadline, further capped by the run's time left.
    pub deadline: Duration,
}

impl ClaimJev {
    /// The production setup: secrets from the environment, the named deadline.
    #[must_use]
    pub fn new(classifier: Arc<dyn Classifier>, model: String, server_session_id: String) -> Self {
        Self {
            classifier,
            redactor: Redactor::from_env(),
            model,
            server_session_id,
            deadline: Duration::from_millis(JEV_CLAIM_DEADLINE_MS),
        }
    }
}

/// One source behind a claim, as Jev sees it.
pub struct ClaimSource {
    /// The source id (`s1`, ...).
    pub id: String,
    /// The excerpt the verify passes see.
    pub excerpt: String,
    /// Flagged by research's injection screen (063).
    pub flagged: bool,
}

/// A claim ready for Jev, or already finished (excluded, or redaction failed).
pub enum PreparedClaim {
    /// No request will be made; the row is complete.
    Done(Box<JevRecord>),
    /// Ask with this request; the row is filled in from the answer.
    Ask(Box<JevRecord>, Box<JevRequest>),
}

/// Build a claim's row and, unless the claim is excluded, its request.
#[must_use]
pub fn prepare_claim(
    jev: &ClaimJev,
    claim_text: &str,
    claim_key: &str,
    sources: &[ClaimSource],
    now: DateTime<Utc>,
) -> PreparedClaim {
    let questions = research_claim_questions(sources.len());
    let mut row = JevRecord {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: now,
        server_session_id: jev.server_session_id.clone(),
        harness_session_id: None,
        site: JevSite::ResearchClaims,
        mode: JevMode::Shadow,
        questions_json: questions_json(&questions, &[]),
        model_requested: jev.model.clone(),
        model_answered: None,
        outcome: JevOutcome::Call(Outcome::Success),
        error_class: None,
        request_id: None,
        latency_ms: None,
        input_tokens: None,
        cost_usd: None,
        eval_id: None,
        invocation_id: None,
        claim_key: Some(claim_key.to_string()),
        source_ids: Some(sources.iter().map(|s| s.id.clone()).collect()),
        label_json: None,
        label_source: None,
        label_cost_usd: None,
        cooldown_suppressed: None,
        state_json: None,
        state_hash: None,
        transcript_ref: None,
        redaction_count: 0,
        truncated: false,
        applied: false,
    };
    if let Some(reason) = claim_exclusion(claim_text, sources.iter().any(|s| s.flagged)) {
        row.outcome = JevOutcome::Excluded;
        row.error_class = Some(reason.as_str().into());
        return PreparedClaim::Done(Box::new(row));
    }
    let numbered: serde_json::Map<String, Value> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| ((i + 1).to_string(), json!({ "excerpts": [s.excerpt] })))
        .collect();
    let state = json!({ "claim": claim_text, "sources": numbered });
    match jev.redactor.redact_json(&state) {
        Err(error) => {
            row.outcome = JevOutcome::Call(Outcome::InvalidInput);
            row.error_class = Some(match error {
                RedactError::TooLarge { .. } => "redaction:too_large".into(),
                RedactError::Pattern(_) => "redaction:pattern".into(),
            });
            PreparedClaim::Done(Box::new(row))
        }
        Ok(redacted) => {
            row.redaction_count = u32::try_from(redacted.redactions()).unwrap_or(u32::MAX);
            row.state_json = redacted.as_json().cloned();
            let request = JevRequest {
                model: jev.model.clone(),
                state: redacted,
                questions,
                timeout: jev.deadline,
                retry: false,
            };
            PreparedClaim::Ask(Box::new(row), Box::new(request))
        }
    }
}

/// Ask Jev about a prepared claim, within the deadline and the run's time
/// left; with no time left nothing is sent and the row records `timeout`.
pub async fn ask_claim(
    jev: &ClaimJev,
    mut row: JevRecord,
    mut request: JevRequest,
    time_left: Duration,
) -> JevRecord {
    let timeout = jev.deadline.min(time_left);
    if timeout.is_zero() {
        row.outcome = JevOutcome::Call(Outcome::Timeout);
        row.error_class = Some("no_time_left".into());
        return row;
    }
    request.timeout = timeout;
    let started = Instant::now();
    let result = tokio::time::timeout(timeout, jev.classifier.classify(&request)).await;
    row.latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_result(&mut row, result, &request, &jev.model);
    row
}

/// [`prepare_claim`] then [`ask_claim`].
pub async fn shadow_claim(
    jev: &ClaimJev,
    claim_text: &str,
    claim_key: &str,
    sources: &[ClaimSource],
    time_left: Duration,
    now: DateTime<Utc>,
) -> JevRecord {
    match prepare_claim(jev, claim_text, claim_key, sources, now) {
        PreparedClaim::Done(row) => *row,
        PreparedClaim::Ask(row, request) => ask_claim(jev, *row, *request, time_left).await,
    }
}

/// The error class of a Jev call abandoned because verify finished first.
pub const ABANDONED_ERROR: &str = "verify_finished; cost unknown";

/// The row of a Jev call abandoned because verify finished first.
#[must_use]
pub fn abandoned(mut row: JevRecord) -> JevRecord {
    row.outcome = JevOutcome::Call(Outcome::Timeout);
    row.error_class = Some(ABANDONED_ERROR.into());
    row
}

/// The ensemble's label for a claim, enumerated fields only.
#[must_use]
pub fn claim_label(
    support: &Value,
    majority_supported: bool,
    agreement: f64,
    passes: u32,
    sources: usize,
) -> Value {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )] // agreement is in 0..=1 and passes is a small count
    let majority = ((agreement * f64::from(passes)).round() as u32).min(passes);
    let (supported, refuted) = if majority_supported {
        (majority, passes - majority)
    } else {
        (passes - majority, majority)
    };
    json!({
        "support": support,
        "votes_supported": supported,
        "votes_refuted": refuted,
        "votes_completed": passes,
        "sources": sources,
    })
}
