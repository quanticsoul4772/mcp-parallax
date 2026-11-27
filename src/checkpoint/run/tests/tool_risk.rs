//! 067 T012: `run_action` with the tool-risk site (FR-002, FR-005, FR-008,
//! FR-009, SC-003; §11 tests 4 and 5).

use super::*;
use crate::jev::redact::Redactor;
use crate::jev::shadow::ToolJev;
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::{Classifier, JevAnswer, JevRequest, JevResponse, JevSite};
use std::sync::Mutex;
use std::time::Duration as StdDuration;

const PINNED: &str = "jev-1.14.0";

/// Jev answering `action_class` with one option at probability 1.0.
struct Answers(&'static str, Arc<Mutex<Vec<JevRequest>>>);

#[async_trait::async_trait]
impl Classifier for Answers {
    async fn classify(&self, request: &JevRequest) -> Result<JevResponse, AppError> {
        self.1.lock().unwrap().push(request.clone());
        let probabilities = ["spend_or_destructive", "state_changing", "read_only"]
            .iter()
            .map(|o| ((*o).to_string(), if *o == self.0 { 1.0 } else { 0.0 }))
            .collect();
        Ok(JevResponse {
            model_answered: PINNED.into(),
            answers: vec![(
                "action_class".into(),
                JevAnswer::Choice {
                    choice: self.0.into(),
                    probabilities,
                },
            )],
            input_tokens: Some(40),
            request_id: Some("req-1".into()),
            latency_ms: 90,
        })
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

/// Jev failing every call.
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

/// Jev that counts every request; excluded calls must leave it at zero.
struct Counting(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait::async_trait]
impl Classifier for Counting {
    async fn classify(&self, _: &JevRequest) -> Result<JevResponse, AppError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(AppError::ClassifierProvider(
            "excluded calls never reach Jev".into(),
        ))
    }
    fn model_id(&self) -> &str {
        PINNED
    }
}

/// The big model's label answer.
fn label_client(class: &'static str) -> Arc<dyn ModelClient> {
    let mut client = MockModelClient::new();
    client.expect_complete().returning(move |_, _| {
        Ok(Completion {
            value: json!({ "class": class }),
            input_tokens: 30,
            output_tokens: 5,
            reported_cost_micros: None,
        })
    });
    Arc::new(client)
}

fn tool_jev(classifier: Arc<dyn Classifier>, label: Arc<dyn ModelClient>) -> Arc<ToolJev> {
    Arc::new(ToolJev {
        classifier,
        redactor: Redactor::new(vec![]),
        model: PINNED.into(),
        server_session_id: "server-1".into(),
        deadline: StdDuration::from_millis(GATE_BUDGET_MS),
        label_client: label,
        label_model: "claude-opus-4-8".into(),
    })
}

type Rows = Arc<Mutex<Vec<JevRecord>>>;

/// A builder that records every checkpoint and Jev row.
fn capturing(rows: &Rows) -> DepsBuilder {
    let mut b = DepsBuilder::new();
    b.storage.expect_record_checkpoint().returning(|_| Ok(()));
    let rows = Arc::clone(rows);
    b.storage.expect_record_jev().returning(move |r| {
        rows.lock().unwrap().extend_from_slice(r);
        Ok(())
    });
    b
}

/// Wait for the off-path Jev task to write its row.
async fn rows_after(rows: &Rows, n: usize) -> Vec<JevRecord> {
    for _ in 0..200 {
        if rows.lock().unwrap().len() >= n {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    rows.lock().unwrap().clone()
}

fn memory_embedder() -> Arc<dyn Embedder> {
    let mut embedder = MockEmbedder::new();
    embedder.expect_embed_query().returning(|_| {
        Ok(Embedding {
            vector: vec![1.0, 0.0],
            input_tokens: 6,
        })
    });
    Arc::new(embedder)
}

#[tokio::test]
async fn every_matched_call_is_classified_with_memory_off() {
    let rows = Rows::default();
    let requests = Arc::new(Mutex::new(vec![]));
    let mut b = capturing(&rows);
    b.tool_jev = Some(tool_jev(
        Arc::new(Answers("read_only", Arc::clone(&requests))),
        label_client("read_only"),
    ));
    let (result, _, _) = run_action(
        &b.build(),
        &action_params("Read", r#"{"file_path":"src/main.rs"}"#),
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.verdict, Verdict::Silence);
    let rows = rows_after(&rows, 1).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.site, JevSite::ToolRisk);
    assert_eq!(
        row.outcome,
        JevOutcome::Call(crate::error::Outcome::Success)
    );
    assert_eq!(row.model_answered.as_deref(), Some(PINNED));
    assert!(!row.applied);
    assert!(row.state_json.is_none());
    assert!(row.state_hash.is_some());
    assert!(row.eval_id.is_some());
    // The big model's label for the same projection.
    assert_eq!(row.label_json, Some(json!({ "class": "read_only" })));
    assert!(row
        .label_source
        .as_deref()
        .unwrap()
        .starts_with("claude-opus-4-8"));
    // The label call's cost, priced at the label model's rate.
    assert_eq!(
        row.label_cost_usd,
        Some(crate::telemetry::cost_usd("claude-opus-4-8", 30, 5))
    );
    // One attempt, deadline GATE_BUDGET_MS, the action_class question only.
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].retry);
    assert_eq!(
        requests[0].timeout,
        StdDuration::from_millis(GATE_BUDGET_MS)
    );
    let ids: Vec<&str> = requests[0]
        .questions
        .iter()
        .map(|q| q.id.as_str())
        .collect();
    assert_eq!(ids, ["action_class"]);
}

#[tokio::test]
async fn every_matched_call_is_classified_with_memory_on() {
    let rows = Rows::default();
    let requests = Arc::new(Mutex::new(vec![]));
    let mut b = capturing(&rows);
    b.embedder = Some(memory_embedder());
    b.tool_jev = Some(tool_jev(
        Arc::new(Answers("state_changing", Arc::clone(&requests))),
        label_client("state_changing"),
    ));
    run_action(
        &b.build(),
        &action_params("Write", r#"{"file_path":"src/lib.rs","content":"x"}"#),
        None,
    )
    .await
    .unwrap();
    let rows = rows_after(&rows, 1).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_newly_matched_tool_gets_no_hold_even_when_risk_matched() {
    let rows = Rows::default();
    let mut b = capturing(&rows);
    let mut embedder = MockEmbedder::new();
    embedder.expect_embed_query().times(0);
    b.embedder = Some(Arc::new(embedder));
    b.storage
        .expect_load_memories()
        .returning(|| Ok(vec![constraint_memory()]));
    let (result, _, _) = run_action(
        &b.build(),
        &action_params("Read", r#"{"file_path":"notes/deploy.md"}"#),
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.verdict, Verdict::Silence);
    assert!(!result.fail_open);
}

/// The Bash hold, for one classifier behavior.
async fn bash_hold_with(classifier: Arc<dyn Classifier>) -> CheckpointResult {
    let rows = Rows::default();
    let mut b = capturing(&rows);
    b.embedder = Some(memory_embedder());
    b.storage
        .expect_load_memories()
        .returning(|| Ok(vec![constraint_memory()]));
    b.tool_jev = Some(tool_jev(classifier, label_client("read_only")));
    let (result, _, _) = run_action(
        &b.build(),
        &action_params("Bash", r#"{"command":"deploy to production"}"#),
        None,
    )
    .await
    .unwrap();
    result
}

#[tokio::test]
async fn a_risk_matched_bash_hold_is_unchanged_whatever_jev_does() {
    let requests = Arc::new(Mutex::new(vec![]));
    for classifier in [
        Arc::new(Hanging) as Arc<dyn Classifier>,
        Arc::new(Failing),
        Arc::new(Answers("read_only", requests)),
    ] {
        let result = bash_hold_with(classifier).await;
        assert_eq!(result.verdict, Verdict::Hold);
        assert!(result.message.unwrap().contains("staging first"));
    }
}

#[tokio::test]
async fn a_never_returning_jev_does_not_delay_the_gate_response() {
    let rows = Rows::default();
    let mut b = capturing(&rows);
    b.tool_jev = Some(tool_jev(Arc::new(Hanging), label_client("read_only")));
    let deps = b.build();
    let params = action_params("Grep", r#"{"pattern":"fn main"}"#);
    // The response path never awaits the Jev request (SC-003).
    let response = tokio::time::timeout(
        StdDuration::from_millis(GATE_BUDGET_MS / 2),
        run_action(&deps, &params, None),
    )
    .await
    .expect("the gate answered without waiting for Jev")
    .unwrap();
    assert_eq!(response.0.verdict, Verdict::Silence);
}

#[tokio::test]
async fn a_jev_timeout_still_leaves_its_row() {
    let rows = Rows::default();
    let mut b = capturing(&rows);
    b.tool_jev = Some(tool_jev(Arc::new(Hanging), label_client("read_only")));
    run_action(
        &b.build(),
        &action_params("Grep", r#"{"pattern":"fn main"}"#),
        None,
    )
    .await
    .unwrap();
    let rows = rows_after(&rows, 1).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].outcome,
        JevOutcome::Call(crate::error::Outcome::Timeout)
    );
    assert!(rows[0].label_json.is_none());
}

/// The verdict and message, with the site off or in shadow.
async fn verdict(shadow: bool, tool: &str, input: &str) -> (Verdict, Option<String>) {
    let rows = Rows::default();
    let mut b = capturing(&rows);
    b.embedder = Some(memory_embedder());
    b.storage
        .expect_load_memories()
        .returning(|| Ok(vec![constraint_memory()]));
    if shadow {
        b.tool_jev = Some(tool_jev(
            Arc::new(Answers("spend_or_destructive", Arc::default())),
            label_client("spend_or_destructive"),
        ));
    }
    let (result, _, _) = run_action(&b.build(), &action_params(tool, input), None)
        .await
        .unwrap();
    (result.verdict, result.message)
}

#[tokio::test]
async fn the_verdict_and_message_are_identical_off_and_in_shadow() {
    for (tool, input) in [
        ("Bash", r#"{"command":"deploy to production"}"#),
        ("Bash", r#"{"command":"ls -la"}"#),
        ("Read", r#"{"file_path":"src/main.rs"}"#),
        ("mcp__exa__exa_search", r#"{"query":"rust"}"#),
    ] {
        assert_eq!(
            verdict(false, tool, input).await,
            verdict(true, tool, input).await,
            "{tool} {input}"
        );
    }
}

#[tokio::test]
async fn excluded_calls_leave_an_excluded_row_with_the_reason_and_no_request() {
    for (input, reason) in [
        (r#"{"command":"git push origin main"}"#, "risk_pattern"),
        (r#"{"command":"cargo test -j 2"}"#, "allowlist"),
    ] {
        let rows = Rows::default();
        let mut b = capturing(&rows);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        b.tool_jev = Some(tool_jev(
            Arc::new(Counting(Arc::clone(&calls))),
            label_client("read_only"),
        ));
        run_action(&b.build(), &action_params("Bash", input), None)
            .await
            .unwrap();
        let rows = rows_after(&rows, 1).await;
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{input}"
        );
        assert_eq!(rows.len(), 1, "{input}");
        assert_eq!(rows[0].outcome, JevOutcome::Excluded);
        assert_eq!(rows[0].error_class.as_deref(), Some(reason));
        assert!(rows[0].model_answered.is_none());
        assert!(rows[0].label_json.is_none());
    }
}

#[tokio::test]
async fn with_the_site_off_no_row_is_written() {
    let mut b = DepsBuilder::new();
    b.storage.expect_record_checkpoint().returning(|_| Ok(()));
    b.storage.expect_record_jev().times(0);
    run_action(
        &b.build(),
        &action_params("Read", r#"{"file_path":"src/main.rs"}"#),
        None,
    )
    .await
    .unwrap();
}
