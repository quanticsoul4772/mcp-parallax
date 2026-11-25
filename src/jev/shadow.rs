//! Jev shadow glue (064): asks Jev beside the big model and builds the
//! `jev_records` row. Shadow only: nothing here changes a decision, and every
//! failure becomes a row, never an error for the caller.

mod claim;
mod turn;

pub use claim::{
    abandoned, ask_claim, claim_label, prepare_claim, shadow_claim, ClaimJev, ClaimSource,
    PreparedClaim, ABANDONED_ERROR,
};
pub use turn::{
    shadow_turn, turn_label, turn_state, ReviewOutcome, TurnIds, TurnInputs, TurnJev, TurnState,
};

use crate::error::{AppError, Outcome};
pub use crate::jev::exclusion::{claim_exclusion, Exclusion};
use crate::jev::policy::validate;
use crate::jev::redact::RedactError;
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::{JevAnswer, JevQuestion, JevRequest, JevResponse};
use serde_json::{json, Value};

/// Fill the row from what the call ended with: the deadline, an error, or an
/// answer (kept only when it passes validation).
fn record_result(
    row: &mut JevRecord,
    result: Result<Result<JevResponse, AppError>, tokio::time::error::Elapsed>,
    request: &JevRequest,
    model: &str,
) {
    match result {
        Err(_) => {
            row.outcome = JevOutcome::Call(Outcome::Timeout);
            row.error_class = Some("deadline; cost unknown".into());
        }
        Ok(Err(error)) => {
            row.outcome = JevOutcome::Call(error.outcome());
            row.error_class = Some(error_class(&error));
        }
        Ok(Ok(response)) => {
            row.model_answered = Some(response.model_answered.clone());
            row.request_id.clone_from(&response.request_id);
            row.input_tokens = response.input_tokens;
            row.cost_usd = response
                .input_tokens
                .map(|tokens| crate::telemetry::cost_usd(model, tokens, 0));
            match validate(&request.questions, &response, model) {
                Ok(()) => {
                    row.questions_json = questions_json(&request.questions, &response.answers);
                    if response.input_tokens.is_none() {
                        row.error_class = Some("cost unknown".into());
                    }
                }
                Err(reason) => {
                    row.outcome = JevOutcome::Call(Outcome::ValidationFailure);
                    row.error_class = Some(format!("validator: {reason}"));
                }
            }
        }
    }
}

/// A turn whose redaction failed: nothing is sent.
fn redaction_failed(mut row: JevRecord, error: &RedactError) -> JevRecord {
    row.outcome = JevOutcome::Call(Outcome::InvalidInput);
    row.error_class = Some(match error {
        RedactError::TooLarge { .. } => "redaction:too_large".into(),
        RedactError::Pattern(_) => "redaction:pattern".into(),
    });
    row
}

/// A short class for a failed call. Calls that sent nothing say why
/// (`disabled_after_401`, `no_slot`); calls cut off mid-request may still be
/// billed, so they say the cost is unknown (064 review J, K).
fn error_class(error: &AppError) -> String {
    match error.root() {
        AppError::ProviderRejected { detail, .. }
            if detail.contains("disabled after an earlier 401") =>
        {
            "disabled_after_401".into()
        }
        AppError::ProviderRejected { status, .. } => format!("http_{status}"),
        AppError::Timeout {
            what: "jev request slot",
            ..
        } => "no_slot".into(),
        AppError::Timeout { .. } => "deadline; cost unknown".into(),
        other => other.outcome().as_str().into(),
    }
}

/// `[{id, version, option_order, answer, probabilities, band}]`; bands stay
/// empty in 064 (no thresholds yet).
fn questions_json(questions: &[JevQuestion], answers: &[(String, JevAnswer)]) -> Value {
    Value::Array(
        questions
            .iter()
            .map(|q| {
                let answer = answers.iter().find(|(id, _)| *id == q.id).map(|(_, a)| a);
                let (answer, probabilities) = match answer {
                    Some(JevAnswer::Noul { p }) => (json!(p), Value::Null),
                    Some(JevAnswer::Choice {
                        choice,
                        probabilities,
                    }) => (
                        json!(choice),
                        Value::Object(
                            probabilities
                                .iter()
                                .map(|(o, p)| (o.clone(), json!(p)))
                                .collect(),
                        ),
                    ),
                    None => (Value::Null, Value::Null),
                };
                json!({
                    "id": q.id,
                    "version": q.version,
                    "option_order": q.options.iter().map(|(o, _)| o.as_str()).collect::<Vec<_>>(),
                    "answer": answer,
                    "probabilities": probabilities,
                    "band": Value::Null,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod tests_support {
    use crate::error::Outcome;
    use crate::storage::jev::{JevOutcome, JevRecord};
    use crate::traits::classifier::{JevMode, JevSite};

    /// A minimal research row.
    pub fn claim_row() -> JevRecord {
        JevRecord {
            id: "r".into(),
            created_at: chrono::Utc::now(),
            server_session_id: "s".into(),
            harness_session_id: None,
            site: JevSite::ResearchClaims,
            mode: JevMode::Shadow,
            questions_json: serde_json::json!([]),
            model_requested: "jev-1.14.0".into(),
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
            source_ids: None,
            label_json: None,
            label_source: None,
            state_json: None,
            state_hash: None,
            transcript_ref: None,
            redaction_count: 0,
            truncated: false,
        }
    }
}
