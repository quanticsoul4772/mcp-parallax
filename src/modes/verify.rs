//! The Verify corrective.
//!
//! k independent, stance-blind verification passes with agreement-derived
//! confidence (research.md D4 — the shape the spike validated: k=3 parallel
//! was immune to pushback where a sequential critic caved; the calibrated
//! profile moved false positives 1/6 → 0/6).

use crate::error::AppError;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::{Deserialize, Serialize};

/// Tool id as exposed over MCP.
pub const VERIFY_ID: &str = "verify";

/// The MCP tool description — this does the routing work (the client selects
/// by description). Kept in sync with `contracts/verify.tool.json`.
pub const VERIFY_DESCRIPTION: &str = "Independently verify a claim. Runs multiple parallel \
    verification passes that see only the claim and optional context - never the requester's \
    stance or conversation. Returns a structured verdict: supported or refuted, specific \
    concrete findings (every refutation names the exact error), and a confidence score derived \
    from cross-pass agreement. Use when an assertion matters and being confidently wrong is \
    costly.";

/// The calibrated verifier profile (design §5): every refutation must name a
/// specific concrete error, and the claim is steelmanned before judgment.
/// Placeholders exist for the **lens** (a fixed critical perspective, not
/// caller-supplied), the claim, and the context ONLY — the requester's stance,
/// history, and identity have no slot to flow through (blindness is
/// structural, not behavioral). The `<<lens>>` slot carries one of [`LENSES`],
/// assigned per pass so the `k` passes scrutinize differently and genuine
/// disagreement can surface (US1; design §"Designing real independence").
const PROMPT_TEMPLATE: &str = "You are an independent verifier. Judge the claim below on its \
    own merits. You know nothing about who made it or how confident they are.\n\
    \n\
    Critical lens for this pass — it shapes HOW you scrutinize the claim, not what you assume \
    about who asked:\n<<lens>>\n\
    \n\
    Rules:\n\
    1. Steelman first: consider the strongest reasonable reading of the claim before judging.\n\
    2. Refute only on specific, concrete error: every finding in a refutation must name the \
    exact error and the correct fact. Vague doubt (\"may be inaccurate\") is not a finding.\n\
    3. If the claim is sound under its strongest reading, support it. Do not invent \
    refutations to appear rigorous.\n\
    4. Findings must be self-contained single sentences.\n\
    \n\
    Claim to verify:\n<<claim>>\n\
    \n\
    Context provided with the claim (may be empty):\n<<context>>\n";

/// A named critical perspective assigned to a verification pass (data-model.md
/// §Lens). The lens changes *how* a pass probes the claim — never *what it
/// knows about the asker* — so stance-blindness is preserved (research D3).
#[derive(Debug, Clone, Copy)]
struct Lens {
    /// Short identifier, prepended to the directive so the pass names its angle.
    name: &'static str,
    /// The one-paragraph instruction injected at the `<<lens>>` slot.
    directive: &'static str,
}

/// The fixed lens set (research D1). Pass `i` uses `LENSES[i % LENSES.len()]`
/// (research D2): with the default `k=3` the first three run; higher `k` cycles
/// deterministically. These are critical angles, not the caller's stance — the
/// agreement ratio only means something if the passes could genuinely disagree.
const LENSES: &[Lens] = &[
    Lens {
        name: "literal",
        directive: "Read the claim at face value. Take the plain, ordinary meaning of its \
            words and judge whether that literal reading holds — do not silently repair it \
            into a more defensible claim than what was written.",
    },
    Lens {
        name: "counterexample",
        directive: "Actively hunt for a single case, edge condition, or boundary where the \
            claim fails. If you can name one concrete counterexample, the claim as stated is \
            refuted; if a determined search finds none, that supports it.",
    },
    Lens {
        name: "definitional",
        directive: "Scrutinize the key terms. Is the claim true only under a loose, shifted, \
            or idiosyncratic definition of its words, and false under the standard one? \
            Pin down what each term must mean for the claim to hold.",
    },
    Lens {
        name: "evidential",
        directive: "Ask what would have to be true for this claim to hold, and whether that \
            is actually established or merely assumed. Distinguish what is demonstrated from \
            what is asserted without support.",
    },
    Lens {
        name: "scope",
        directive: "Test for overgeneralization. Is the claim true in some cases but asserted \
            as universal — quantifier creep from \"sometimes\" or \"often\" to \"always\"? \
            Judge whether the breadth claimed is the breadth justified.",
    },
];

