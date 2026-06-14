//! The Preference-Elicitation corrective (014) — the wrong-objective corrective.
//!
//! Run *before* the model commits: given a task and optional context, a single
//! stance-blind pass surfaces the **assumed objective**, the **governing
//! preferences** (each traced to its signal, revealed/verified > stated), the
//! **divergence points** where the assumed objective likely departs from the
//! user's real one, and a self-reported **signal level**. When memory is
//! configured, the server recalls relevant **trusted** stored preferences
//! (reusing [`crate::memory::tools::recall`]) and injects them into the prompt as
//! the revealed signal. The model produces the structured inference (flat
//! parallel arrays); the server validates, zips, and assembles. It **only
//! surfaces** — enforcement stays `checkpoint_action`'s job. Single pass; it does
//! not use `verify`'s vote aggregation.

use crate::error::AppError;
use crate::memory::contract::RecallParams;
use crate::memory::tools::{recall, MemoryDeps};
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::{Deserialize, Serialize};

/// Tool id as exposed over MCP.
pub const ELICIT_ID: &str = "elicit";

/// The MCP tool description — the routing text (kept in sync with contracts/).
pub const ELICIT_DESCRIPTION: &str = "Surface the objective you're about to pursue and the \
    preferences that should govern it, before you commit - the corrective for solving the assumed \
    problem instead of the user's real one. Returns the objective a surface reading would assume, \
    the governing preferences/constraints (each traced to its signal; revealed/stored ones \
    outrank merely stated ones), and the divergence points where the assumed objective likely \
    departs from the user's actual one - the questions worth resolving first. Inference, not \
    interrogation: with little signal it says so rather than inventing preferences. When memory \
    is configured it also consults your stored verified preferences. It surfaces only - it does \
    not block or modify anything (that is the checkpoint layer).";

/// The elicitation prompt. Placeholders exist for the task, the context, and the
/// server-fetched preferences ONLY — task/context are the only caller-prose
/// inputs (stance-blind); `<<preferences>>` is server-filled from recall.
const PROMPT_TEMPLATE: &str = "You are an external objective check. Before a worker commits to a \
    task, surface what objective they are about to pursue and what should actually govern it, so \
    they don't solve the assumed problem instead of the real one. You know nothing about who they \
    are beyond what is below.\n\
    \n\
    Rules:\n\
    1. `assumed_objective`: state, in one sentence, the objective a surface reading of the task \
    would silently commit to.\n\
    2. Governing preferences (parallel arrays, one entry each): `preference_texts[i]` a \
    preference/constraint that should shape the work; `preference_signals[i]` where you inferred \
    it from (the request, the context, or a stored preference); `preference_strengths[i]` is \
    \"revealed\" for a stored/verified or demonstrated preference, \"stated\" for one merely \
    asserted in the request. Revealed outranks stated.\n\
    3. Divergence points (parallel arrays): `divergence_questions[i]` an assumption in the assumed \
    objective that a signal calls into question - a question worth resolving before proceeding; \
    `divergence_signals[i]` the conflicting signal it cites. Only where a signal suggests a real \
    gap - do NOT manufacture doubt.\n\
    4. Inference, not interrogation: infer from the signals present; do NOT invent preferences or \
    questions with no signal. Set `signal_level` to \"low\" when you have little preference signal \
    (and return few/empty preferences and divergence), \"medium\"/\"high\" as it grows.\n\
    5. Do not judge truth, do not choose an option, do not recommend an action - only surface.\n\
    \n\
    Task the worker is about to do:\n<<task>>\n\
    \n\
    Context provided with the task (may be empty):\n<<context>>\n\
    \n\
    Stored preferences (server-supplied):\n<<preferences>>\n";

/// How much preference signal the model had — a scalar enum (flat-legal,
/// grammar-enforced; the 011 H1 caveat is only about `Option<enum>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum SignalLevel {
    /// Little/no preference signal — return few or none, fabricate nothing.
    Low,
    /// Some preference signal.
    Medium,
    /// Strong preference signal.
    High,
}

/// Tool input: the task plus optional neutral context.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ElicitParams {
    /// What the caller is about to do, stated neutrally.
    pub task: String,
    /// Optional neutral context — the only extra caller-prose input.
    pub context: Option<String>,
}

