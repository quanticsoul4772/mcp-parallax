//! The Decide corrective (013) — methodology-driven choice.
//!
//! For indecision and gut-feel choices: given a decision and 2 to 10 options, a
//! single stance-blind pass selects a decision **methodology** (weigh / causal /
//! probabilistic) and emits a numeric **score per option** (plus per-option
//! rationales and the deciding factors), one named field per option position
//! (056; parallel arrays before it, which the grammar could not keep the same
//! length). The server pairs those with the option labels, ranks by score, and
//! **deterministically**
//! assembles the recommendation: the top option, the runner-up and why it lost,
//! the surfaced methodology, the deciding factors, and a confidence derived from
//! the score *margin*. The model scores; the server ranks and reports — so
//! the choice traces to the factors, never an unexamined preference. Single pass
//! (closest to `unstick`); it does **not** use `verify`'s vote aggregation.

use crate::error::AppError;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Tool id as exposed over MCP.
pub const DECIDE_ID: &str = "decide";

/// Fewest options decide accepts.
pub const DECIDE_OPTIONS_MIN: usize = 2;

/// Most options decide accepts (056). One output format is registered per
/// count, so the cap bounds the format at 20 per-option fields.
pub const DECIDE_OPTIONS_MAX: usize = 10;

/// The MCP tool description — the routing text (kept in sync with contracts/).
pub const DECIDE_DESCRIPTION: &str = "Choose among two or more options under tradeoffs, with the \
    reasoning shown. Applies an explicit decision methodology (weigh named criteria, trace what \
    each option causes, or reason under uncertainty), scores every option, and returns the \
    recommended option, the runner-up and why it lost, the deciding factors, the methodology \
    used, and a confidence. Confidence restates this run's top-two score gap (0.5 + gap / 200). \
    Scores come from one pass: an identical call can give different scores and a different winner \
    (seen on one input with leads of 38 points one way and 33 the other), and the winner has \
    changed when one sentence of context was removed and when the options were reordered. Report \
    the lead in points alongside the recommendation, and say when it is small. The choice is \
    computed from the scores, not asserted - never a menu handed back, never a hidden gut pick. \
    To judge whether a claim is true use verify; for one next step when you are looping use \
    unstick; for a computable comparison use check.";

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
    3. Score every option from 0 to 100 on its overall standing under your methodology: \
    `option_1_score` and `option_1_rationale` are for option 1 as listed, `option_2_score` and \
    `option_2_rationale` for option 2, and so on for every option. A score is an integer in \
    0–100.\n\
    4. Each `option_<i>_rationale` is one self-contained sentence on why option i scored as it \
    did, in the methodology's terms.\n\
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
    // Also read under the name `question`, which callers guess when the
    // schema is not in front of them. Deliberately not in the description:
    // the published schema requires `decision`, and advertising a second
    // name there would contradict it.
    #[serde(alias = "question")]
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
    /// Reasoning effort for this call alone. Omit to use the deployment's
    /// configured level. Not every model family accepts this; when one rejects
    /// it the error names the model, the level, and the remedies.
    ///
    /// Typed rather than a free string so the accepted levels appear in the
    /// published schema — the consumer of this server is a model reading that
    /// schema, and a prose-only list is not something it can be constrained by.
    #[serde(default)]
    pub effort: Option<crate::routing::Effort>,
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

/// One pass's assessment, read from the per-option fields by position.
///
/// The model-facing format (`pass_schema`) names one score and one rationale
/// field per option, so these vectors always have one entry per option: the
/// format, not a later check, guarantees the length (056).
#[derive(Debug, Clone)]
pub struct DecidePass {
    /// The methodology the model applied.
    pub methodology: Methodology,
    /// Score per option (0–100), in input order.
    pub option_scores: Vec<i64>,
    /// Rationale per option, in input order.
    pub option_rationales: Vec<String>,
    /// The factors/criteria/effects/likelihoods the methodology used.
    pub deciding_factors: Vec<String>,
}

