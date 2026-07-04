//! The Decide corrective (013) — methodology-driven choice.
//!
//! For indecision and miscalibration: given a decision and ≥2 options, a single
//! stance-blind pass selects a decision **methodology** (weigh / causal /
//! probabilistic) and emits a numeric **score per option** (plus per-option
//! rationales and the deciding factors) as flat scalar arrays. The server zips
//! those with the option labels, ranks by score, and **deterministically**
//! assembles the recommendation: the top option, the runner-up and why it lost,
//! the surfaced methodology, the deciding factors, and a confidence derived from
//! the score *margin*. The model scores; the server chooses and calibrates — so
//! the choice traces to the factors, never an unexamined preference. Single pass
//! (closest to `unstick`); it does **not** use `verify`'s vote aggregation.

use crate::error::AppError;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::{Deserialize, Serialize};

/// Tool id as exposed over MCP.
pub const DECIDE_ID: &str = "decide";

/// The MCP tool description — the routing text (kept in sync with contracts/).
pub const DECIDE_DESCRIPTION: &str = "Choose among two or more options under tradeoffs, with the \
    reasoning shown. Applies an explicit decision methodology (weigh named criteria, trace what \
    each option causes, or reason under uncertainty), scores every option, and returns the \
    recommended option, the runner-up and why it lost, the deciding factors, the methodology \
    used, and a confidence calibrated to how close the call is. The choice is computed from the \
    scores, not asserted - never a menu handed back, never a hidden gut pick. To judge whether a \
    claim is true use verify; for one next step when you are looping use unstick; for a computable \
    comparison use check.";

/// The decision prompt. Placeholders exist for the decision, the options, and the
/// context ONLY — no slot for the caller's preferred option or stance (blindness
/// is structural). The model scores option *i* in the listed order.
const PROMPT_TEMPLATE: &str = "You are an external decision aid. Someone must choose among the \
    options below and is either stalling or about to pick on a gut feel. Assess the options \
    explicitly and score them; you do not pick the winner — the server does, from your scores.\n\
    \n\
    Rules:\n\
    1. Choose the decision methodology that fits this decision's shape and report it in \
    `methodology`: \"weigh\" (the decision turns on several named criteria), \"causal\" (it turns \
    on what each option causes or prevents downstream), or \"probabilistic\" (it turns on \
    uncertainty about which outcome obtains).\n\
    2. In `deciding_factors`, name the factors/criteria/effects/likelihoods your methodology uses \
    — the terms the decision actually turns on. Non-empty.\n\
    3. Score every option from 0 to 100 on its overall standing under your methodology, in the \
    SAME ORDER the options are listed: `option_scores[i]` and `option_rationales[i]` are for \
    option i. Provide exactly one score and one rationale per option. A score is an integer in \
    0–100.\n\
    4. `option_rationales[i]` is one self-contained sentence on why option i scored as it did, in \
    the methodology's terms.\n\
    5. Do not state a verdict, and do not name a single winner — only assess and score. The \
    higher score is the better option.\n\
    \n\
    Decision:\n<<decision>>\n\
    \n\
    Options (score in this order):\n<<options>>\n\
    \n\
    Context provided with the decision (may be empty):\n<<context>>\n<<violation_clause>>";

/// The decision methodology applied — a scalar enum (flat-legal, grammar-enforced;
/// the 011 H1 caveat is only about `Option<enum>`, not this required field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum Methodology {
    /// The decision turns on several named criteria.
    Weigh,
    /// The decision turns on what each option causes or prevents.
    Causal,
    /// The decision turns on uncertainty about which outcome obtains.
    Probabilistic,
}

/// Tool input: the decision, the candidate options (≥2), and optional context.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DecideParams {
    /// The question to settle, stated neutrally.
    pub decision: String,
    /// The candidate options — at least two. Order is the index basis for the
    /// per-option score arrays. May be omitted when `options_text` is supplied.
    #[serde(default)]
    pub options: Vec<String>,
    /// Options as a single newline-delimited string (one per line) — an alternative to the `options`
    /// array for clients that cannot reliably serialize a multi-element array argument. Used when
    /// `options` is empty.
    #[serde(default)]
    pub options_text: Option<String>,
    /// Optional neutral context/criteria — the only extra subject input.
    pub context: Option<String>,
}