/// What the single pass is grammar-constrained to produce (data-model.md).
///
/// A string, a scalar enum, and five arrays of scalars (per-item data as parallel
/// arrays, since arrays of objects are illegal under the flat-schema gate). Flat +
/// closed.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ElicitPass {
    /// The objective a surface reading would commit to.
    pub assumed_objective: String,
    /// The governing preferences/constraints.
    pub preference_texts: Vec<String>,
    /// Where each preference was inferred from.
    pub preference_signals: Vec<String>,
    /// `"revealed"` or `"stated"` per preference (server-validated).
    pub preference_strengths: Vec<String>,
    /// The divergence questions worth resolving.
    pub divergence_questions: Vec<String>,
    /// The conflicting signal each divergence cites.
    pub divergence_signals: Vec<String>,
    /// The model's self-report of available preference signal.
    pub signal_level: SignalLevel,
}

/// One governing preference — server-zipped from the parallel arrays.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GoverningPreference {
    /// The preference/constraint.
    pub preference: String,
    /// Where it was inferred from.
    pub signal: String,
    /// `revealed` | `stated`.
    pub strength: String,
}

/// One divergence point — server-zipped from the parallel arrays.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DivergencePoint {
    /// The assumption worth resolving, as a question.
    pub question: String,
    /// The conflicting signal it cites.
    pub signal: String,
}

/// The aggregated tool output (data-model.md) — surfacing only, no enforcement.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ElicitResult {
    /// The surfaced assumed objective.
    pub assumed_objective: String,
    /// The governing preferences (may be empty — low signal).
    pub governing_preferences: Vec<GoverningPreference>,
    /// The divergence points (empty when signals are consistent).
    pub divergence_points: Vec<DivergencePoint>,
    /// The surfaced signal level.
    pub signal_level: SignalLevel,
    /// Whether stored preferences were consulted (memory configured).
    pub memory_consulted: bool,
}

/// One Elicit run: the result plus token usage for the invocation record.
#[derive(Debug)]
pub struct ElicitRun {
    /// The server-assembled surfacing.
    pub result: ElicitResult,
    /// Input tokens (recall embed + the inference pass).
    pub input_tokens: u64,
    /// Output tokens (the inference pass).
    pub output_tokens: u64,
}

/// Pre-filter recall width; the trust filter is applied before capping at
/// [`RECALL_LIMIT`] (analyze L1 — a trusted pref must not be crowded out).
const RECALL_WIDE: u32 = 20;
/// How many trusted stored preferences to inject after filtering.
const RECALL_LIMIT: usize = 5;

/// Register the elicit mode (boot-time; enforces flat+closed). Single pass —
/// `ensemble_k = 1`.
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(ElicitPass))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(ELICIT_ID, ELICIT_DESCRIPTION, PROMPT_TEMPLATE, schema, 1)
}

/// Build the prompt. Task and context are the only caller-prose inputs; the
/// preferences block is server-fetched (recall), not caller-asserted.
fn build_prompt(template: &str, params: &ElicitParams, preferences: &str) -> String {
    template
        .replace("<<task>>", &params.task)
        .replace("<<context>>", params.context.as_deref().unwrap_or(""))
        .replace("<<preferences>>", preferences)
}

/// Validate input before any model call (FR-008).
fn check_input(params: &ElicitParams, max_chars: usize) -> Result<(), AppError> {
    if params.task.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "task is empty or whitespace-only".to_string(),
        ));
    }
    let total = params.task.chars().count()
        + params
            .context
            .as_deref()
            .unwrap_or_default()
            .chars()
            .count();
    if total > max_chars {
        return Err(AppError::InvalidInput(format!(
            "combined input is {total} characters; the configured maximum is {max_chars} \
             (INPUT_MAX_CHARS); it was not trimmed"
        )));
    }
    Ok(())
}

/// Recall trusted stored preferences relevant to the task (research D2). Returns
/// the formatted preferences block and the recall's token usage. Filter-to-trusted
/// **before** capping at `RECALL_LIMIT` so a trusted pref is not crowded out.
async fn recalled_preferences(
    memory: &MemoryDeps,
    task: &str,
) -> Result<(String, u64, u64), AppError> {
    let params = RecallParams {
        query: task.to_string(),
        kind: None,
        limit: Some(RECALL_WIDE),
    };
    let (result, inp, out) = recall(memory, &params).await?;
    let trusted: Vec<String> = result
        .memories
        .into_iter()
        .filter(|m| m.trust.is_trusted())
        .take(RECALL_LIMIT)
        .map(|m| format!("- {}", m.content))
        .collect();
    let block = if trusted.is_empty() {
        "(no relevant stored preferences found)".to_string()
    } else {
        format!(
            "stored verified preferences (revealed signal — these outrank merely stated \
             ones):\n{}",
            trusted.join("\n")
        )
    };
    Ok((block, inp, out))
}