/// Tool input (data-model.md §2).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct VerifyParams {
    /// The claim to verify, stated neutrally.
    pub claim: String,
    /// Optional supporting context the verifier may consult. This is the only
    /// extra information a verification pass receives.
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
    /// Independent passes to run for this call alone. **Minimum 1.** The
    /// effective maximum is the deployment's configured count, not a fixed
    /// number, so it cannot appear in the schema — a request above it is
    /// rejected with an error naming the ceiling.
    ///
    /// May be **lower** than the configured count, never higher: each pass is
    /// a whole model call, so raising it would buy work the operator did not
    /// authorise. Omit to use the configured count. Confidence is derived from
    /// cross-pass agreement, so fewer passes means a narrower basis; the
    /// result reports the count it actually used.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub passes: Option<u8>,
}

/// Verdict status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)] // keep the enum inline — no $ref/$defs; mode schemas are flat
pub enum VerdictKind {
    /// The claim holds under its strongest reading.
    Supported,
    /// The claim contains at least one named concrete error.
    Refuted,
}

/// What each of the k passes is grammar-constrained to produce
/// (data-model.md §3) — deliberately nothing for the sanitizer to strip.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PassVerdict {
    /// supported | refuted.
    pub verdict: VerdictKind,
    /// Specific findings; non-empty when refuting (validator-enforced).
    pub findings: Vec<String>,
}

/// The aggregated tool output (data-model.md §4).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Verdict {
    /// Majority verdict across passes; ties resolve to refuted.
    pub verdict: VerdictKind,
    /// Deduplicated findings from the majority-side passes.
    pub findings: Vec<String>,
    /// Agreement ratio (majority count / passes completed). Computed by the
    /// server — never model self-report.
    pub confidence: f64,
    /// Number of verification passes that completed.
    pub passes: u32,
}

/// One completed pass reduced to what aggregation needs: verdict, findings, and
/// (input, output) token usage. Shared by `verify` and `grounded_verify` via
/// [`aggregate_core`].
pub(crate) type PassTuple = (VerdictKind, Vec<String>, u64, u64);

/// One Verify run: the aggregated verdict plus summed token usage for the
/// invocation record.
#[derive(Debug)]
pub struct VerifyRun {
    /// The aggregated verdict.
    pub verdict: Verdict,
    /// Input tokens summed across completed passes.
    pub input_tokens: u64,
    /// Output tokens summed across completed passes.
    pub output_tokens: u64,
}

/// Register the verify mode (boot-time; enforces flat+closed).
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry, ensemble_k: u8) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(PassVerdict))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        VERIFY_ID,
        VERIFY_DESCRIPTION,
        PROMPT_TEMPLATE,
        schema,
        ensemble_k,
    )
}

/// Build the per-pass prompt. The lens (a fixed critical perspective), the
/// claim, and the context are the only dynamic content; the lens does not carry
/// any caller stance.
fn build_prompt(template: &str, lens: &str, claim: &str, context: Option<&str>) -> String {
    template
        .replace("<<lens>>", lens)
        .replace("<<claim>>", claim)
        .replace("<<context>>", context.unwrap_or(""))
}

/// Validate input before any model call (FR-007 `invalid_input`; edge cases
/// 1–2: empty/whitespace rejected, oversize rejected — never silently trimmed).
fn check_input(params: &VerifyParams, max_claim_chars: usize) -> Result<(), AppError> {
    if params.claim.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "claim is empty or whitespace-only".to_string(),
        ));
    }
    // Context counts. It is substituted into the prompt unconditionally, so a
    // per-field bound would pass two 40 000-character fields and send 80 000 —
    // and before this it did not bound `context` at all. Combined, matching
    // `elicit`, `unstick` and `decide`, which have always summed every field
    // that reaches the model.
    let total = params.claim.chars().count()
        + params
            .context
            .as_deref()
            .unwrap_or_default()
            .chars()
            .count();
    if total > max_claim_chars {
        return Err(AppError::InvalidInput(format!(
            "combined input is {total} characters; the configured maximum is {max_claim_chars} \
             (INPUT_MAX_CHARS); it was not trimmed"
        )));
    }
    Ok(())
}

