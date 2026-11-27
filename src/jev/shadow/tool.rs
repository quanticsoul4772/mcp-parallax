//! The tool-risk shadow (067): asks Jev the `action_class` question about a
//! pending tool call's projection, off the gate's response path, and asks
//! the big model the same question for the row's label.

use super::{questions_json, record_result, redaction_failed};
use crate::checkpoint::tool_risk::redacted_projection;
use crate::client::typesafe::JEV_TOOL_DEADLINE_MS;
use crate::error::Outcome;
use crate::jev::questions::action_class_question;
use crate::jev::redact::Redactor;
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::{Classifier, JevMode, JevQuestion, JevRequest, JevSite};
use crate::traits::client::ModelClient;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The prompt version recorded in `label_source` beside the model id.
pub const LABEL_PROMPT_VERSION: &str = "tool_risk_label_v1";

/// Jev for the tool-risk site.
pub struct ToolJev {
    /// The classifier (the TypeSafe client in production).
    pub classifier: Arc<dyn Classifier>,
    /// The mandatory redactor.
    pub redactor: Redactor,
    /// The pinned Jev model.
    pub model: String,
    /// This process's session id.
    pub server_session_id: String,
    /// The Jev request deadline (the gate budget, one attempt).
    pub deadline: Duration,
    /// The big model that answers the same question for the label.
    pub label_client: Arc<dyn ModelClient>,
    /// Its model id, recorded in `label_source`.
    pub label_model: String,
}

impl ToolJev {
    /// The production setup: secrets from the environment, the named deadline.
    #[must_use]
    pub fn new(
        classifier: Arc<dyn Classifier>,
        model: String,
        server_session_id: String,
        label_client: Arc<dyn ModelClient>,
        label_model: String,
    ) -> Self {
        Self {
            classifier,
            redactor: Redactor::from_env(),
            model,
            server_session_id,
            deadline: Duration::from_millis(JEV_TOOL_DEADLINE_MS),
            label_client,
            label_model,
        }
    }
}

/// Where the row belongs.
pub struct ToolIds<'a> {
    /// The checkpoint record's id.
    pub eval_id: &'a str,
    /// The Claude Code session.
    pub harness_session_id: &'a str,
    /// The transcript path.
    pub transcript_path: &'a str,
    /// The row's timestamp.
    pub now: DateTime<Utc>,
}

/// The tool-risk site's questions: `action_class` only, no memory question.
#[must_use]
pub fn tool_questions() -> Vec<JevQuestion> {
    vec![action_class_question()]
}

/// The row for a call settled in code: no request, the reason in
/// `error_class`.
#[must_use]
pub fn excluded_tool_row(jev: &ToolJev, reason: &str, ids: &ToolIds<'_>) -> JevRecord {
    let mut row = tool_row(jev, &tool_questions(), ids);
    row.outcome = JevOutcome::Excluded;
    row.error_class = Some(reason.to_string());
    row
}

/// Ask Jev about one pending call's projection, then the big model for the
/// label when Jev answered, and build the row.
pub async fn shadow_tool(
    jev: &ToolJev,
    tool_name: &str,
    tool_input: &str,
    ids: ToolIds<'_>,
) -> JevRecord {
    let questions = tool_questions();
    let mut row = tool_row(jev, &questions, &ids);
    let state = match redacted_projection(&jev.redactor, tool_name, tool_input) {
        Ok(state) => state,
        Err(error) => return redaction_failed(row, &error),
    };
    row.redaction_count = u32::try_from(state.redactions()).unwrap_or(u32::MAX);
    row.state_hash = Some(format!(
        "{:016x}",
        crate::checkpoint::fnv1a64(state.as_str().as_bytes())
    ));
    let state_text = state.as_str().to_string();
    let request = JevRequest {
        model: jev.model.clone(),
        state,
        questions,
        timeout: jev.deadline,
        retry: false,
    };
    let started = Instant::now();
    let result = tokio::time::timeout(jev.deadline, jev.classifier.classify(&request)).await;
    row.latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_result(&mut row, result, &request, &jev.model);
    if row.outcome == JevOutcome::Call(Outcome::Success) {
        match label(jev, &state_text).await {
            Ok(class) => {
                row.label_json = Some(json!({ "class": class }));
                row.label_source = Some(format!("{}:{LABEL_PROMPT_VERSION}", jev.label_model));
            }
            Err(reason) => {
                let note = format!("label: {reason}");
                row.error_class = Some(match row.error_class.take() {
                    Some(existing) => format!("{existing}; {note}"),
                    None => note,
                });
            }
        }
    }
    row
}

/// The big model's `action_class` answer for the same redacted projection.
async fn label(jev: &ToolJev, state: &str) -> Result<String, String> {
    let question = action_class_question();
    let names: Vec<&str> = question.options.iter().map(|(n, _)| n.as_str()).collect();
    let options = question
        .options
        .iter()
        .fold(String::new(), |mut text, (name, criteria)| {
            use std::fmt::Write as _;
            let _ = writeln!(text, "- {name}: {}", criteria.as_deref().unwrap_or(""));
            text
        });
    let prompt = format!(
        "{}\n\nOptions:\n{options}\nProposed tool call:\n{state}\n",
        question.instructions
    );
    let schema = json!({
        "type": "object",
        "properties": { "class": { "type": "string", "enum": names } },
        "required": ["class"],
        "additionalProperties": false,
    });
    let completion = jev
        .label_client
        .complete(&prompt, &schema)
        .await
        .map_err(|error| error.outcome().as_str().to_string())?;
    let class = completion
        .value
        .get("class")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "no class".to_string())?;
    if names.contains(&class) {
        Ok(class.to_string())
    } else {
        Err("undeclared class".to_string())
    }
}

fn tool_row(jev: &ToolJev, questions: &[JevQuestion], ids: &ToolIds<'_>) -> JevRecord {
    JevRecord {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: ids.now,
        server_session_id: jev.server_session_id.clone(),
        harness_session_id: Some(ids.harness_session_id.to_string()),
        site: JevSite::ToolRisk,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::tool_questions;
    use crate::jev::questions::action_class_question;

    // 067 T015 (§11 test 7): the tool-risk site asks Jev no memory question
    // at all, only `action_class`.
    #[test]
    fn the_tool_risk_site_asks_no_memory_question() {
        let questions = tool_questions();
        let ids: Vec<&str> = questions.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids, [action_class_question().id.as_str()]);
    }
}