impl DecideParams {
    /// Populate `options` from `options_text` (split on newlines, trimmed, blanks dropped) when `options`
    /// is empty. A no-op when `options` is already supplied.
    fn normalize(&mut self) {
        if self.options.is_empty() {
            if let Some(text) = self.options_text.take() {
                self.options = text
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
            }
        }
    }
}

/// What the single pass is grammar-constrained to produce (data-model.md).
///
/// A scalar enum plus three arrays of scalars (per-option data as parallel
/// arrays, since arrays of objects are illegal under the flat-schema gate). Flat
/// + closed.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DecidePass {
    /// The methodology the model applied.
    pub methodology: Methodology,
    /// Score per option (0–100), index-aligned to the input options.
    pub option_scores: Vec<i64>,
    /// Rationale per option, index-aligned to the input options.
    pub option_rationales: Vec<String>,
    /// The factors/criteria/effects/likelihoods the methodology used.
    pub deciding_factors: Vec<String>,
}

/// One option's server-assembled assessment — the model's score and rationale
/// paired with the option label.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct OptionAssessment {
    /// The option label (from the input).
    pub option: String,
    /// The model's score (0–100, validated).
    pub score: i64,
    /// The model's one-sentence rationale.
    pub rationale: String,
}

/// The aggregated tool output (data-model.md) — the server-derived recommendation
/// with its scored rationale. No verdict, no next step.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DecideResult {
    /// The top-scored option.
    pub recommended: String,
    /// The second-scored option.
    pub runner_up: String,
    /// Why the runner-up lost (server-composed: margin + its rationale).
    pub runner_up_reason: String,
    /// Confidence, server-derived from the score margin (`[0.5, 1.0]`).
    pub confidence: f64,
    /// The surfaced methodology.
    pub methodology: Methodology,
    /// The factors the methodology used.
    pub deciding_factors: Vec<String>,
    /// The full per-option breakdown, in input order (for audit).
    pub assessments: Vec<OptionAssessment>,
}

/// One Decide run: the result plus token usage for the invocation record.
#[derive(Debug)]
pub struct DecideRun {
    /// The server-assembled recommendation.
    pub result: DecideResult,
    /// Input tokens for the single pass.
    pub input_tokens: u64,
    /// Output tokens for the single pass.
    pub output_tokens: u64,
}

/// The score scale; the margin→confidence map uses it as the full range.
const SCALE: f64 = 100.0;

/// Initial pass + one violation-fed retry on a malformed assessment, mirroring
/// the deterministic layer's single retry (`TRANSLATION_ATTEMPTS_MAX`). The
/// grammar cannot constrain array length, so a transient arity slip is recovered
/// by feeding the exact rejection back once; a second malformed pass errors loud.
const DECIDE_ATTEMPTS_MAX: u32 = 2;

/// Register the decide mode (boot-time; enforces flat+closed). Single pass —
/// `ensemble_k = 1`.
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(DecidePass))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(DECIDE_ID, DECIDE_DESCRIPTION, PROMPT_TEMPLATE, schema, 1)
}

/// Build the prompt. Decision, the ordered options, and context are the only
/// subject content; there is no slot for a preferred option or stance. The
/// `violation` slot is empty on the first attempt and carries the prior pass's
/// rejection reason verbatim on the single retry (server-composed message, no
/// raw model text).
fn build_prompt(template: &str, params: &DecideParams, violation: Option<&str>) -> String {
    let options_text = params
        .options
        .iter()
        .enumerate()
        .map(|(i, opt)| format!("{}. {opt}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let violation_clause = violation.map_or(String::new(), |v| {
        format!(
            "\nYOUR PREVIOUS ASSESSMENT WAS REJECTED for this exact reason: {v}. Produce a \
             corrected assessment — exactly one score and one rationale per option, every score \
             an integer 0-100, and non-empty deciding_factors."
        )
    });
    template
        .replace("<<decision>>", &params.decision)
        .replace("<<options>>", &options_text)
        .replace("<<context>>", params.context.as_deref().unwrap_or(""))
        .replace("<<violation_clause>>", &violation_clause)
}

/// Validate input before any model call (FR-008): non-empty/non-oversize
/// decision, and **at least two** options. No fabricated comparison.
fn check_input(params: &DecideParams, max_chars: usize) -> Result<(), AppError> {
    if params.decision.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "decision is empty or whitespace-only".to_string(),
        ));
    }
    if params.options.len() < 2 {
        return Err(AppError::InvalidInput(format!(
            "decide requires at least two options; got {}",
            params.options.len()
        )));
    }
    let total = params.decision.chars().count()
        + params
            .options
            .iter()
            .map(|o| o.chars().count())
            .sum::<usize>()
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