/// Run one Verify invocation: k parallel blind passes, then aggregate.
///
/// # Errors
///
/// `InvalidInput` before any model call; otherwise the dominant failure class
/// when fewer than ⌈k/2⌉ passes complete — a verdict is never synthesized from
/// a minority.
pub async fn run(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    params: &VerifyParams,
    max_claim_chars: usize,
) -> Result<VerifyRun, AppError> {
    check_input(params, max_claim_chars)?;
    // 028 FR-012/FR-012a: the caller may narrow the ensemble for this claim,
    // never widen it. `mode.ensemble_k` stays the configured default.
    let k = super::resolve_passes(params.passes, mode.ensemble_k)?;

    // Each pass scrutinizes under a distinct lens (research D1/D2): pass i uses
    // LENSES[i % LENSES.len()], so genuinely contestable claims scatter and the
    // agreement-ratio confidence spans its range. Aggregation is unchanged.
    let passes = futures::future::join_all((0..k).map(|i| {
        let lens = LENSES[usize::from(i) % LENSES.len()];
        let prompt = build_prompt(
            mode.prompt_template,
            &format!("{}: {}", lens.name, lens.directive),
            &params.claim,
            params.context.as_deref(),
        );
        async move { one_pass(client, mode, &prompt).await }
    }))
    .await;
    let core = passes
        .into_iter()
        .map(|pass| pass.map(|(v, inp, out)| (v.verdict, v.findings, inp, out)))
        .collect();
    aggregate_core(core, k)
}

/// One blind pass: constrained completion → local validation → typed verdict.
async fn one_pass(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    prompt: &str,
) -> Result<(PassVerdict, u64, u64), AppError> {
    let completion = client.complete(prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let pass: PassVerdict = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("verdict shape: {e}")))?;
    // The calibrated profile's hard rule, beyond what a flat schema can say:
    // a refutation with no named error is not evidence.
    if pass.verdict == VerdictKind::Refuted && pass.findings.iter().all(|f| f.trim().is_empty()) {
        return Err(AppError::ValidationFailure(
            "refutation without a named concrete error".to_string(),
        ));
    }
    Ok((pass, completion.input_tokens, completion.output_tokens))
}

/// Aggregate per data-model.md §4: majority verdict (tie → refuted, noted),
/// findings deduplicated from the majority side, confidence = agreement ratio
/// over completed passes, quorum ⌈k/2⌉.
pub(crate) fn aggregate_core(
    passes: Vec<Result<PassTuple, AppError>>,
    k: u8,
) -> Result<VerifyRun, AppError> {
    let mut completed: Vec<(VerdictKind, Vec<String>)> = Vec::new();
    let mut failures: Vec<AppError> = Vec::new();
    let (mut input_tokens, mut output_tokens) = (0_u64, 0_u64);

    for pass in passes {
        match pass {
            Ok((verdict, findings, inp, out)) => {
                completed.push((verdict, findings));
                input_tokens += inp;
                output_tokens += out;
            }
            Err(e) => failures.push(e),
        }
    }

    let quorum = usize::from(k).div_ceil(2);
    if completed.len() < quorum {
        // The completed passes were billed even though they cannot produce a
        // verdict — their tokens ride out on the error (020).
        return Err(dominant_failure_metered(
            failures,
            (input_tokens, output_tokens),
        ));
    }

    let refuted = completed
        .iter()
        .filter(|p| p.0 == VerdictKind::Refuted)
        .count();
    let supported = completed.len() - refuted;
    let tie = refuted == supported;
    // Ties fail toward scrutiny (data-model §4).
    let verdict = if refuted >= supported {
        VerdictKind::Refuted
    } else {
        VerdictKind::Supported
    };

    let mut findings: Vec<String> = Vec::new();
    for pass in completed.iter().filter(|p| p.0 == verdict) {
        for finding in &pass.1 {
            if !finding.trim().is_empty() && !findings.contains(finding) {
                findings.push(finding.clone());
            }
        }
    }
    if tie {
        findings.push(format!(
            "Note: passes split {refuted}-{supported} between refuted and supported; \
             resolving toward scrutiny (refuted)."
        ));
    }

    let majority = refuted.max(supported);
    #[allow(clippy::cast_precision_loss)] // k ≤ 255: exact in f64
    let confidence = majority as f64 / completed.len() as f64;
    #[allow(clippy::cast_possible_truncation)] // bounded by k: u8
    let passes_completed = completed.len() as u32;

    Ok(VerifyRun {
        verdict: Verdict {
            verdict,
            findings,
            confidence,
            passes: passes_completed,
        },
        input_tokens,
        output_tokens,
    })
}

