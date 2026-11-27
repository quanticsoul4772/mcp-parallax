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
    /// Tool-risk Jev requests in flight: [`TOOL_JEV_IN_FLIGHT`], so this site
    /// holds at most that many of the client's shared slots (067 review).
    pub jev_slot: Arc<tokio::sync::Semaphore>,
    /// Label calls in flight: [`LABEL_IN_FLIGHT`]; a label that finds none
    /// free is skipped and recorded as busy (067 review).
    pub label_slot: Arc<tokio::sync::Semaphore>,
}

/// Tool-risk Jev requests in flight at once, out of the client's shared slots.
pub const TOOL_JEV_IN_FLIGHT: usize = 1;

/// Tool-risk label calls in flight at once.
pub const LABEL_IN_FLIGHT: usize = 1;

/// The `error_class` note for a label skipped because one was running.
pub const LABEL_BUSY: &str = "label: busy";

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
            jev_slot: Arc::new(tokio::sync::Semaphore::new(TOOL_JEV_IN_FLIGHT)),
            label_slot: Arc::new(tokio::sync::Semaphore::new(LABEL_IN_FLIGHT)),
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

/// Ask Jev about one pending call's projection and build the row.
///
/// When Jev answered, also return the redacted state the label is asked
/// about; the caller writes the row first and labels it after (067 review).
pub async fn shadow_tool(
    jev: &ToolJev,
    tool_name: &str,
    tool_input: &str,
    ids: ToolIds<'_>,
) -> (JevRecord, Option<String>) {
    let questions = tool_questions();
    let mut row = tool_row(jev, &questions, &ids);
    let state = match redacted_projection(&jev.redactor, tool_name, tool_input) {
        Ok(state) => state,
        Err(error) => return (redaction_failed(row, &error), None),
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
    // The site's own slot is taken inside the deadline, so waiting for it
    // counts against the gate budget like the request itself.
    let result = tokio::time::timeout(jev.deadline, async {
        let _slot = jev
            .jev_slot
            .acquire()
            .await
            .map_err(|_| crate::error::AppError::ClassifierProvider("no_slot".into()))?;
        jev.classifier.classify(&request).await
    })
    .await;
    row.latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_result(&mut row, result, &request, &jev.model);
    let answered = row.outcome == JevOutcome::Call(Outcome::Success);
    (row, answered.then_some(state_text))
}

/// The label fields for a row Jev answered: the big model's answer for the
/// same redacted state, or a `label:` note in `error_class` when the call
/// failed or another label call was running.
pub async fn label_tool(
    jev: &ToolJev,
    row: &JevRecord,
    state: &str,
) -> crate::storage::jev::JevLabelUpdate {
    let with_note = |note: &str| crate::storage::jev::JevLabelUpdate {
        error_class: Some(row.error_class.as_ref().map_or_else(
            || note.to_string(),
            |existing| format!("{existing}; {note}"),
        )),
        ..Default::default()
    };
    let Ok(_slot) = jev.label_slot.try_acquire() else {
        return with_note(LABEL_BUSY);
    };
    match label(jev, state).await {
        Ok((class, cost)) => crate::storage::jev::JevLabelUpdate {
            label_json: Some(json!({ "class": class })),
            label_source: Some(format!("{}:{LABEL_PROMPT_VERSION}", jev.label_model)),
            label_cost_usd: Some(cost),
            error_class: row.error_class.clone(),
        },
        Err(reason) => with_note(&format!("label: {reason}")),
    }
}

/// The big model's `action_class` answer for the same redacted projection,
/// with the call's cost (the backend's own figure when it reports one).
async fn label(jev: &ToolJev, state: &str) -> Result<(String, f64), String> {
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
        #[allow(clippy::cast_precision_loss)]
        let cost = completion.reported_cost_micros.map_or_else(
            || {
                crate::telemetry::cost_usd(
                    &jev.label_model,
                    completion.input_tokens,
                    completion.output_tokens,
                )
            },
            |micros| micros as f64 / 1_000_000.0,
        );
        Ok((class.to_string(), cost))
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
        label_cost_usd: None,
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

    use super::{excluded_tool_row, label_tool, ToolIds, ToolJev};
    use crate::error::AppError;
    use crate::jev::redact::Redactor;
    use crate::traits::classifier::{Classifier, JevRequest, JevResponse};
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    struct NoJev;

    #[async_trait::async_trait]
    impl Classifier for NoJev {
        async fn classify(&self, _: &JevRequest) -> Result<JevResponse, AppError> {
            Err(AppError::ClassifierProvider("not used".into()))
        }
        fn model_id(&self) -> &'static str {
            "jev-1.14.0"
        }
    }

    fn jev_with(label: MockModelClient) -> ToolJev {
        ToolJev {
            classifier: Arc::new(NoJev),
            redactor: Redactor::new(vec![]),
            model: "jev-1.14.0".into(),
            server_session_id: "server-1".into(),
            deadline: std::time::Duration::from_millis(500),
            label_client: Arc::new(label),
            label_model: "claude-opus-4-8".into(),
            jev_slot: Arc::new(tokio::sync::Semaphore::new(1)),
            label_slot: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    fn answering(value: serde_json::Value, reported: Option<u64>) -> MockModelClient {
        let mut client = MockModelClient::new();
        client.expect_complete().returning(move |_, _| {
            Ok(Completion {
                value: value.clone(),
                input_tokens: 30,
                output_tokens: 5,
                reported_cost_micros: reported,
            })
        });
        client
    }

    fn row(jev: &ToolJev) -> crate::storage::jev::JevRecord {
        let mut row = excluded_tool_row(
            jev,
            "unused",
            &ToolIds {
                eval_id: "eval-1",
                harness_session_id: "s1",
                transcript_path: "t.jsonl",
                now: chrono::Utc::now(),
            },
        );
        row.error_class = None;
        row
    }

    // 067 review finding 9: each label failure leaves a `label:` note and no
    // label or cost.
    #[tokio::test]
    async fn a_failed_label_leaves_a_note_and_no_label_or_cost() {
        let mut erroring = MockModelClient::new();
        erroring.expect_complete().returning(|_, _| {
            Err(AppError::Timeout {
                what: "label",
                ms: 1,
            })
        });
        for (client, note) in [
            (erroring, "label: timeout"),
            (answering(json!({}), None), "label: no class"),
            (
                answering(json!({ "class": "maybe" }), None),
                "label: undeclared class",
            ),
        ] {
            let jev = jev_with(client);
            let update = label_tool(&jev, &row(&jev), "{}").await;
            assert_eq!(update.error_class.as_deref(), Some(note));
            assert!(update.label_json.is_none() && update.label_cost_usd.is_none());
            assert!(update.label_source.is_none());
        }
    }

    // 067 review finding 9: a backend that reports its own cost is recorded
    // at that figure.
    #[tokio::test]
    async fn a_reported_cost_is_recorded_as_the_label_cost() {
        let jev = jev_with(answering(json!({ "class": "read_only" }), Some(2_500)));
        let update = label_tool(&jev, &row(&jev), "{}").await;
        assert_eq!(update.label_json, Some(json!({ "class": "read_only" })));
        assert_eq!(update.label_cost_usd, Some(0.0025));
        assert_eq!(
            update.label_source.as_deref(),
            Some("claude-opus-4-8:tool_risk_label_v1")
        );
    }

    // 067 review finding 10: the label prompt carries the redacted state the
    // caller passes, the question and its options, and no secret.
    #[tokio::test]
    async fn the_label_prompt_carries_the_redacted_state_and_no_secret() {
        let secret = ["sk-", "label", "Probe", "Secret", "42"].concat();
        let state = crate::checkpoint::tool_risk::redacted_projection(
            &Redactor::new(vec![secret.clone()]),
            "Bash",
            &json!({ "command": format!("echo {secret}") }).to_string(),
        )
        .unwrap();
        let prompts = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&prompts);
        let mut client = MockModelClient::new();
        client.expect_complete().returning(move |prompt, _| {
            seen.lock().unwrap().push(prompt.to_string());
            Ok(Completion {
                value: json!({ "class": "read_only" }),
                input_tokens: 30,
                output_tokens: 5,
                reported_cost_micros: None,
            })
        });
        let jev = jev_with(client);
        label_tool(&jev, &row(&jev), state.as_str()).await;
        let prompts = prompts.lock().unwrap().clone();
        assert_eq!(prompts.len(), 1);
        let prompt = &prompts[0];
        assert!(prompt.contains(state.as_str()));
        assert!(prompt.contains(&action_class_question().instructions));
        for (name, _) in action_class_question().options {
            assert!(prompt.contains(&name), "{name}");
        }
        assert!(!prompt.contains(&secret));
    }
}
