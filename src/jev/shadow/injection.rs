//! The injection screen on kept research pages (065).
//!
//! Each page research keeps for extraction is asked Jev's `directs_ai`
//! question as redacted `{page_url, page_text}`. With the site `on`, a page
//! at or above the pinned threshold carries an advisory warning; in shadow
//! the same call is only recorded. A failed or late call never warns.

use super::{questions_json, record_result, redaction_failed};
use crate::client::typesafe::JEV_CLAIM_DEADLINE_MS;
use crate::error::Outcome;
use crate::jev::policy::{injection_decision, injection_threshold, InjectionDecision};
use crate::jev::questions::injection_question;
use crate::jev::redact::Redactor;
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::{Classifier, JevMode, JevRequest, JevSite};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Jev for the research injection screen.
pub struct InjectionJev {
    /// The classifier (the TypeSafe client in production).
    pub classifier: Arc<dyn Classifier>,
    /// The mandatory redactor.
    pub redactor: Redactor,
    /// The pinned model.
    pub model: String,
    /// This process's session id.
    pub server_session_id: String,
    /// `Shadow` records only; `On` lets a warning reach the result.
    pub mode: JevMode,
    /// The pinned threshold for `model`; `None` only in shadow for a model
    /// without one, where no decision is made.
    pub threshold: Option<f64>,
    /// The request deadline, retries included, further capped by the run's
    /// time left (065 clarification: 2000 ms).
    pub deadline: Duration,
}

impl InjectionJev {
    /// The production setup: secrets from the environment, the pinned
    /// threshold for `model`, the named deadline.
    #[must_use]
    pub fn new(
        classifier: Arc<dyn Classifier>,
        model: String,
        server_session_id: String,
        mode: JevMode,
    ) -> Self {
        Self {
            threshold: injection_threshold(&model),
            classifier,
            redactor: Redactor::from_env(),
            model,
            server_session_id,
            mode,
            deadline: Duration::from_millis(JEV_CLAIM_DEADLINE_MS),
        }
    }
}

/// One page's screening, as the research result shows it with the site on.
#[derive(Debug, Clone, PartialEq)]
pub struct Screening {
    /// Jev returned a valid answer from the pinned model in time.
    pub screened: bool,
    /// P(yes) when screened.
    pub probability: Option<f64>,
    /// Screened and at or above the threshold.
    pub warned: bool,
    /// Why the page was not screened.
    pub reason: Option<&'static str>,
}

fn row(jev: &InjectionJev, source_id: &str, patterns063: &[&str], now: DateTime<Utc>) -> JevRecord {
    JevRecord {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: now,
        server_session_id: jev.server_session_id.clone(),
        harness_session_id: None,
        site: JevSite::ResearchInjection,
        mode: jev.mode,
        questions_json: questions_json(&[injection_question()], &[]),
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
        claim_key: None,
        source_ids: Some(vec![source_id.to_string()]),
        label_json: Some(json!({ "patterns063": patterns063 })),
        label_source: Some("063 pattern screen".into()),
        label_cost_usd: None,
        state_json: None,
        state_hash: None,
        transcript_ref: None,
        redaction_count: 0,
        truncated: false,
        applied: false,
    }
}

/// Why a row that ended without a usable answer was not screened.
fn reason(row: &JevRecord) -> &'static str {
    match row.outcome {
        JevOutcome::Call(Outcome::Timeout) => "timeout",
        JevOutcome::Call(Outcome::ProviderRejected)
            if row.error_class.as_deref() == Some("disabled_after_401") =>
        {
            "disabled"
        }
        JevOutcome::Call(Outcome::ProviderRejected) => "provider_rejected",
        JevOutcome::Call(Outcome::InvalidInput) => "redaction_failed",
        JevOutcome::Call(Outcome::ValidationFailure) => "validation_failure",
        _ => "classifier_provider",
    }
}

/// The error class of a call abandoned because its research run was dropped
/// while the request was in flight; it may still be billed.
pub const CANCELLED_ERROR: &str = "cancelled; cost unknown";

/// The row for a page whose Jev call was in flight when the run was dropped.
#[must_use]
pub fn abandoned_row(
    jev: &InjectionJev,
    source_id: &str,
    patterns063: &[&str],
    now: DateTime<Utc>,
) -> JevRecord {
    let mut row = row(jev, source_id, patterns063, now);
    row.outcome = JevOutcome::Call(Outcome::Timeout);
    row.error_class = Some(CANCELLED_ERROR.into());
    row
}

fn not_screened(reason: &'static str) -> Screening {
    Screening {
        screened: false,
        probability: None,
        warned: false,
        reason: Some(reason),
    }
}

/// Screen one kept page: build its row, ask Jev within the deadline and the
/// run's time left (retrying only when the site is on), and decide.
pub async fn screen_page(
    jev: &InjectionJev,
    source_id: &str,
    url: &str,
    text: &str,
    patterns063: &[&str],
    time_left: Duration,
    now: DateTime<Utc>,
) -> (JevRecord, Screening) {
    let mut row = row(jev, source_id, patterns063, now);
    let page = json!({ "page_url": url, "page_text": text });
    let state = match jev.redactor.redact_json(&page) {
        Ok(state) => state,
        Err(error) => {
            return (
                redaction_failed(row, &error),
                not_screened("redaction_failed"),
            )
        }
    };
    row.redaction_count = u32::try_from(state.redactions()).unwrap_or(u32::MAX);
    let timeout = jev.deadline.min(time_left);
    if timeout.is_zero() {
        row.outcome = JevOutcome::Call(Outcome::Timeout);
        row.error_class = Some("no_time_left".into());
        return (row, not_screened("timeout"));
    }
    let request = JevRequest {
        model: jev.model.clone(),
        state,
        questions: vec![injection_question()],
        timeout,
        retry: jev.mode == JevMode::On,
    };
    let started = Instant::now();
    let result = tokio::time::timeout(timeout, jev.classifier.classify(&request)).await;
    row.latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_result(&mut row, result, &request, &jev.model);
    // The one screened rule the span and the report use too.
    let probability = crate::jev::report::injection::is_screened(&row)
        .then(|| row.questions_json[0]["answer"].as_f64())
        .flatten();
    let Some(p) = probability else {
        let why = reason(&row);
        return (row, not_screened(why));
    };
    let warned = jev
        .threshold
        .is_some_and(|t| injection_decision(Some(p), t) == InjectionDecision::Warned);
    if jev.threshold.is_some() {
        row.questions_json[0]["band"] = json!(if warned { "warned" } else { "clear" });
    }
    row.applied = warned && jev.mode == JevMode::On;
    (
        row,
        Screening {
            screened: true,
            probability: Some(p),
            warned,
            reason: None,
        },
    )
}

#[cfg(test)]
mod tests;