/// Run one Elicit invocation: optional recall, a single stance-blind pass, then
/// deterministic validation + zip + assembly.
///
/// # Errors
///
/// `InvalidInput` before any model call; `ValidationFailure` for a malformed
/// inference (arity mismatch or a bad `preference_strengths` value — a loud failed
/// pass, never normalized); provider classes from the recall or the pass.
pub async fn run(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    memory: Option<&MemoryDeps>,
    params: &ElicitParams,
    max_chars: usize,
) -> Result<ElicitRun, AppError> {
    check_input(params, max_chars)?;

    let (preferences, recall_in, recall_out, memory_consulted) = match memory {
        Some(deps) => {
            let (block, inp, out) = recalled_preferences(deps, &params.task).await?;
            (block, inp, out, true)
        }
        None => (
            "(no stored preferences — memory not configured)".to_string(),
            0,
            0,
            false,
        ),
    };

    let prompt = build_prompt(mode.prompt_template, params, &preferences);
    let completion = client.complete(&prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let pass: ElicitPass = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("elicit inference shape: {e}")))?;

    let result = assemble(pass, memory_consulted)?;
    Ok(ElicitRun {
        result,
        input_tokens: recall_in + completion.input_tokens,
        output_tokens: recall_out + completion.output_tokens,
    })
}

