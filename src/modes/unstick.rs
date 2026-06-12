//! The Unstick corrective.
//!
//! The Step primitive: one committed, externalized next step for a stuck
//! caller (design §4 — "stuck/looping → externalized structured step"). The
//! cheap workhorse: a single generation pass (research.md D1), with the
//! one-step guarantee split across schema shape, deterministic code checks,
//! and the calibrated prompt (research.md D2).

use crate::error::AppError;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::{Deserialize, Serialize};

/// Tool id as exposed over MCP.
pub const UNSTICK_ID: &str = "unstick";

/// The MCP tool description — does the routing work. Kept in sync with
/// `contracts/unstick.tool.json`.
pub const UNSTICK_DESCRIPTION: &str = "Break a stuck loop by committing to one concrete next \
    step. Call when you have a goal, you have tried things, and you are producing plausible \
    motion that goes nowhere. Provide the goal, where you are blocked, and what you already \
    tried; you get back exactly one immediately actionable step with a rationale - never a menu \
    of options, never a plan. An external frame breaks the loop you cannot see from inside.";

/// The calibrated one-step profile. Placeholders exist for goal, blocker, and
/// attempts ONLY — nothing else can flow through (structural blindness, as
/// with verify).
const PROMPT_TEMPLATE: &str = "You are an external unsticker. A worker is stuck on a task and \
    cannot see the way forward from inside their loop. Commit them to exactly ONE next step.\n\
    \n\
    Rules:\n\
    1. Exactly one step: a single concrete, immediately actionable action. Never offer \
    alternatives (\"either X or Y\"), never a multi-step plan, never \"consider...\".\n\
    2. Do not repeat anything from the already-tried list, and do not rephrase a tried item \
    as if it were new.\n\
    3. Prefer the step that produces the most INFORMATION about the blocker if the cause is \
    unclear; prefer the step that makes direct PROGRESS if the cause is clear.\n\
    4. The rationale is one or two sentences: why this step breaks the loop.\n\
    5. watch_for is the single most likely pitfall of the step, or null if none stands out.\n\
    \n\
    Goal:\n<<goal>>\n\
    \n\
    Where they are blocked:\n<<blocked>>\n\
    \n\
    Already tried:\n<<tried>>\n";

/// Tool input (data-model.md §2).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct UnstickParams {
    /// What you are ultimately trying to accomplish.
    pub goal: String,
    /// Where you are stuck right now - the immediate blocker, error, or dead end.
    pub blocked: String,
    /// Approaches already attempted. The returned step will not restate any of
    /// these.
    pub tried: Option<Vec<String>>,
}

/// Tool output — also the model-hop schema (single pass; data-model.md §3).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NextStep {
    /// Exactly one concrete, immediately actionable step. Never multiple
    /// alternatives, never a plan.
    pub next_step: String,
    /// Why this step breaks the loop, in one or two sentences.
    pub rationale: String,
    /// One pitfall likely to derail the step, or null.
    pub watch_for: Option<String>,
}

/// One Unstick run: the step plus token usage for the invocation record.
#[derive(Debug)]
pub struct UnstickRun {
    /// The committed step.
    pub step: NextStep,
    /// Input tokens for the single pass.
    pub input_tokens: u64,
    /// Output tokens for the single pass.
    pub output_tokens: u64,
}

/// Register the unstick mode (boot-time; enforces flat+closed). Single pass —
/// `ensemble_k = 1` (research.md D1).
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(NextStep))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(UNSTICK_ID, UNSTICK_DESCRIPTION, PROMPT_TEMPLATE, schema, 1)
}