/// Run one Decide invocation: a single stance-blind pass, then deterministic
/// validation + rank + calibrate.
///
/// # Errors
///
/// `InvalidInput` before any model call; `ValidationFailure` if the assessment
/// is still malformed after the single violation-fed retry (arity mismatch,
/// empty `deciding_factors`, or a score outside 0–100 — a failed pass, never
/// normalized); provider classes from the pass.
pub async fn run(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    params: &DecideParams,
    max_chars: usize,
) -> Result<DecideRun, AppError> {
    let mut owned = params.clone();
    owned.normalize(); // options_text -> options when the array argument was omitted
    let params = &owned;
    check_input(params, max_chars)?;

    let (mut input_tokens, mut output_tokens) = (0_u64, 0_u64);
    let mut violation: Option<String> = None;

    for _ in 1..=DECIDE_ATTEMPTS_MAX {
        let prompt = build_prompt(mode.prompt_template, params, violation.as_deref());
        let completion = client.complete(&prompt, &mode.sanitized_schema).await?;
        input_tokens += completion.input_tokens;
        output_tokens += completion.output_tokens;

        // The constrained-output contract itself (schema/shape) is a hard error,
        // not retried — only a well-formed-but-malformed assessment (the arity /
        // factors / score checks the grammar can't express) feeds the retry.
        validate(&mode.output_schema, &completion.value)?;
        let pass: DecidePass = serde_json::from_value(completion.value)
            .map_err(|e| AppError::ValidationFailure(format!("decide assessment shape: {e}")))?;

        match assemble(params, pass) {
            Ok(result) => {
                return Ok(DecideRun {
                    result,
                    input_tokens,
                    output_tokens,
                });
            }
            Err(AppError::ValidationFailure(v)) => violation = Some(v),
            Err(other) => return Err(other),
        }
    }

    Err(AppError::ValidationFailure(format!(
        "decide produced a malformed assessment after the retry; last violation: {}",
        violation.unwrap_or_else(|| "(none recorded)".to_string())
    )))
}