/// Validate well-formedness, then zip the parallel arrays into the output
/// (research D4). A malformed inference is a loud failed pass — arity mismatch or
/// a `preference_strengths` value that is not `"revealed"`/`"stated"` (013
/// convention). Empty arrays are valid (low signal — FR-005).
fn assemble(pass: ElicitPass, memory_consulted: bool) -> Result<ElicitResult, AppError> {
    let np = pass.preference_texts.len();
    if pass.preference_signals.len() != np || pass.preference_strengths.len() != np {
        return Err(AppError::ValidationFailure(format!(
            "preference arrays disagree: {np} texts, {} signals, {} strengths",
            pass.preference_signals.len(),
            pass.preference_strengths.len()
        )));
    }
    let nd = pass.divergence_questions.len();
    if pass.divergence_signals.len() != nd {
        return Err(AppError::ValidationFailure(format!(
            "divergence arrays disagree: {nd} questions, {} signals",
            pass.divergence_signals.len()
        )));
    }
    if let Some(bad) = pass
        .preference_strengths
        .iter()
        .find(|s| s.as_str() != "revealed" && s.as_str() != "stated")
    {
        return Err(AppError::ValidationFailure(format!(
            "preference strength {bad:?} is not \"revealed\" or \"stated\" — a malformed \
             inference is a failed pass"
        )));
    }

    let governing_preferences = pass
        .preference_texts
        .into_iter()
        .zip(pass.preference_signals)
        .zip(pass.preference_strengths)
        .map(|((preference, signal), strength)| GoverningPreference {
            preference,
            signal,
            strength,
        })
        .collect();
    let divergence_points = pass
        .divergence_questions
        .into_iter()
        .zip(pass.divergence_signals)
        .map(|(question, signal)| DivergencePoint { question, signal })
        .collect();

    Ok(ElicitResult {
        assumed_objective: pass.assumed_objective,
        governing_preferences,
        divergence_points,
        signal_level: pass.signal_level,
        memory_consulted,
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
        registry.get(ELICIT_ID).unwrap().clone()
    }

    fn params(task: &str, context: Option<&str>) -> ElicitParams {
        ElicitParams {
            task: task.to_string(),
            context: context.map(ToString::to_string),
        }
    }

    /// A canned inference body.
    fn inference(
        objective: &str,
        prefs: &[(&str, &str, &str)],
        divergences: &[(&str, &str)],
        signal: &str,
    ) -> Value {
        json!({
            "assumed_objective": objective,
            "preference_texts": prefs.iter().map(|p| p.0).collect::<Vec<_>>(),
            "preference_signals": prefs.iter().map(|p| p.1).collect::<Vec<_>>(),
            "preference_strengths": prefs.iter().map(|p| p.2).collect::<Vec<_>>(),
            "divergence_questions": divergences.iter().map(|d| d.0).collect::<Vec<_>>(),
            "divergence_signals": divergences.iter().map(|d| d.1).collect::<Vec<_>>(),
            "signal_level": signal,
        })
    }

    fn client_returning(value: Value) -> MockModelClient {
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(1).returning(move |_, _| {
            Ok(Completion {
                value: value.clone(),
                input_tokens: 120,
                output_tokens: 40,
            })
        });
        mock
    }

    // ---- T005: schema, prompt, assembly ------------------------------------

    #[test]
    fn mode_registers_flat_closed_single_pass() {
        let mode = test_mode();
        assert_eq!(mode.ensemble_k, 1);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
        assert_eq!(
            mode.sanitized_schema["properties"]["signal_level"]["enum"],
            json!(["low", "medium", "high"])
        );
        assert_eq!(
            mode.sanitized_schema["properties"]["preference_texts"]["items"]["type"],
            json!("string")
        );
    }

    #[test]
    fn prompt_has_only_task_context_preferences_slots() {
        assert_eq!(PROMPT_TEMPLATE.matches("<<").count(), 3);
        assert!(
            PROMPT_TEMPLATE.contains("<<task>>")
                && PROMPT_TEMPLATE.contains("<<context>>")
                && PROMPT_TEMPLATE.contains("<<preferences>>")
        );
    }

    #[tokio::test]
    async fn surfaces_objective_and_traced_preferences() {
        let mode = test_mode();
        let mock = client_returning(inference(
            "Add a cache to speed up the endpoint",
            &[
                ("minimal new infra", "stored memory", "revealed"),
                ("p99 latency target", "the request", "stated"),
            ],
            &[],
            "medium",
        ));
        let out = run(
            &mock,
            &mode,
            None,
            &params("speed up the endpoint", None),
            50_000,
        )
        .await
        .unwrap()
        .result;
        assert_eq!(
            out.assumed_objective,
            "Add a cache to speed up the endpoint"
        );
        assert_eq!(out.governing_preferences.len(), 2);
        assert_eq!(out.governing_preferences[0].strength, "revealed");
        assert_eq!(out.governing_preferences[0].signal, "stored memory");
        assert!(!out.memory_consulted); // no memory passed
                                        // surfacing only — the struct has no enforcement field by construction.
    }

    #[tokio::test]
    async fn divergence_points_zip_and_empty_when_consistent() {
        let mode = test_mode();
        let with = client_returning(inference(
            "obj",
            &[],
            &[(
                "is a cache the goal, or lower p99?",
                "stored 'avoid infra' vs 'add cache'",
            )],
            "medium",
        ));
        let out = run(&with, &mode, None, &params("t", None), 50_000)
            .await
            .unwrap()
            .result;
        assert_eq!(out.divergence_points.len(), 1);
        assert!(out.divergence_points[0].question.contains("p99"));

        let none = client_returning(inference("obj", &[], &[], "high"));
        let out = run(&none, &mode, None, &params("t", None), 50_000)
            .await
            .unwrap()
            .result;
        assert!(out.divergence_points.is_empty());
    }

    #[tokio::test]
    async fn low_signal_returns_nothing_fabricated() {
        let mode = test_mode();
        let mock = client_returning(inference("Rename tmp to a clearer name", &[], &[], "low"));
        let out = run(&mock, &mode, None, &params("rename tmp", None), 50_000)
            .await
            .unwrap()
            .result;
        assert_eq!(out.signal_level, SignalLevel::Low);
        assert!(out.governing_preferences.is_empty());
        assert!(out.divergence_points.is_empty());
    }

    #[tokio::test]
    async fn empty_task_is_rejected_before_any_model_call() {
        let mode = test_mode();
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);
        let err = run(&mock, &mode, None, &params("  ", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn preference_arity_mismatch_is_a_failed_pass() {
        let mode = test_mode();
        // 2 texts but 1 signal.
        let mock = client_returning(json!({
            "assumed_objective": "o",
            "preference_texts": ["a", "b"],
            "preference_signals": ["s"],
            "preference_strengths": ["revealed", "stated"],
            "divergence_questions": [],
            "divergence_signals": [],
            "signal_level": "medium",
        }));
        let err = run(&mock, &mode, None, &params("t", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err.to_string().contains("preference arrays"));
    }

    #[tokio::test]
    async fn bad_strength_value_is_a_failed_pass() {
        let mode = test_mode();
        let mock = client_returning(inference("o", &[("p", "s", "guessed")], &[], "medium"));
        let err = run(&mock, &mode, None, &params("t", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err.to_string().contains("revealed"));
    }

    #[tokio::test]
    async fn divergence_arity_mismatch_is_a_failed_pass() {
        let mode = test_mode();
        let mock = client_returning(json!({
            "assumed_objective": "o",
            "preference_texts": [],
            "preference_signals": [],
            "preference_strengths": [],
            "divergence_questions": ["q1", "q2"],
            "divergence_signals": ["s1"],
            "signal_level": "medium",
        }));
        let err = run(&mock, &mode, None, &params("t", None), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err.to_string().contains("divergence arrays"));
    }
}