/// The model-facing format for `n` options: methodology, deciding factors, and
/// one required `option_<i>_score` (integer) and `option_<i>_rationale`
/// (string) per position. Flat + closed; registered for every accepted count.
fn pass_schema(n: usize) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = vec![json!("methodology"), json!("deciding_factors")];
    properties.insert(
        "methodology".into(),
        json!({
            "description": "The methodology the model applied.",
            "type": "string",
            "enum": ["weigh", "causal", "probabilistic"]
        }),
    );
    properties.insert(
        "deciding_factors".into(),
        json!({
            "description": "The factors/criteria/effects/likelihoods the methodology used.",
            "type": "array",
            "items": { "type": "string" }
        }),
    );
    for i in 1..=n {
        let score = format!("option_{i}_score");
        let rationale = format!("option_{i}_rationale");
        properties.insert(
            score.clone(),
            json!({ "description": format!("Score for option {i} (0–100)."), "type": "integer" }),
        );
        properties.insert(
            rationale.clone(),
            json!({
                "description": format!("One sentence on why option {i} scored as it did."),
                "type": "string"
            }),
        );
        required.push(json!(score));
        required.push(json!(rationale));
    }
    json!({
        "description": "What the single pass is grammar-constrained to produce: one named score and rationale field per option (056).",
        "type": "object",
        "properties": properties,
        "required": required
    })
}

/// Read a validated answer for `n` options into a [`DecidePass`].
fn parse_pass(value: &Value, n: usize) -> Result<DecidePass, AppError> {
    let shape =
        |what: String| AppError::ValidationFailure(format!("decide assessment shape: {what}"));
    let methodology: Methodology = serde_json::from_value(value["methodology"].clone())
        .map_err(|e| shape(format!("methodology: {e}")))?;
    let deciding_factors: Vec<String> = serde_json::from_value(value["deciding_factors"].clone())
        .map_err(|e| shape(format!("deciding_factors: {e}")))?;
    let mut option_scores = Vec::with_capacity(n);
    let mut option_rationales = Vec::with_capacity(n);
    for i in 1..=n {
        let score = value[format!("option_{i}_score")]
            .as_i64()
            .ok_or_else(|| shape(format!("option_{i}_score is missing or not an integer")))?;
        let rationale = value[format!("option_{i}_rationale")]
            .as_str()
            .ok_or_else(|| shape(format!("option_{i}_rationale is missing or not a string")))?;
        option_scores.push(score);
        option_rationales.push(rationale.to_string());
    }
    Ok(DecidePass {
        methodology,
        option_scores,
        option_rationales,
        deciding_factors,
    })
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
    /// This run's top-two score gap rescaled to [0.5, 1.0] (0.5 + gap / 200). An identical call can score differently.
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
/// grammar cannot constrain score ranges or a non-empty factor list, so such a
/// slip is recovered by feeding the exact rejection back once; a second
/// malformed pass errors loud.
const DECIDE_ATTEMPTS_MAX: u32 = 2;

/// Register the decide mode (boot-time; enforces flat+closed for every
/// accepted option count). Single pass — `ensemble_k = 1`.
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    registry.register_counted(
        DECIDE_ID,
        DECIDE_DESCRIPTION,
        PROMPT_TEMPLATE,
        DECIDE_OPTIONS_MIN..=DECIDE_OPTIONS_MAX,
        &pass_schema,
        1,
    )
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
             corrected assessment — every score an integer 0-100, and non-empty deciding_factors."
        )
    });
    template
        .replace("<<decision>>", &params.decision)
        .replace("<<options>>", &options_text)
        .replace("<<context>>", params.context.as_deref().unwrap_or(""))
        .replace("<<violation_clause>>", &violation_clause)
}