/// Build the prompt. Goal, blocker, and the attempts list are the ONLY dynamic
/// content.
fn build_prompt(template: &str, params: &UnstickParams) -> String {
    let tried_text = match params.tried.as_deref() {
        None | Some([]) => "(nothing yet)".to_string(),
        Some(items) => items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    template
        .replace("<<goal>>", &params.goal)
        .replace("<<blocked>>", &params.blocked)
        .replace("<<tried>>", &tried_text)
}

/// Validate input before any model call (FR-006).
fn check_input(params: &UnstickParams, max_chars: usize) -> Result<(), AppError> {
    if params.goal.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "goal is empty or whitespace-only".to_string(),
        ));
    }
    if params.blocked.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "blocked is empty or whitespace-only".to_string(),
        ));
    }
    let total = params.goal.chars().count()
        + params.blocked.chars().count()
        + params
            .tried
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|t| t.chars().count())
            .sum::<usize>();
    if total > max_chars {
        return Err(AppError::InvalidInput(format!(
            "combined input is {total} characters; the configured maximum is {max_chars} \
             (VERIFY_MAX_CLAIM_CHARS); it was not trimmed"
        )));
    }
    Ok(())
}

/// Case-folded, whitespace-collapsed, punctuation-insensitive form for the
/// no-restatement check (research.md D2 — exact-normalized, same tradeoff as
/// verify's finding dedup).
fn normalize(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c.is_whitespace() {
                Some(' ')
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run one Unstick invocation: a single blind pass, then deterministic checks.
///
/// # Errors
///
/// `InvalidInput` before any model call; `ValidationFailure` when the response
/// violates the one-step rules a schema cannot express (empty step, or a
/// normalized restatement of a tried item).
pub async fn run(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    params: &UnstickParams,
    max_chars: usize,
) -> Result<UnstickRun, AppError> {
    check_input(params, max_chars)?;
    let prompt = build_prompt(mode.prompt_template, params);

    let completion = client.complete(&prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let step: NextStep = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("step shape: {e}")))?;

    // Deterministic one-step rules beyond what a flat schema can say (D2).
    if step.next_step.trim().is_empty() {
        return Err(AppError::ValidationFailure(
            "next_step is empty — a committed step is required".to_string(),
        ));
    }
    let step_normalized = normalize(&step.next_step);
    if let Some(tried) = params.tried.as_deref() {
        for item in tried {
            if !item.trim().is_empty() && normalize(item) == step_normalized {
                return Err(AppError::ValidationFailure(format!(
                    "next_step restates an already-tried item: {item:?}"
                )));
            }
        }
    }

    Ok(UnstickRun {
        step,
        input_tokens: completion.input_tokens,
        output_tokens: completion.output_tokens,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::{json, Value};

    fn test_mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        register(&mut registry).unwrap();
        registry.get(UNSTICK_ID).unwrap().clone()
    }

    fn params(goal: &str, blocked: &str, tried: Option<Vec<&str>>) -> UnstickParams {
        UnstickParams {
            goal: goal.to_string(),
            blocked: blocked.to_string(),
            tried: tried.map(|items| items.into_iter().map(String::from).collect()),
        }
    }

    fn ok_step(next_step: &str) -> Value {
        json!({ "next_step": next_step, "rationale": "breaks the loop", "watch_for": null })
    }

    fn client_returning(value: Value) -> MockModelClient {
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(1).returning(move |_, _| {
            Ok(Completion {
                value: value.clone(),
                input_tokens: 80,
                output_tokens: 25,
            })
        });
        mock
    }

    // ---- T003: schema/contract sync ----------------------------------------

    #[test]
    fn derived_schemas_match_the_contract_file() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../specs/002-unstick-mode/contracts/unstick.tool.json"
        ))
        .unwrap();

        let props = |schema: &Value| -> Vec<String> {
            schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };

        let input = serde_json::to_value(schemars::schema_for!(UnstickParams)).unwrap();
        assert_eq!(props(&input), props(&contract["inputSchema"]));
        assert_eq!(
            contract["inputSchema"]["required"],
            json!(["goal", "blocked"])
        );

        let output = serde_json::to_value(schemars::schema_for!(NextStep)).unwrap();
        assert_eq!(props(&output), props(&contract["outputSchema"]));
        // watch_for is nullable in both.
        assert_eq!(
            output["properties"]["watch_for"]["type"],
            contract["outputSchema"]["properties"]["watch_for"]["type"]
        );

        assert_eq!(contract["description"], UNSTICK_DESCRIPTION);
    }

    #[test]
    fn mode_registers_flat_closed_with_single_pass() {
        let mode = test_mode();
        assert_eq!(mode.ensemble_k, 1);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
    }

    // ---- structural blindness (as verify's T019) ----------------------------

    #[test]
    fn prompt_contains_inputs_verbatim_and_nothing_else() {
        let p = params("ship it", "tests are red", Some(vec!["rerun CI"]));
        let prompt = build_prompt(PROMPT_TEMPLATE, &p);
        let expected = PROMPT_TEMPLATE
            .replace("<<goal>>", "ship it")
            .replace("<<blocked>>", "tests are red")
            .replace("<<tried>>", "- rerun CI");
        assert_eq!(prompt, expected);
        assert_eq!(PROMPT_TEMPLATE.matches("<<").count(), 3);
    }

    #[test]
    fn empty_tried_renders_as_nothing_yet() {
        let p = params("g", "b", None);
        assert!(build_prompt(PROMPT_TEMPLATE, &p).contains("(nothing yet)"));
    }

    // ---- T002: input validation before any model call -----------------------

    #[tokio::test]
    async fn empty_goal_or_blocked_rejected_before_any_model_call() {
        let mode = test_mode();
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);

        let err = run(&mock, &mode, &params("  ", "blocked", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");

        let err = run(&mock, &mode, &params("goal", "\n", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
    }

    #[tokio::test]
    async fn oversized_combined_input_is_rejected_not_trimmed() {
        let mode = test_mode();
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);

        let big = "x".repeat(40);
        let err = run(
            &mock,
            &mode,
            &params(&big, &big, Some(vec![&big])),
            100, // 120 chars total > 100
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
        assert!(err.to_string().contains("120"));
    }

    // ---- T002: single pass, happy path, deterministic checks ----------------

    #[tokio::test]
    async fn happy_path_is_exactly_one_model_call_with_usage() {
        let mode = test_mode();
        let mock = client_returning(ok_step("Export the CI env vars locally and rerun"));

        let run_result = run(
            &mock,
            &mode,
            &params(
                "green CI",
                "two tests fail only on CI",
                Some(vec!["rerun CI"]),
            ),
            50_000,
        )
        .await
        .unwrap();
        assert_eq!(
            run_result.step.next_step,
            "Export the CI env vars locally and rerun"
        );
        assert_eq!(run_result.input_tokens, 80);
        assert_eq!(run_result.output_tokens, 25);
        // times(1) on the mock enforces the single pass (FR-007).
    }

    #[tokio::test]
    async fn restating_a_tried_item_is_a_validation_failure() {
        let mode = test_mode();
        // Same step modulo case/punctuation/whitespace.
        let mock = client_returning(ok_step("Re-run   the CI, job!"));

        let err = run(
            &mock,
            &mode,
            &params("g", "b", Some(vec!["rerun the CI job"])),
            50_000,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err.to_string().contains("restates"));
    }

    #[tokio::test]
    async fn blank_next_step_is_a_validation_failure() {
        let mode = test_mode();
        let mock = client_returning(ok_step("   "));

        let err = run(&mock, &mode, &params("g", "b", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
    }

    #[test]
    fn normalize_folds_case_whitespace_and_punctuation() {
        assert_eq!(normalize("Re-run   the CI, job!"), "rerun the ci job");
        assert_eq!(normalize("rerun the CI job"), "rerun the ci job");
        assert_ne!(
            normalize("rerun the CI job twice"),
            normalize("rerun the CI job")
        );
    }
}