/// Pick the most frequent failure class from `failures` (helper for quorum
/// failure; separated for testability). Reused by `diverge` for its
/// zero-completion case (012, no shared aggregation otherwise).
///
/// `completed` is the token total from passes that succeeded but could not
/// reach quorum. Those passes were billed, as were the sibling failures that
/// lost the class vote, and only one error survives to the record — so every
/// token from the round is attached to it (020). Without this the class choice
/// would silently determine how much spend gets reported.
pub(crate) fn dominant_failure_metered(failures: Vec<AppError>, completed: (u64, u64)) -> AppError {
    let (mut input_tokens, mut output_tokens) = completed;
    for failure in &failures {
        let (inp, out) = failure.billed();
        input_tokens = input_tokens.saturating_add(inp);
        output_tokens = output_tokens.saturating_add(out);
    }
    // The chosen error's own tokens are already in the sum above; strip them so
    // they are not counted a second time when it is re-metered.
    dominant_failure_unmetered(failures).metered(input_tokens, output_tokens)
}

/// Pick the most frequent failure class carrying **no** usage.
///
/// For callers that already meter the failed calls themselves.
/// [`dominant_failure_metered`] is the right choice when the failure set is the
/// only record of what was spent — `verify`'s ensemble and `diverge`'s, where
/// the passes have no meter outside their own errors. It is the wrong choice
/// when the caller has one: `AppError::metered` **adds**, so a caller that
/// meters each failure and then re-attaches its cumulative total bills every
/// failed call twice. `research`'s pipeline meters each dropped source and
/// claim as it goes (020) and closes with `error.metered(meter…)`, so it takes
/// this.
pub(crate) fn dominant_failure_unmetered(failures: Vec<AppError>) -> AppError {
    match dominant_failure(failures) {
        AppError::Metered { source, .. } => *source,
        other => other,
    }
}