/// Validate input before any model call (FR-008): non-empty/non-oversize
/// decision, and **two to ten** options. No fabricated comparison.
fn check_input(params: &DecideParams, max_chars: usize) -> Result<(), AppError> {
    if params.decision.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "the decision (the question to settle) is empty or whitespace-only".to_string(),
        ));
    }
    if params.options.len() < DECIDE_OPTIONS_MIN {
        return Err(AppError::InvalidInput(format!(
            "decide requires at least two options; got {}",
            params.options.len()
        )));
    }
    if params.options.len() > DECIDE_OPTIONS_MAX {
        return Err(AppError::InvalidInput(format!(
            "decide accepts at most {DECIDE_OPTIONS_MAX} options; got {}",
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
/// validation + rank + confidence.
///
/// # Errors
///
/// `InvalidInput` before any model call; `ValidationFailure` if the assessment
/// is still malformed after the single violation-fed retry (empty
/// `deciding_factors` or a score outside 0–100 — a failed pass, never
/// normalized); provider classes from the pass.
pub async fn run(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    params: &DecideParams,
    max_chars: usize,
) -> Result<DecideRun, AppError> {
    // 020: the retry means a failure can follow two complete model calls. The
    // accumulated tokens ride out on the error rather than being discarded
    // with it — "the assessment was malformed twice" is not "the call was
    // free".
    let mut spent = (0_u64, 0_u64);
    let outcome = run_spent(client, mode, params, max_chars, &mut spent).await;
    outcome.map_err(|error| error.metered(spent.0, spent.1))
}

async fn run_spent(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    params: &DecideParams,
    max_chars: usize,
    spent: &mut (u64, u64),
) -> Result<DecideRun, AppError> {
    let mut owned = params.clone();
    owned.normalize(); // options_text -> options when the array argument was omitted
    let params = &owned;
    check_input(params, max_chars)?;
    let n = params.options.len();
    let (output_schema, sanitized_schema) = mode.schemas_for(n).ok_or_else(|| {
        AppError::InvalidInput(format!("decide has no output format for {n} options"))
    })?;

    let (mut input_tokens, mut output_tokens) = (0_u64, 0_u64);
    let mut violation: Option<String> = None;

    for _ in 1..=DECIDE_ATTEMPTS_MAX {
        let prompt = build_prompt(mode.prompt_template, params, violation.as_deref());
        let completion = client.complete(&prompt, sanitized_schema).await?;
        input_tokens += completion.input_tokens;
        output_tokens += completion.output_tokens;
        *spent = (input_tokens, output_tokens);

        // The constrained-output contract itself (schema/shape) is a hard error,
        // not retried — only a well-formed-but-malformed assessment (the factors
        // / score checks the grammar can't express) feeds the retry.
        validate(output_schema, &completion.value)?;
        let pass = parse_pass(&completion.value, n)?;

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

/// Validate well-formedness, then pair + rank + confidence (research D2/D3).
/// A malformed assessment is a failed pass (loud) — empty `deciding_factors`
/// or a score outside 0–100 (analyze M1: never clamped). One score and
/// rationale per option is guaranteed by the per-option format (056).
fn assemble(params: &DecideParams, pass: DecidePass) -> Result<DecideResult, AppError> {
    let n = params.options.len();
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
            effort: None,
        }
    }

    /// A canned assessment body: methodology + parallel arrays.
    fn assessment(
        methodology: &str,
        scores: &[i64],
        rationales: &[&str],
        factors: &[&str],
    ) -> Value {
        // 056: the per-field shape — `option_<i>_score` / `option_<i>_rationale`.
        let mut answer = serde_json::Map::new();
        answer.insert("methodology".into(), json!(methodology));
        answer.insert("deciding_factors".into(), json!(factors));
        for (i, score) in scores.iter().enumerate() {
            answer.insert(format!("option_{}_score", i + 1), json!(score));
        }
        for (i, rationale) in rationales.iter().enumerate() {
            answer.insert(format!("option_{}_rationale", i + 1), json!(rationale));
        }
        Value::Object(answer)
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

    // ---- T005: schema, prompt, confidence (pure) ---------------------------

    #[test]
    fn mode_registers_flat_closed_with_single_pass() {
        let mode = test_mode();
        assert_eq!(mode.ensemble_k, 1);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
        assert_eq!(
            mode.sanitized_schema["properties"]["methodology"]["enum"],
            json!(["weigh", "causal", "probabilistic"])
        );
        // 056: per-option fields, one format per count; the mode's default
        // schema is the two-option one.
        assert_eq!(
            mode.sanitized_schema["properties"]["option_1_score"]["type"],
            json!("integer")
        );
        assert!(mode.sanitized_schema["properties"]
            .get("option_scores")
            .is_none());
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

    // ---- 056: per-option fields ------------------------------------------

    /// A per-field answer: methodology, factors, and `option_<i>_score` /
    /// `option_<i>_rationale` for each position.
    fn per_field_answer(scores: &[i64], factors: &[&str]) -> Value {
        let mut answer = serde_json::Map::new();
        answer.insert("methodology".into(), json!("weigh"));
        answer.insert("deciding_factors".into(), json!(factors));
        for (i, score) in scores.iter().enumerate() {
            answer.insert(format!("option_{}_score", i + 1), json!(score));
            answer.insert(
                format!("option_{}_rationale", i + 1),
                json!(format!("rationale {}", i + 1)),
            );
        }
        Value::Object(answer)
    }

    fn labels(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("option {i}")).collect()
    }

    /// T004(a): every accepted count has one required score and rationale
    /// field per position and no per-option lists.
    #[test]
    fn each_count_has_one_required_score_and_rationale_field_per_option() {
        let mode = test_mode();
        for n in 2..=10 {
            let (output, _) = mode
                .schemas_for(n)
                .expect("a format for every count 2..=10");
            let properties = output["properties"].as_object().unwrap();
            let required: Vec<&str> = output["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert!(!properties.contains_key("option_scores"));
            assert!(!properties.contains_key("option_rationales"));
            for field in ["methodology", "deciding_factors"] {
                assert!(required.contains(&field), "{n}: {field} not required");
            }
            for i in 1..=n {
                let score = format!("option_{i}_score");
                let rationale = format!("option_{i}_rationale");
                assert_eq!(properties[&score]["type"], json!("integer"), "{n}: {score}");
                assert_eq!(
                    properties[&rationale]["type"],
                    json!("string"),
                    "{n}: {rationale}"
                );
                assert!(
                    required.contains(&score.as_str()) && required.contains(&rationale.as_str())
                );
            }
            assert_eq!(properties.len(), 2 + 2 * n, "{n}: no other fields");
        }
        assert!(mode.schemas_for(1).is_none() && mode.schemas_for(11).is_none());
    }

    /// T004(b): a call sends the format registered for its option count.
    #[tokio::test]
    async fn a_call_sends_the_format_for_its_option_count() {
        let mode = test_mode();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let sink = std::sync::Arc::clone(&seen);
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(1).returning(move |_, schema| {
            sink.lock().unwrap().push(schema.clone());
            Ok(Completion {
                value: per_field_answer(&[70, 20, 40], &["cost"]),
                input_tokens: 90,
                output_tokens: 30,
            })
        });
        let options = labels(3);
        let refs: Vec<&str> = options.iter().map(String::as_str).collect();
        run(&mock, &mode, &params("d", &refs), 50_000)
            .await
            .unwrap();
        let (_, sanitized) = mode.schemas_for(3).unwrap();
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            std::slice::from_ref(sanitized)
        );
    }

    /// T004(b): the OpenAI-compatible backend receives the same format.
    #[tokio::test]
    async fn the_openai_compatible_backend_receives_the_same_format() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let body = json!({
            "choices": [{
                "message": { "content": per_field_answer(&[30, 60], &["f"]).to_string() },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        });
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = crate::client::openai_compat::OpenAiCompatClient::with_base_url(
            &crate::client::openai_compat::openai_test_config(),
            &server.uri(),
        )
        .with_backoff_base_ms(1);
        let mode = test_mode();
        run(&client, &mode, &params("d", &["a", "b"]), 50_000)
            .await
            .unwrap();
        let sent: Value = server.received_requests().await.unwrap()[0]
            .body_json()
            .unwrap();
        let (_, sanitized) = mode.schemas_for(2).unwrap();
        assert_eq!(sent["response_format"]["json_schema"]["schema"], *sanitized);
    }

    /// T004(c): a well-formed per-field answer gives one assessment per
    /// option, in input order.
    #[tokio::test]
    async fn a_per_field_answer_gives_one_assessment_per_option_in_order() {
        let mode = test_mode();
        let mock = client_returning(per_field_answer(&[10, 90, 50, 30], &["f"]));
        let options = labels(4);
        let refs: Vec<&str> = options.iter().map(String::as_str).collect();
        let result = run(&mock, &mode, &params("d", &refs), 50_000)
            .await
            .unwrap()
            .result;
        let got: Vec<(String, i64)> = result
            .assessments
            .iter()
            .map(|a| (a.option.clone(), a.score))
            .collect();
        assert_eq!(
            got,
            vec![
                ("option 1".into(), 10),
                ("option 2".into(), 90),
                ("option 3".into(), 50),
                ("option 4".into(), 30)
            ]
        );
        assert_eq!(result.recommended, "option 2");
    }

    /// T005: more than ten options is rejected before any model call; ten is
    /// accepted.
    #[tokio::test]
    async fn more_than_ten_options_is_rejected_and_ten_is_accepted() {
        let mode = test_mode();
        let mut none = MockModelClient::new();
        none.expect_complete().times(0);
        let eleven = labels(11);
        let refs: Vec<&str> = eleven.iter().map(String::as_str).collect();
        let err = run(&none, &mode, &params("d", &refs), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
        assert!(
            err.to_string().contains("11") && err.to_string().contains("10"),
            "{err}"
        );

        let ten = labels(10);
        let refs: Vec<&str> = ten.iter().map(String::as_str).collect();
        let scores: Vec<i64> = (1..=10).map(|i| i * 9).collect();
        let mock = client_returning(per_field_answer(&scores, &["f"]));
        run(&mock, &mode, &params("d", &refs), 50_000)
            .await
            .unwrap();
    }

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

    /// 056: the old arity case (more or fewer scores than options) can no
    /// longer be expressed. Its nearest successor — an answer missing a
    /// required per-option field — is a contract violation the validator
    /// rejects outright, as for any other schema breach (no retry).
    #[tokio::test]
    async fn a_missing_per_option_field_is_rejected_by_the_validator() {
        let mode = test_mode();
        // 2 options, but option 2's score and rationale are absent.
        let mock = client_returning(assessment("weigh", &[80], &["a"], &["f"]));
        let err = run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
            .await
            .unwrap_err();
        assert!(
            matches!(err.root(), AppError::ValidationFailure(_)),
            "{err}"
        );
        // 020: the call was billed; a malformed answer is not free.
        assert!(err.billed().0 > 0, "spend lost on the failure path");
        assert!(!err.to_string().contains("arity"), "{err}");
    }

    #[tokio::test]
    async fn out_of_range_score_after_retry_is_a_failed_pass_not_clamped() {
        let mode = test_mode();
        let mock = client_returning_twice(assessment("weigh", &[105, 40], &["a", "b"], &["f"]));
        let err = run(&mock, &mode, &params("d", &["x", "y"]), 50_000)
            .await
            .unwrap_err();
        assert!(
            matches!(err.root(), AppError::ValidationFailure(_)),
            "{err}"
        );
        // 020: both attempts were billed; a malformed assessment is not free.
        assert!(err.billed().0 > 0, "spend lost on the failure path");
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
        assert!(
            matches!(err.root(), AppError::ValidationFailure(_)),
            "{err}"
        );
        // 020: both attempts were billed; a malformed assessment is not free.
        assert!(err.billed().0 > 0, "spend lost on the failure path");
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
                // First pass: a score outside 0–100 → rejected (056: was an
                // arity mismatch, which the per-option format rules out).
                assessment("weigh", &[180, 50], &["a", "b"], &["f"])
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
                // 056: an out-of-range score, not an arity mismatch.
                assessment("weigh", &[180, 50], &["a", "b"], &["f"])
            } else {
                assert!(
                    prompt.contains("WAS REJECTED") && prompt.contains("score 180"),
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
        // Build assessments directly and exercise the confidence via assemble.
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
    /// 038: the contract file and the derived schemas must agree.
    ///
    /// `decide` shipped without a contract, so for four of fifteen tools the
    /// constraint comparison 029 added had nothing to run against. That is the
    /// gap this closes: not that the schema is currently wrong — it is derived
    /// from the Rust types and validated at registration — but that a change to
    /// those types had no checked-in statement of intent to diff against.
    ///
    /// The file is a **baseline** captured from today's schemas, not a
    /// specification written ahead of the code. Its value starts here: the next
    /// change to the input or output surface fails until someone updates the
    /// contract deliberately.
    #[test]
    fn derived_schemas_match_the_contract_file() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../specs/013-decide-methodology/contracts/decide.tool.json"
        ))
        .unwrap();

        let input = serde_json::to_value(schemars::schema_for!(DecideParams)).unwrap();
        let props = |v: &Value| -> Vec<String> {
            v["properties"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default()
        };
        assert_eq!(
            props(&contract["inputSchema"]),
            props(&input),
            "decide input properties drifted from the contract"
        );
        assert_eq!(
            contract["inputSchema"]["required"], input["required"],
            "decide required inputs drifted"
        );
        crate::schema::assert_constraints_agree(&contract["inputSchema"], &input, "decide");

        let output = serde_json::to_value(schemars::schema_for!(DecideResult)).unwrap();
        assert_eq!(
            props(&contract["outputSchema"]),
            props(&output),
            "decide output properties drifted from the contract"
        );
        crate::schema::assert_constraints_agree(
            &contract["outputSchema"],
            &output,
            "decide output",
        );
    }

    /// Callers repeatedly wrote the question under `question` (7 calls in
    /// the calling agents' session transcripts, each failing with "missing
    /// field `decision`" before the server could record it), so that name is
    /// read too. Sending both is still an error, not a silent choice; the
    /// parser's wording is pinned so a change to it is noticed.
    #[test]
    fn question_is_accepted_as_a_second_name_for_decision() {
        let params: DecideParams = serde_json::from_value(serde_json::json!({
            "question": "Which one?",
            "options": ["a", "b"],
        }))
        .unwrap();
        assert_eq!(params.decision, "Which one?");

        let both = serde_json::from_value::<DecideParams>(serde_json::json!({
            "decision": "Which one?",
            "question": "Which other?",
            "options": ["a", "b"],
        }));
        let err = both.unwrap_err().to_string();
        assert_eq!(err, "duplicate field `decision`");
    }

    // ---- 054: what the description says, and what must not move -----------

    /// The prompt as built for one fixed input, the same with the retry
    /// wording, and the schema the request carries, as they were before 054.
    const PIN_PROMPT: &str = "You are an external decision aid. Someone must choose among the options below and is either stalling or about to pick on a gut feel. Assess the options explicitly and score them; you do not pick the winner — the server does, from your scores.\n\nRules:\n1. Choose the decision methodology that fits this decision's shape and report it in `methodology`: \"weigh\" (the decision turns on several named criteria), \"causal\" (it turns on what each option causes or prevents downstream), or \"probabilistic\" (it turns on uncertainty about which outcome obtains).\n2. In `deciding_factors`, name the factors/criteria/effects/likelihoods your methodology uses — the terms the decision actually turns on. Non-empty.\n3. Score every option from 0 to 100 on its overall standing under your methodology: `option_1_score` and `option_1_rationale` are for option 1 as listed, `option_2_score` and `option_2_rationale` for option 2, and so on for every option. A score is an integer in 0–100.\n4. Each `option_<i>_rationale` is one self-contained sentence on why option i scored as it did, in the methodology's terms.\n5. Do not state a verdict, and do not name a single winner — only assess and score. The higher score is the better option.\n\nDecision:\nWhich database?\n\nOptions (score in this order):\n1. Postgres\n2. SQLite\n\nContext provided with the decision (may be empty):\nSingle user, local.\n";
    const PIN_PROMPT_RETRY: &str = "You are an external decision aid. Someone must choose among the options below and is either stalling or about to pick on a gut feel. Assess the options explicitly and score them; you do not pick the winner — the server does, from your scores.\n\nRules:\n1. Choose the decision methodology that fits this decision's shape and report it in `methodology`: \"weigh\" (the decision turns on several named criteria), \"causal\" (it turns on what each option causes or prevents downstream), or \"probabilistic\" (it turns on uncertainty about which outcome obtains).\n2. In `deciding_factors`, name the factors/criteria/effects/likelihoods your methodology uses — the terms the decision actually turns on. Non-empty.\n3. Score every option from 0 to 100 on its overall standing under your methodology: `option_1_score` and `option_1_rationale` are for option 1 as listed, `option_2_score` and `option_2_rationale` for option 2, and so on for every option. A score is an integer in 0–100.\n4. Each `option_<i>_rationale` is one self-contained sentence on why option i scored as it did, in the methodology's terms.\n5. Do not state a verdict, and do not name a single winner — only assess and score. The higher score is the better option.\n\nDecision:\nWhich database?\n\nOptions (score in this order):\n1. Postgres\n2. SQLite\n\nContext provided with the decision (may be empty):\nSingle user, local.\n\nYOUR PREVIOUS ASSESSMENT WAS REJECTED for this exact reason: score 180 is outside the 0-100 range — a malformed assessment is a failed pass. Produce a corrected assessment — every score an integer 0-100, and non-empty deciding_factors.";
    const PIN_SCHEMA: &str = "{\"additionalProperties\":false,\"description\":\"What the single pass is grammar-constrained to produce: one named score and rationale field per option (056).\",\"properties\":{\"deciding_factors\":{\"description\":\"The factors/criteria/effects/likelihoods the methodology used.\",\"items\":{\"type\":\"string\"},\"type\":\"array\"},\"methodology\":{\"description\":\"The methodology the model applied.\",\"enum\":[\"weigh\",\"causal\",\"probabilistic\"],\"type\":\"string\"},\"option_1_rationale\":{\"description\":\"One sentence on why option 1 scored as it did.\",\"type\":\"string\"},\"option_1_score\":{\"description\":\"Score for option 1 (0–100).\",\"type\":\"integer\"},\"option_2_rationale\":{\"description\":\"One sentence on why option 2 scored as it did.\",\"type\":\"string\"},\"option_2_score\":{\"description\":\"Score for option 2 (0–100).\",\"type\":\"integer\"}},\"required\":[\"deciding_factors\",\"methodology\",\"option_1_rationale\",\"option_1_score\",\"option_2_rationale\",\"option_2_score\"],\"type\":\"object\"}";

    /// 054 changes words callers read, never what the model is sent. Nothing
    /// pinned the prompt text before (the existing prompt test only counts
    /// placeholders), and the retry wording lives outside the template, so
    /// all three are held here as literals.
    #[test]
    fn the_prompt_the_model_sees_is_unchanged_by_054() {
        let mode = test_mode();
        let mut p = params("Which database?", &["Postgres", "SQLite"]);
        p.context = Some("Single user, local.".to_string());
        // 056: the pins were re-taken for the per-option prompt and format; the
        // retry pin now carries a score violation (arity can no longer occur).
        let violation =
            "score 180 is outside the 0-100 range — a malformed assessment is a failed pass";
        assert_eq!(
            (
                build_prompt(mode.prompt_template, &p, None),
                build_prompt(mode.prompt_template, &p, Some(violation)),
                serde_json::to_string(&mode.sanitized_schema).unwrap(),
            ),
            (
                PIN_PROMPT.to_string(),
                PIN_PROMPT_RETRY.to_string(),
                PIN_SCHEMA.to_string(),
            )
        );
    }

    /// The description must say what was measured and must not claim the
    /// confidence is tuned to reliability. Every problem is collected, so one
    /// run shows all of them.
    #[test]
    fn decide_description_states_what_was_measured() {
        let required = [
            "Confidence restates this run's top-two score gap (0.5 + gap / 200).",
            "Scores come from one pass: an identical call can give different scores and a different winner (seen on one input with leads of 38 points one way and 33 the other), and the winner has changed when one sentence of context was removed and when the options were reordered.",
            "Report the lead in points alongside the recommendation, and say when it is small.",
        ];
        let mut problems = Vec::new();
        for sentence in required {
            let found = DECIDE_DESCRIPTION.matches(sentence).count();
            if found != 1 {
                problems.push(format!("found {found} times, expected once: {sentence}"));
            }
        }
        if DECIDE_DESCRIPTION.to_lowercase().contains("calibrat") {
            problems.push("contains the forbidden word stem".to_string());
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// The confidence field's description is what callers read beside the
    /// number. The contract test compares names and constraints only, so the
    /// text is held here: first to the specified wording, then to the
    /// contract file.
    #[test]
    fn decide_description_of_confidence_matches_the_contract() {
        let expected = "This run's top-two score gap rescaled to [0.5, 1.0] (0.5 + gap / 200). An identical call can score differently.";
        let derived = serde_json::to_value(schemars::schema_for!(DecideResult)).unwrap();
        let derived = derived["properties"]["confidence"]["description"]
            .as_str()
            .unwrap()
            .to_string();
        let contract: Value = serde_json::from_str(include_str!(
            "../../specs/013-decide-methodology/contracts/decide.tool.json"
        ))
        .unwrap();
        let contract = contract["outputSchema"]["properties"]["confidence"]["description"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(derived, expected, "the derived schema's description");
        assert_eq!(derived, contract, "the contract file's description");
    }
}