/// Validate well-formedness, then zip + rank + calibrate (research D2/D3).
/// A malformed assessment is a failed pass (loud) — arity mismatch, empty
/// `deciding_factors`, or a score outside 0–100 (analyze M1: never clamped).
fn assemble(params: &DecideParams, pass: DecidePass) -> Result<DecideResult, AppError> {
    let n = params.options.len();
    if pass.option_scores.len() != n || pass.option_rationales.len() != n {
        return Err(AppError::ValidationFailure(format!(
            "assessment arity mismatch: {} options but {} scores / {} rationales",
            n,
            pass.option_scores.len(),
            pass.option_rationales.len()
        )));
    }
    if pass.deciding_factors.iter().all(|f| f.trim().is_empty()) {
        return Err(AppError::ValidationFailure(
            "deciding_factors is empty — the methodology's terms are required".to_string(),
        ));
    }
    if let Some(bad) = pass
        .option_scores
        .iter()
        .find(|&&s| !(0..=100).contains(&s))
    {
        return Err(AppError::ValidationFailure(format!(
            "score {bad} is outside the 0-100 range — a malformed assessment is a failed pass"
        )));
    }

    // Per-option assessments in input order.
    let assessments: Vec<OptionAssessment> = params
        .options
        .iter()
        .zip(pass.option_scores.iter())
        .zip(pass.option_rationales.iter())
        .map(|((option, &score), rationale)| OptionAssessment {
            option: option.clone(),
            score,
            rationale: rationale.clone(),
        })
        .collect();

    // Rank by score descending; stable sort preserves input order on ties.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| assessments[b].score.cmp(&assessments[a].score));
    let top = &assessments[order[0]];
    let runner = &assessments[order[1]];

    let margin = top.score - runner.score;
    #[allow(clippy::cast_precision_loss)] // margin is 0..=100, exact in f64
    let margin_f = margin as f64;
    let confidence = (0.5 + 0.5 * margin_f.min(SCALE) / SCALE).clamp(0.5, 1.0);
    let runner_up_reason = format!("scored {margin} below {}: {}", top.option, runner.rationale);

    Ok(DecideResult {
        recommended: top.option.clone(),
        runner_up: runner.option.clone(),
        runner_up_reason,
        confidence,
        methodology: pass.methodology,
        deciding_factors: pass.deciding_factors,
        assessments,
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
        registry.get(DECIDE_ID).unwrap().clone()
    }

    fn params(decision: &str, options: &[&str]) -> DecideParams {
        DecideParams {
            decision: decision.to_string(),
            options: options.iter().map(|s| (*s).to_string()).collect(),
            options_text: None,
            context: None,
        }
    }

    /// A canned assessment body: methodology + parallel arrays.
    fn assessment(
        methodology: &str,
        scores: &[i64],
        rationales: &[&str],
        factors: &[&str],
    ) -> Value {
        json!({
            "methodology": methodology,
            "option_scores": scores,
            "option_rationales": rationales,
            "deciding_factors": factors,
        })
    }

    fn client_returning(value: Value) -> MockModelClient {
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(1).returning(move |_, _| {
            Ok(Completion {
                value: value.clone(),
                input_tokens: 90,
                output_tokens: 30,
            })
        });
        mock
    }

    /// A mock that returns the same (malformed) body on both attempts — the
    /// initial pass and the single retry — to exercise the post-retry failure.
    fn client_returning_twice(value: Value) -> MockModelClient {
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(2).returning(move |_, _| {
            Ok(Completion {
                value: value.clone(),
                input_tokens: 90,
                output_tokens: 30,
            })
        });
        mock
    }

    // ---- T005: schema, prompt, calibration (pure) --------------------------

    #[test]
    fn mode_registers_flat_closed_with_single_pass() {
        let mode = test_mode();
        assert_eq!(mode.ensemble_k, 1);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
        assert_eq!(
            mode.sanitized_schema["properties"]["methodology"]["enum"],
            json!(["weigh", "causal", "probabilistic"])
        );
        assert_eq!(
            mode.sanitized_schema["properties"]["option_scores"]["items"]["type"],
            json!("integer")
        );
    }

    #[test]
    fn prompt_has_only_subject_and_violation_slots_no_stance() {
        let p = params("pick one", &["A", "B"]);
        let prompt = build_prompt(PROMPT_TEMPLATE, &p, None);
        assert!(prompt.contains("1. A") && prompt.contains("2. B"));
        // The four slots: decision / options / context (subject) + the retry
        // violation feedback. No slot for a preferred option or stance.
        assert_eq!(PROMPT_TEMPLATE.matches("<<").count(), 4);
        assert!(
            PROMPT_TEMPLATE.contains("<<decision>>")
                && PROMPT_TEMPLATE.contains("<<options>>")
                && PROMPT_TEMPLATE.contains("<<context>>")
                && PROMPT_TEMPLATE.contains("<<violation_clause>>")
        );
        // First attempt carries no violation text.
        assert!(!prompt.contains("WAS REJECTED"));
    }

    #[tokio::test]
    async fn dominant_winner_is_recommended_with_high_confidence() {
        let mode = test_mode();
        let mock = client_returning(assessment(
            "weigh",
            &[85, 40],
            &["safe and reversible", "fast but risky"],
            &["risk", "speed"],
        ));
        let out = run(
            &mock,
            &mode,
            &params("how to ship?", &["ramp", "big-bang"]),
            50_000,
        )
        .await
        .unwrap()
        .result;
        assert_eq!(out.recommended, "ramp");
        assert_eq!(out.runner_up, "big-bang");
        // margin 45 → 0.5 + 0.5*45/100 = 0.725
        assert!((out.confidence - 0.725).abs() < 1e-9);
        assert!(out.runner_up_reason.contains("45 below ramp"));
        assert_eq!(out.assessments.len(), 2);
    }

    #[tokio::test]
    async fn near_tie_is_low_confidence() {
        let mode = test_mode();
        let mock = client_returning(assessment("weigh", &[60, 55], &["a", "b"], &["cost"]));
        let out = run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
            .await
            .unwrap()
            .result;
        // margin 5 → 0.525
        assert!((out.confidence - 0.525).abs() < 1e-9);
    }

    #[tokio::test]
    async fn exact_tie_resolves_by_input_order_at_floor_confidence() {
        let mode = test_mode();
        let mock = client_returning(assessment("causal", &[70, 70], &["a", "b"], &["effect"]));
        let out = run(&mock, &mode, &params("d", &["first", "second"]), 50_000)
            .await
            .unwrap()
            .result;
        assert_eq!(out.recommended, "first"); // input order wins the tie
        assert!((out.confidence - 0.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn methodology_is_surfaced_unchanged() {
        // T007 / US2: the surfaced methodology echoes the model's choice.
        for m in ["weigh", "causal", "probabilistic"] {
            let mode = test_mode();
            let mock = client_returning(assessment(m, &[80, 20], &["a", "b"], &["f"]));
            let out = run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
                .await
                .unwrap()
                .result;
            assert_eq!(serde_json::to_value(out.methodology).unwrap(), json!(m));
        }
    }

    // ---- T006: validation (well-formedness, scope, input) ------------------

    #[tokio::test]
    async fn fewer_than_two_options_is_rejected_before_any_model_call() {
        let mode = test_mode();
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);
        let err = run(&mock, &mode, &params("d", &["only one"]), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
    }

    #[tokio::test]
    async fn empty_decision_is_rejected_before_any_model_call() {
        let mode = test_mode();
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);
        let err = run(&mock, &mode, &params("  ", &["a", "b"]), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn arity_mismatch_after_retry_is_a_failed_pass() {
        let mode = test_mode();
        // 2 options but 3 scores — malformed on both the pass and the retry.
        let mock =
            client_returning_twice(assessment("weigh", &[80, 50, 30], &["a", "b", "c"], &["f"]));
        let err = run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err.to_string().contains("arity"));
        assert!(err.to_string().contains("after the retry"));
    }

    #[tokio::test]
    async fn out_of_range_score_after_retry_is_a_failed_pass_not_clamped() {
        let mode = test_mode();
        let mock = client_returning_twice(assessment("weigh", &[105, 40], &["a", "b"], &["f"]));
        let err = run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err.to_string().contains("0-100"));
        assert!(err.to_string().contains("after the retry"));
    }

    #[tokio::test]
    async fn empty_deciding_factors_after_retry_is_a_failed_pass() {
        let mode = test_mode();
        let mock = client_returning_twice(assessment("weigh", &[80, 40], &["a", "b"], &[]));
        let err = run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err.to_string().contains("deciding_factors"));
        assert!(err.to_string().contains("after the retry"));
    }

    #[tokio::test]
    async fn a_malformed_assessment_triggers_one_retry_then_succeeds() {
        let mode = test_mode();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(2).returning(move |_, _| {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let value = if n == 0 {
                // First pass: 2 options but 3 scores → rejected.
                assessment("weigh", &[80, 50, 30], &["a", "b", "c"], &["f"])
            } else {
                // Retry: well-formed.
                assessment("weigh", &[80, 40], &["a", "b"], &["f"])
            };
            Ok(Completion {
                value,
                input_tokens: 10,
                output_tokens: 5,
            })
        });
        let out = run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
            .await
            .unwrap();
        assert_eq!(out.result.recommended, "x");
        // Both attempts are metered.
        assert_eq!(out.input_tokens, 20);
        assert_eq!(out.output_tokens, 10);
    }

    #[tokio::test]
    async fn the_retry_prompt_carries_the_violation_verbatim() {
        let mode = test_mode();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(2).returning(move |prompt, _| {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let value = if n == 0 {
                assert!(
                    !prompt.contains("WAS REJECTED"),
                    "first prompt is clean: {prompt}"
                );
                assessment("weigh", &[80, 50, 30], &["a", "b", "c"], &["f"])
            } else {
                assert!(
                    prompt.contains("WAS REJECTED") && prompt.contains("arity"),
                    "retry must carry the violation: {prompt}"
                );
                assessment("weigh", &[80, 40], &["a", "b"], &["f"])
            };
            Ok(Completion {
                value,
                input_tokens: 1,
                output_tokens: 1,
            })
        });
        run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
            .await
            .unwrap();
    }

    #[test]
    fn margin_to_confidence_map() {
        // Build assessments directly and exercise the calibration via assemble.
        let p = params("d", &["x", "y"]);
        let mk = |a: i64, b: i64| {
            assemble(
                &p,
                DecidePass {
                    methodology: Methodology::Weigh,
                    option_scores: vec![a, b],
                    option_rationales: vec!["a".into(), "b".into()],
                    deciding_factors: vec!["f".into()],
                },
            )
            .unwrap()
            .confidence
        };
        assert!((mk(50, 50) - 0.5).abs() < f64::EPSILON); // margin 0
        assert!((mk(75, 25) - 0.75).abs() < 1e-9); // margin 50
        assert!((mk(100, 0) - 1.0).abs() < f64::EPSILON); // margin 100
    }
}