/// Pick the most frequent failure class, ignoring usage. Prefer
/// [`dominant_failure_metered`] at aggregation points.
pub(crate) fn dominant_failure(failures: Vec<AppError>) -> AppError {
    use std::collections::HashMap;
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for failure in &failures {
        *counts.entry(failure.outcome().as_str()).or_insert(0) += 1;
    }
    let dominant_class = counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(class, _)| class);
    let mut chosen = None;
    for failure in failures {
        if Some(failure.outcome().as_str()) == dominant_class {
            chosen = Some(failure);
            break;
        }
    }
    chosen.unwrap_or_else(|| {
        AppError::ValidationFailure("no passes completed and no failure recorded".to_string())
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_mode(k: u8) -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        register(&mut registry, k).unwrap();
        registry.get(VERIFY_ID).unwrap().clone()
    }

    fn params(claim: &str) -> VerifyParams {
        VerifyParams {
            claim: claim.to_string(),
            context: None,
            effort: None,
            passes: None,
        }
    }

    /// Mock client that serves canned per-pass results round-robin.
    fn scripted_client(results: Vec<Result<Value, AppError>>) -> MockModelClient {
        let cursor = Arc::new(AtomicUsize::new(0));
        let mut mock = MockModelClient::new();
        mock.expect_complete().returning(move |_, _| {
            let i = cursor.fetch_add(1, Ordering::SeqCst);
            match &results[i % results.len()] {
                Ok(value) => Ok(Completion {
                    value: value.clone(),
                    input_tokens: 100,
                    output_tokens: 10,
                }),
                Err(AppError::Refusal(m)) => Err(AppError::Refusal(m.clone())),
                Err(AppError::Timeout { what, ms }) => Err(AppError::Timeout { what, ms: *ms }),
                Err(other) => Err(AppError::Client(other.to_string())),
            }
        });
        mock
    }

    fn refuted(finding: &str) -> Value {
        json!({ "verdict": "refuted", "findings": [finding] })
    }

    fn supported() -> Value {
        json!({ "verdict": "supported", "findings": [] })
    }

    /// 028 T027 / FR-012, FR-013: a lowered count actually runs that many
    /// passes and the result reports the count it ran.
    ///
    /// `resolve_passes` is unit-tested next door; what this adds is that the
    /// resolved value reaches both the fan-out *and* the aggregation. Those
    /// are two separate uses of the same number, and a change that updated one
    /// and not the other would produce a confidence computed over a different
    /// denominator than the passes that ran — the exact failure the reported
    /// count exists to make visible.
    #[tokio::test]
    async fn a_lowered_pass_count_runs_and_is_reported() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let mut mock = MockModelClient::new();
        mock.expect_complete().returning(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(Completion {
                value: supported(),
                input_tokens: 100,
                output_tokens: 10,
            })
        });

        let mode = test_mode(3);
        let narrowed = VerifyParams {
            passes: Some(1),
            ..params("a claim")
        };
        let outcome = run(&mock, &mode, &narrowed, 50_000).await.unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "one pass requested, so exactly one model call"
        );
        assert_eq!(
            outcome.verdict.passes, 1,
            "the result must report what it ran"
        );

        // Omitting the field still runs the configured count.
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let mut mock = MockModelClient::new();
        mock.expect_complete().returning(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(Completion {
                value: supported(),
                input_tokens: 100,
                output_tokens: 10,
            })
        });
        let outcome = run(&mock, &mode, &params("a claim"), 50_000).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(outcome.verdict.passes, 3);

        // Above the ceiling: rejected before any model call is made.
        let mut mock = MockModelClient::new();
        mock.expect_complete().never();
        let over = VerifyParams {
            passes: Some(9),
            ..params("a claim")
        };
        assert!(run(&mock, &mode, &over, 50_000).await.is_err());
    }

    // ---- T015: schema/contract sync ----------------------------------------

    #[test]
    fn derived_schemas_match_the_contract_file() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../specs/001-core-layer/contracts/verify.tool.json"
        ))
        .unwrap();

        // Input: same property set and required list.
        let input = serde_json::to_value(schemars::schema_for!(VerifyParams)).unwrap();
        let contract_props: Vec<&str> = contract["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let derived_props: Vec<&str> = input["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(contract_props, derived_props);
        // 029: names alone are not the contract. Two defects shipped past a
        // name-only comparison — `effort` as an untyped string, and `passes`
        // publishing `minimum: 0` while the server rejects zero.
        crate::schema::assert_constraints_agree(&contract["inputSchema"], &input, "verify");
        assert_eq!(contract["inputSchema"]["required"], json!(["claim"]));

        // Output (aggregated Verdict): property set, required, verdict enum.
        let output = serde_json::to_value(schemars::schema_for!(Verdict)).unwrap();
        let contract_out: Vec<&str> = contract["outputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let derived_out: Vec<&str> = output["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(contract_out, derived_out);

        // Description stays in sync with the routing text.
        assert_eq!(contract["description"], VERIFY_DESCRIPTION);
    }

    #[test]
    fn pass_schema_registers_flat_and_closed() {
        let mode = test_mode(3);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
        assert_eq!(
            mode.sanitized_schema["properties"]["verdict"]["enum"],
            json!(["supported", "refuted"])
        );
    }

    // ---- T019 / T003: stance-blindness is structural -----------------------

    #[test]
    fn prompt_contains_lens_claim_and_context_verbatim_and_nothing_else() {
        let lens = "literal: read the claim at face value.";
        let claim = "The Battle of Hastings was fought in 1067.";
        let context = "From a history quiz.";
        let prompt = build_prompt(PROMPT_TEMPLATE, lens, claim, Some(context));

        // Exactly the template with the three substitutions — byte-for-byte.
        let expected = PROMPT_TEMPLATE
            .replace("<<lens>>", lens)
            .replace("<<claim>>", claim)
            .replace("<<context>>", context);
        assert_eq!(prompt, expected);

        // The only placeholders are the (fixed) lens and the two subject inputs
        // — no slot for stance/history/identity to flow through. The lens is a
        // critical perspective drawn from LENSES, never caller-supplied prose.
        assert_eq!(PROMPT_TEMPLATE.matches("<<").count(), 3);
        assert!(
            PROMPT_TEMPLATE.contains("<<lens>>")
                && PROMPT_TEMPLATE.contains("<<claim>>")
                && PROMPT_TEMPLATE.contains("<<context>>")
        );
    }

    // ---- T003: the k passes apply distinct lenses --------------------------

    #[test]
    fn each_pass_gets_a_pairwise_distinct_lens() {
        // Build the prompts the way run() does, one per lens in the set.
        let claim = "Some contestable claim.";
        let prompts: Vec<String> = (0..LENSES.len())
            .map(|i| {
                let lens = LENSES[i % LENSES.len()];
                build_prompt(
                    PROMPT_TEMPLATE,
                    &format!("{}: {}", lens.name, lens.directive),
                    claim,
                    None,
                )
            })
            .collect();

        // Every pair of pass prompts differs (the lens is what varies).
        for a in 0..prompts.len() {
            for b in (a + 1)..prompts.len() {
                assert_ne!(
                    prompts[a], prompts[b],
                    "lenses {a} and {b} produced identical prompts"
                );
            }
        }
    }

    #[test]
    fn lens_set_is_nonempty_with_unique_names() {
        assert!(!LENSES.is_empty());
        let mut names: Vec<&str> = LENSES.iter().map(|l| l.name).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "lens names must be unique");
    }

    #[test]
    fn lens_cycles_when_k_exceeds_the_lens_count() {
        // Assignment is LENSES[i % len]: pass `len` reuses lens 0 (research D2).
        let i = LENSES.len(); // first index past the set
        assert_eq!(LENSES[i % LENSES.len()].name, LENSES[0].name);
    }

    // ---- T022 half: input validation before any model call -----------------

    #[tokio::test]
    async fn empty_claim_is_rejected_before_any_model_call() {
        let mode = test_mode(3);
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);

        let err = run(&mock, &mode, &params("   \n"), 50_000)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
    }

    #[tokio::test]
    async fn oversized_claim_is_rejected_not_trimmed() {
        let mode = test_mode(3);
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);

        let big = "x".repeat(51);
        let err = run(&mock, &mode, &params(&big), 50).await.unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
        assert!(err.to_string().contains("51") && err.to_string().contains("50"));
    }

    /// `context` counts toward the bound, and a claim that fits cannot smuggle
    /// an unbounded one past it.
    ///
    /// `context` reaches `build_prompt` unconditionally, so leaving it out of
    /// the check meant `INPUT_MAX_CHARS` bounded the claim while the prompt
    /// itself was unbounded — a caller could send a 200 MB context behind a
    /// one-word claim. Every sibling corrective that takes an optional field
    /// (`elicit`, `unstick`, `decide`) already summed it; `verify` and
    /// `diverge` were the two that did not.
    #[tokio::test]
    async fn oversized_context_is_rejected_not_trimmed() {
        let mode = test_mode(3);
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);

        let p = VerifyParams {
            context: Some("x".repeat(51)),
            ..params("a short claim")
        };
        let err = run(&mock, &mode, &p, 50).await.unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
        // The reported total is claim + context, not either alone.
        assert!(
            err.to_string().contains("64") && err.to_string().contains("50"),
            "{err}"
        );
    }

    // ---- T014: aggregation --------------------------------------------------

    #[tokio::test]
    async fn majority_refuted_with_agreement_confidence_and_summed_usage() {
        let mode = test_mode(3);
        let mock = scripted_client(vec![
            Ok(refuted("1066, not 1067")),
            Ok(refuted("1066, not 1067")),
            Ok(supported()),
        ]);

        let run_result = run(&mock, &mode, &params("c"), 50_000).await.unwrap();
        assert_eq!(run_result.verdict.verdict, VerdictKind::Refuted);
        assert_eq!(
            run_result.verdict.findings,
            vec!["1066, not 1067".to_string()]
        );
        assert!((run_result.verdict.confidence - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(run_result.verdict.passes, 3);
        assert_eq!(run_result.input_tokens, 300);
        assert_eq!(run_result.output_tokens, 30);
    }

    #[tokio::test]
    async fn unanimous_support_is_full_confidence() {
        let mode = test_mode(3);
        let mock = scripted_client(vec![Ok(supported())]);

        let out = run(&mock, &mode, &params("c"), 50_000)
            .await
            .unwrap()
            .verdict;
        assert_eq!(out.verdict, VerdictKind::Supported);
        assert!(out.findings.is_empty());
        assert!((out.confidence - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn tie_resolves_to_refuted_and_says_so() {
        let mode = test_mode(2);
        let mock = scripted_client(vec![Ok(refuted("the error")), Ok(supported())]);

        let out = run(&mock, &mode, &params("c"), 50_000)
            .await
            .unwrap()
            .verdict;
        assert_eq!(out.verdict, VerdictKind::Refuted);
        assert!(out.findings.iter().any(|f| f.contains("the error")));
        assert!(out.findings.iter().any(|f| f.contains("scrutiny")));
        assert!((out.confidence - 0.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn findings_are_deduplicated_from_the_majority_side_only() {
        let mode = test_mode(3);
        let mock = scripted_client(vec![
            Ok(json!({ "verdict": "refuted", "findings": ["A", "B"] })),
            Ok(json!({ "verdict": "refuted", "findings": ["B", "C"] })),
            Ok(json!({ "verdict": "supported", "findings": ["minority view"] })),
        ]);

        let out = run(&mock, &mode, &params("c"), 50_000)
            .await
            .unwrap()
            .verdict;
        assert_eq!(out.findings, vec!["A", "B", "C"]);
    }

    #[tokio::test]
    async fn below_quorum_returns_the_dominant_failure_never_a_minority_verdict() {
        // k=3, quorum=2: two refusals + one completed pass → Refusal, no verdict.
        let mode = test_mode(3);
        let mock = scripted_client(vec![
            Err(AppError::Refusal("declined".into())),
            Err(AppError::Refusal("declined".into())),
            Ok(refuted("real finding")),
        ]);

        let err = run(&mock, &mode, &params("c"), 50_000).await.unwrap_err();
        assert!(matches!(err.root(), AppError::Refusal(_)), "got: {err}");
        // 020: the one pass that completed was billed 100/10 even though it
        // could not reach quorum. Before this, choosing the dominant failure
        // class silently discarded that spend.
        assert_eq!(err.billed(), (100, 10));
    }

    #[tokio::test]
    async fn quorum_holds_with_reduced_passes_reported() {
        // k=3, quorum=2: one refusal + two completed refuted → verdict with passes=2.
        let mode = test_mode(3);
        let mock = scripted_client(vec![
            Err(AppError::Refusal("declined".into())),
            Ok(refuted("err")),
            Ok(refuted("err")),
        ]);

        let out = run(&mock, &mode, &params("c"), 50_000)
            .await
            .unwrap()
            .verdict;
        assert_eq!(out.passes, 2);
        assert_eq!(out.verdict, VerdictKind::Refuted);
        assert!((out.confidence - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn a_refutation_without_findings_is_a_failed_pass_not_evidence() {
        let mode = test_mode(1);
        let mock = scripted_client(vec![Ok(json!({ "verdict": "refuted", "findings": [] }))]);

        let err = run(&mock, &mode, &params("c"), 50_000).await.unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
    }

    #[test]
    fn dominant_failure_picks_the_most_frequent_class() {
        let dominant = dominant_failure(vec![
            AppError::Timeout {
                what: "request",
                ms: 1,
            },
            AppError::Refusal("a".into()),
            AppError::Refusal("b".into()),
        ]);
        assert!(matches!(dominant, AppError::Refusal(_)));
    }
}
