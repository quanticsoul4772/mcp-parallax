//! `check` orchestration: validate → translate (single violation-fed retry)
//! → execute each part → server-assembled result (research.md 005 D4/D5;
//! `bundle.rs` for the part-to-claim assembly).
//!
//! The verdict and its explanation are pure functions of engine output — the
//! model classifies and translates, nothing more (FR-002).

use crate::deterministic::bundle::{self, Checked};
use crate::deterministic::contract::{CheckParams, CheckResult, PartResult};
use crate::deterministic::solver::SolverOutcome;
use crate::deterministic::translate::{self, Part, Translation};
use crate::deterministic::{
    arithmetic, solver, Engine, Polarity, Verdict, Violation, PARTS_CONCURRENCY, SOLVER_TIMEOUT_MS,
    TRANSLATION_ATTEMPTS_MAX,
};
use crate::error::AppError;
use crate::modes::CorrectiveMode;
use crate::traits::client::ModelClient;
use std::sync::Arc;

/// Everything one check needs.
pub struct CheckDeps {
    /// For the translation hop only.
    pub model_client: Arc<dyn ModelClient>,
    /// The registered translation mode.
    pub translate_mode: CorrectiveMode,
    /// Generic input bound (`INPUT_MAX_CHARS`).
    pub input_max_chars: usize,
}

/// Run one check. Returns the result plus (input, output) token usage for
/// the invocation record.
///
/// # Errors
///
/// `InvalidInput` before any model call; provider classes from the
/// translation hop; `Timeout` (solver-naming message) when the solver gives
/// up; `ValidationFailure` (translation-naming message) when both
/// translation attempts fail. A verdict is NEVER synthesized from a failed
/// translation (FR-005).
/// Run against the client the server selected for this call (028).
///
/// Taken as a parameter rather than read from `deps` because the deps struct
/// binds its client at startup, and a per-call effort resolves to a different
/// one. Non-optional: every production path supplies it, and an `Option` whose
/// `None` arm nothing takes is a branch the tests would exercise and
/// production never would.
///
/// # Errors
///
/// Returns [`AppError`] when translation or engine execution fails — the same
/// classes [`run`] returns, metered with whatever the attempts were billed.
pub async fn run_with(
    deps: &CheckDeps,
    params: &CheckParams,
    client: &dyn ModelClient,
) -> Result<(CheckResult, u64, u64), AppError> {
    // 020: as in decide — the violation-fed retry means a failure can follow
    // two translation calls, and those were billed.
    let mut spent = (0_u64, 0_u64);
    let outcome = run_spent(deps, params, client, &mut spent).await;
    outcome.map_err(|error| error.metered(spent.0, spent.1))
}

async fn run_spent(
    deps: &CheckDeps,
    params: &CheckParams,
    client: &dyn ModelClient,
    spent: &mut (u64, u64),
) -> Result<(CheckResult, u64, u64), AppError> {
    // Both fields build one prompt (`translate.rs`: `Claim: <<claim>>\nContext:
    // <<context>>`), so the bound is on their sum. Per-field admitted twice the
    // configured maximum, and the violation-fed retry sends it again.
    //
    // **Bound the artifact at its build point, and count artifacts — not
    // fields.** The typed formal target this check produces is per-field-ish
    // and downstream, which is what makes per-field validation look defensible
    // here; it is the wrong end of the dependency arrow. `save` is the genuine
    // exception, and not because its consumer treats the fields separately:
    // `content` builds an embedding, `origin` builds a stored metadata row, and
    // two independent artifacts are two independent bounds.
    check_text("claim", &params.claim, deps.input_max_chars)?;
    let total = params.claim.chars().count()
        + params
            .context
            .as_deref()
            .unwrap_or_default()
            .chars()
            .count();
    if total > deps.input_max_chars {
        return Err(AppError::InvalidInput(format!(
            "combined input is {total} characters; the configured maximum is {} \
             (INPUT_MAX_CHARS); it was not trimmed",
            deps.input_max_chars
        )));
    }

    let (mut input_tokens, mut output_tokens) = (0_u64, 0_u64);
    let mut violation: Option<Violation> = None;

    for attempt in 1..=TRANSLATION_ATTEMPTS_MAX {
        let (outcome, inp, out) = translate::translate_once(
            client,
            &deps.translate_mode,
            &params.claim,
            params.context.as_deref(),
            violation.as_ref(),
        )
        .await?;
        input_tokens += inp;
        output_tokens += out;
        *spent = (input_tokens, output_tokens);

        // Violations are reported one stage at a time. A reply the validator
        // rejects goes straight back with every part's validation violation;
        // no engine runs on it.
        let parts = match outcome {
            Ok(parts) => parts,
            Err(v) => {
                violation = Some(v);
                continue;
            }
        };

        // On a reply the validator accepted, every part runs and every engine
        // violation is collected, in part order. One rejected part sends the
        // whole claim back: a part is never dropped to rescue the others. A
        // hard failure (a solver that gave up) fails the call.
        let count = parts.len();
        let texts: Vec<String> = parts.iter().map(|part| part.text.clone()).collect();
        let mut checked = Vec::with_capacity(count);
        let mut violations = Vec::new();
        for (index, executed) in execute_all(parts, execute).await.into_iter().enumerate() {
            match executed? {
                Ok(done) => checked.push(done),
                Err(v) => {
                    violations.push(translate::part_violation(index, count, &texts[index], &v));
                }
            }
        }
        if violations.is_empty() {
            return Ok((
                bundle::assemble(checked, attempt),
                input_tokens,
                output_tokens,
            ));
        }
        violation = Some(Violation(violations.join("; ")));
    }

    // Both attempts failed on REAL violations — no verdict is synthesized.
    Err(AppError::ValidationFailure(format!(
        "translation failed after the retry; last engine violation: {}",
        violation.map_or_else(|| "(none recorded)".to_string(), |v| v.0)
    )))
}

/// Run every part through `run`, at most [`PARTS_CONCURRENCY`] at a time,
/// returning the outcomes in part order.
///
/// Bounded both ways. One after another, twelve slow solver parts could
/// outlast the request timeout. All at once, they would share the processor
/// while each is held to a clock-time limit, so a part that is fine alone
/// could time out because of the others — and one timeout fails the call.
async fn execute_all<F, Fut, T>(parts: Vec<Part>, run: F) -> Vec<T>
where
    F: FnMut(Part) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    use futures::StreamExt;
    futures::stream::iter(parts)
        .map(run)
        .buffered(PARTS_CONCURRENCY)
        .collect()
        .await
}

/// Execute one part on its engine.
///
/// The inner `Err` is a REAL engine violation (retryable); the outer one is a
/// hard failure — a solver that gave up fails the call, since a verdict can
/// only exist when the engine proved something.
async fn execute(part: Part) -> Result<Result<Checked, Violation>, AppError> {
    let claim = part.text;
    match part.translation {
        Translation::NotCheckable { reason } => Ok(Ok(Checked {
            part: PartResult {
                claim,
                verdict: Verdict::NotCheckable,
                engine: None,
                formal_form: None,
                engine_result: None,
                witness: None,
                reason: Some(reason),
            },
            explanation: "The claim was not formalized: its truth is not computable by the \
                          available engines. See reason; route judgment claims to verify."
                .to_string(),
        })),
        Translation::Arithmetic { expression } => {
            // Exact arithmetic is blocking CPU work, like Z3 below.
            let form = expression.clone();
            let evaluated = tokio::task::spawn_blocking(move || arithmetic::evaluate(&form))
                .await
                .map_err(|e| AppError::Client(format!("arithmetic task failed: {e}")))?;
            Ok(evaluated.map(|outcome| {
                let verdict = if outcome.holds {
                    Verdict::Supported
                } else {
                    Verdict::Refuted
                };
                Checked {
                    explanation: explain_arithmetic(
                        &expression,
                        verdict,
                        outcome.exact_root_equality,
                    ),
                    part: PartResult {
                        claim,
                        verdict,
                        engine: Some(Engine::Arithmetic),
                        formal_form: Some(expression),
                        engine_result: Some(outcome.result_text),
                        witness: None,
                        reason: None,
                    },
                }
            }))
        }
        Translation::Constraints { smtlib, asserted } => {
            // Z3 is blocking CPU work — keep it off the async workers.
            let script = smtlib.clone();
            let solved = tokio::task::spawn_blocking(move || solver::check(&script))
                .await
                .map_err(|e| AppError::Client(format!("solver task failed: {e}")))?;
            let outcome = match solved {
                Ok(outcome) => outcome,
                Err(v) => return Ok(Err(v)),
            };
            // Narrow to PROVEN outcomes only — Unknown cannot reach the
            // verdict mapping by construction.
            let proven = match outcome {
                SolverOutcome::Sat { witness } => Proven::Sat { witness },
                SolverOutcome::Unsat => Proven::Unsat,
                SolverOutcome::Unknown => {
                    return Err(AppError::Timeout {
                        what: "solver (timeout or incompleteness)",
                        ms: u64::from(SOLVER_TIMEOUT_MS),
                    });
                }
            };
            let engine_result = match proven {
                Proven::Sat { .. } => "sat",
                Proven::Unsat => "unsat",
            };
            let (verdict, witness) = constraint_verdict(proven, asserted);
            Ok(Ok(Checked {
                explanation: explain_constraints(asserted, verdict, witness.is_some()),
                part: PartResult {
                    claim,
                    verdict,
                    engine: Some(Engine::Constraints),
                    formal_form: Some(smtlib),
                    engine_result: Some(engine_result.to_string()),
                    witness,
                    reason: None,
                },
            }))
        }
    }
}

/// A PROVEN solver outcome — `Unknown` is unrepresentable here, so a verdict
/// can only exist when the engine actually proved something (FR-002/FR-005;
/// review finding: the invariant is in the types, not statement order).
enum Proven {
    Sat { witness: String },
    Unsat,
}

/// Pure verdict mapping for the solver (data-model.md §4): the proven result
/// crossed with the claim's asserted polarity; the witness rides on
/// whichever side holds a model.
fn constraint_verdict(proven: Proven, asserted: Polarity) -> (Verdict, Option<String>) {
    match (proven, asserted) {
        (Proven::Sat { witness }, Polarity::Satisfiable) => (Verdict::Supported, Some(witness)),
        (Proven::Sat { witness }, Polarity::Unsatisfiable) => (Verdict::Refuted, Some(witness)),
        (Proven::Unsat, Polarity::Unsatisfiable) => (Verdict::Supported, None),
        (Proven::Unsat, Polarity::Satisfiable) => (Verdict::Refuted, None),
    }
}

/// Deterministic explanation templates (D4) — never model-phrased.
///
/// `exact_root_equality` marks a refuted bare square root compared with `==`:
/// a plain rounded claim ("the square root of 2 is 1.414") is checked exactly
/// and refuted, so the caller is pointed at the executed form. The pointer
/// gives no advice: telling a caller how to restate a claim would coach it
/// past the refutation.
fn explain_arithmetic(expression: &str, verdict: Verdict, exact_root_equality: bool) -> String {
    if verdict == Verdict::Supported {
        return format!(
            "The expression `{expression}` evaluated to true: the claim holds as formalized."
        );
    }
    let pointer = if exact_root_equality {
        " An exact equality was checked; compare formal_form with the claim to see what was \
         executed."
    } else {
        ""
    };
    format!(
        "The expression `{expression}` evaluated to false: the claim does not hold as \
         formalized.{pointer}"
    )
}

fn explain_constraints(asserted: Polarity, verdict: Verdict, has_witness: bool) -> String {
    let proved = if has_witness {
        "satisfiable (a concrete assignment exists — see witness)"
    } else {
        "unsatisfiable (proven: no assignment exists)"
    };
    let claimed = match asserted {
        Polarity::Satisfiable => "satisfiable",
        Polarity::Unsatisfiable => "unsatisfiable",
    };
    let stands = match verdict {
        Verdict::Supported => "the claim is supported",
        _ => "the claim is refuted",
    };
    format!(
        "The solver proved the constraint system is {proved}; the claim asserted it was \
         {claimed}, so {stands}."
    )
}

/// Shared input validation (FR-008): non-empty after trim, bounded.
fn check_text(field: &str, text: &str, max_chars: usize) -> Result<(), AppError> {
    if text.trim().is_empty() {
        return Err(AppError::InvalidInput(format!(
            "{field} is empty or whitespace-only"
        )));
    }
    let len = text.chars().count();
    if len > max_chars {
        return Err(AppError::InvalidInput(format!(
            "{field} is {len} characters; the configured maximum is {max_chars} \
             (INPUT_MAX_CHARS); it was not trimmed"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::deterministic::translate::{register, TRANSLATE_MODE_ID};
    use crate::modes::ModeRegistry;
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::{json, Value};

    fn deps_with(client: MockModelClient) -> CheckDeps {
        let mut registry = ModeRegistry::new();
        register(&mut registry).unwrap();
        CheckDeps {
            model_client: Arc::new(client),
            translate_mode: registry.get(TRANSLATE_MODE_ID).unwrap().clone(),
            input_max_chars: 50_000,
        }
    }

    fn client_translating(value: Value) -> MockModelClient {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(move |_, _| {
            Ok(Completion {
                value: value.clone(),
                input_tokens: 10,
                output_tokens: 5,
                reported_cost_micros: None,
            })
        });
        client
    }

    /// One part of a stand-in translator reply: (text, checkable, reason,
    /// engine, form, polarity — sent as `stated`).
    type PartSpec<'a> = (&'a str, bool, &'a str, &'a str, &'a str, &'a str);

    /// A stand-in translator reply in the index-aligned wire shape.
    fn translation_of(parts: &[PartSpec<'_>]) -> Value {
        json!({
            "parts": parts.iter().map(|p| p.0).collect::<Vec<_>>(),
            "checkable": parts.iter().map(|p| p.1).collect::<Vec<_>>(),
            "reasons": parts.iter().map(|p| p.2).collect::<Vec<_>>(),
            "engines": parts.iter().map(|p| p.3).collect::<Vec<_>>(),
            "forms": parts.iter().map(|p| p.4).collect::<Vec<_>>(),
            "stated": parts.iter().map(|p| p.5).collect::<Vec<_>>(),
        })
    }

    fn arithmetic_translation(expression: &str) -> Value {
        translation_of(&[("the claim", true, "", "arithmetic", expression, "")])
    }

    fn constraints_translation(smtlib: &str, asserted: &str) -> Value {
        translation_of(&[("the claim", true, "", "constraints", smtlib, asserted)])
    }

    fn params(claim: &str) -> CheckParams {
        CheckParams {
            claim: claim.to_string(),
            context: None,
            effort: None,
        }
    }

    // ---- T006: ground truth through the REAL engines -----------------------

    #[tokio::test]
    async fn true_arithmetic_claims_are_supported_with_the_form_shown() {
        let client = client_translating(arithmetic_translation(
            "math::abs(1840 * 0.63 - 1159.2) <= 0.001",
        ));
        let (result, inp, out) = {
            let d = &deps_with(client);
            run_with(d, &params("63% of 1840 is 1159.2"), d.model_client.as_ref()).await
        }
        .unwrap();
        assert_eq!(result.verdict, Verdict::Supported);
        assert_eq!(result.engine, Some(Engine::Arithmetic));
        assert!(result.formal_form.as_deref().unwrap().contains("math::abs"));
        assert_eq!(result.engine_result.as_deref(), Some("true"));
        assert!(result.explanation.contains("evaluated to true"));
        assert_eq!(result.translation_attempts, 1);
        assert_eq!((inp, out), (10, 5));
    }

    #[tokio::test]
    async fn false_arithmetic_claims_are_refuted() {
        let client = client_translating(arithmetic_translation("2^31 - 1 >= 2147483648"));
        let (result, _, _) = {
            let d = &deps_with(client);
            run_with(
                d,
                &params("i32::MAX is at least 2^31"),
                d.model_client.as_ref(),
            )
            .await
        }
        .unwrap();
        assert_eq!(result.verdict, Verdict::Refuted);
        assert!(result.explanation.contains("evaluated to false"));
    }

    /// A plain rounded square-root claim is refuted as an exact equality, and
    /// the explanation points the caller at the executed form. It makes no
    /// statement about rounding and gives no advice to restate the claim.
    #[tokio::test]
    async fn a_refuted_bare_root_equality_points_at_the_executed_form() {
        let explanation_of = |expression: &'static str| async move {
            let client = client_translating(arithmetic_translation(expression));
            let d = &deps_with(client);
            let (result, _, _) = run_with(d, &params("the claim"), d.model_client.as_ref())
                .await
                .unwrap();
            assert_eq!(result.verdict, Verdict::Refuted, "{expression}");
            assert!(!result.every_listed_part_held, "{expression}");
            result.explanation
        };
        assert_eq!(
            explanation_of("math::sqrt(2) == 1.414").await,
            "The expression `math::sqrt(2) == 1.414` evaluated to false: the claim does not \
             hold as formalized. An exact equality was checked; compare formal_form with the \
             claim to see what was executed."
        );
        // Not a bare square-root equality: the plain explanation.
        assert!(explanation_of("7 / 2 == 3.4")
            .await
            .ends_with("does not hold as formalized."));
    }

    /// A claim checked in parts gets the bundle summary, not each part's own
    /// explanation, so the pointer does not appear there. The changelog
    /// states this limit; this pins it.
    #[tokio::test]
    async fn a_bundle_of_root_equalities_carries_no_pointer() {
        let result = check_bundle(&[
            (
                "the square root of 2 is 1.414",
                true,
                "",
                "arithmetic",
                "math::sqrt(2) == 1.414",
                "",
            ),
            (
                "the square root of 3 is 1.732",
                true,
                "",
                "arithmetic",
                "math::sqrt(3) == 1.732",
                "",
            ),
        ])
        .await;
        assert_eq!(result.verdict, Verdict::Refuted);
        assert!(result.parts.iter().all(|p| p.verdict == Verdict::Refuted));
        assert!(
            !result.explanation.contains("exact equality was checked"),
            "{}",
            result.explanation
        );
    }

    #[tokio::test]
    async fn refuted_impossibility_claims_carry_the_solver_witness() {
        // Claim: "x>2 and x<10 cannot both hold" — asserted unsatisfiable,
        // but the system is satisfiable: refuted WITH a counterexample.
        let client = client_translating(constraints_translation(
            "(declare-const x Int)\n(assert (> x 2))\n(assert (< x 10))",
            "unsatisfiable",
        ));
        let (result, _, _) = {
            let d = &deps_with(client);
            run_with(
                d,
                &params("no x is both >2 and <10"),
                d.model_client.as_ref(),
            )
            .await
        }
        .unwrap();
        assert_eq!(result.verdict, Verdict::Refuted);
        assert_eq!(result.engine, Some(Engine::Constraints));
        assert_eq!(result.engine_result.as_deref(), Some("sat"));
        let witness = result.witness.expect("counterexample witness");
        assert!(witness.contains('x'), "{witness}");
        assert!(result.explanation.contains("refuted"));
    }

    #[tokio::test]
    async fn proven_unsat_supports_an_impossibility_claim() {
        let client = client_translating(constraints_translation(
            "(declare-const a Bool)\n(assert a)\n(assert (not a))",
            "unsatisfiable",
        ));
        let (result, _, _) = {
            let d = &deps_with(client);
            run_with(
                d,
                &params("a and not-a cannot both hold"),
                d.model_client.as_ref(),
            )
            .await
        }
        .unwrap();
        assert_eq!(result.verdict, Verdict::Supported);
        assert_eq!(result.engine_result.as_deref(), Some("unsat"));
        assert!(result.witness.is_none());
    }

    #[tokio::test]
    async fn satisfiable_claims_are_supported_with_a_witness() {
        let client = client_translating(constraints_translation(
            "(declare-const x Int)\n(declare-const y Int)\n(assert (= (+ x y) 11))\n(assert (> x 2))",
            "satisfiable",
        ));
        let (result, _, _) = {
            let d = &deps_with(client);
            run_with(
                d,
                &params("x+y=11 with x>2 is solvable"),
                d.model_client.as_ref(),
            )
            .await
        }
        .unwrap();
        assert_eq!(result.verdict, Verdict::Supported);
        assert!(result.witness.is_some());
    }

    // ---- US2 (T009): the honest decline path -------------------------------

    #[tokio::test]
    async fn uncheckable_claims_return_not_checkable_with_reason_and_null_engine_fields() {
        let client = client_translating(translation_of(&[(
            "the claim",
            false,
            "elegance is a judgment, not a computation",
            "",
            "",
            "",
        )]));
        let (result, _, _) = {
            let d = &deps_with(client);
            run_with(
                d,
                &params("Rust is more elegant than C++"),
                d.model_client.as_ref(),
            )
            .await
        }
        .unwrap();
        assert_eq!(result.verdict, Verdict::NotCheckable);
        assert!(result.reason.unwrap().contains("judgment"));
        assert!(result.engine.is_none());
        assert!(result.formal_form.is_none());
        assert!(result.engine_result.is_none());
        assert!(result.witness.is_none());
        assert_eq!(result.translation_attempts, 1);
    }

    // ---- US3 (T010): the symbolic feedback loop -----------------------------

    #[tokio::test]
    async fn an_engine_violation_triggers_exactly_one_violation_fed_retry() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut client = MockModelClient::new();
        client
            .expect_complete()
            .times(2)
            .returning(move |prompt, _| {
                let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    // Wrong dialect: abs() is not in the whitelist → engine rejects.
                    Ok(Completion {
                        value: arithmetic_translation("abs(1840 * 0.63 - 1159) <= 0.5"),
                        input_tokens: 10,
                        output_tokens: 5,
                        reported_cost_micros: None,
                    })
                } else {
                    assert!(
                        prompt.contains("rejected by the engine"),
                        "retry must carry the violation: {prompt}"
                    );
                    Ok(Completion {
                        value: arithmetic_translation("math::abs(1840 * 0.63 - 1159) <= 0.5"),
                        input_tokens: 10,
                        output_tokens: 5,
                        reported_cost_micros: None,
                    })
                }
            });
        let (result, inp, _) = {
            let d = &deps_with(client);
            run_with(
                d,
                &params("63% of 1840 is about 1159"),
                d.model_client.as_ref(),
            )
            .await
        }
        .unwrap();
        assert_eq!(result.verdict, Verdict::Supported);
        assert_eq!(result.translation_attempts, 2);
        assert_eq!(inp, 20); // both attempts metered
    }

    #[tokio::test]
    async fn double_translation_failure_is_validation_failure_with_no_verdict() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(2).returning(|_, _| {
            Ok(Completion {
                value: arithmetic_translation("this is not an expression ((("),
                input_tokens: 10,
                output_tokens: 5,
                reported_cost_micros: None,
            })
        });
        let err = {
            let d = &deps_with(client);
            run_with(d, &params("c"), d.model_client.as_ref()).await
        }
        .unwrap_err();
        assert!(
            matches!(err.root(), AppError::ValidationFailure(_)),
            "{err}"
        );
        // 020: both translation attempts were billed; the failure carries them.
        assert!(err.billed().0 > 0, "spend lost on the failure path");
        assert!(err
            .to_string()
            .contains("translation failed after the retry"));
    }

    // ---- Per-claim results: a bundle is checked part by part ----------------

    async fn check_bundle(parts: &[PartSpec<'_>]) -> CheckResult {
        let client = client_translating(translation_of(parts));
        let d = &deps_with(client);
        run_with(d, &params("a bundle of claims"), d.model_client.as_ref())
            .await
            .unwrap()
            .0
    }

    #[tokio::test]
    async fn a_single_claim_keeps_its_top_level_fields_and_gains_one_part() {
        let result =
            check_bundle(&[("2 + 2 is 4", true, "", "arithmetic", "2 + 2 == 4", "")]).await;
        assert_eq!(result.verdict, Verdict::Supported);
        assert_eq!(result.formal_form.as_deref(), Some("2 + 2 == 4"));
        assert_eq!(result.engine_result.as_deref(), Some("true"));
        assert!(result.explanation.contains("evaluated to true"));
        assert_eq!(result.parts.len(), 1);
        let part = &result.parts[0];
        assert_eq!(part.claim, "2 + 2 is 4");
        assert_eq!(part.verdict, Verdict::Supported);
        assert_eq!(part.engine, Some(Engine::Arithmetic));
        assert_eq!(part.formal_form.as_deref(), Some("2 + 2 == 4"));
        assert_eq!(part.engine_result.as_deref(), Some("true"));
        assert!(part.witness.is_none() && part.reason.is_none());
    }

    #[tokio::test]
    async fn a_bundle_whose_parts_all_hold_is_not_supported_as_a_whole() {
        let result = check_bundle(&[
            (
                "3 x 2.5 is 7.5",
                true,
                "",
                "arithmetic",
                "3 * 2.5 == 7.5",
                "",
            ),
            ("7.5 is at most 30", true, "", "arithmetic", "7.5 <= 30", ""),
        ])
        .await;
        assert_eq!(result.verdict, Verdict::NotCheckable);
        // The one value a verdict-only caller can read instead, and the
        // explanation that points to it.
        assert!(result.every_listed_part_held);
        assert!(
            result
                .explanation
                .contains("every_listed_part_held is true"),
            "{}",
            result.explanation
        );
        assert_eq!(result.engine, Some(Engine::Arithmetic));
        assert_eq!(
            result.formal_form.as_deref(),
            Some("[1] 3 * 2.5 == 7.5\n[2] 7.5 <= 30")
        );
        assert_eq!(result.engine_result.as_deref(), Some("[1] true\n[2] true"));
        assert!(result.witness.is_none());
        // Every part held, yet the claim as a whole is not called supported:
        // the reason is the coverage sentence alone.
        let reason = result.reason.as_deref().unwrap();
        assert!(
            reason.starts_with("the parts were checked separately") && !reason.contains('\n'),
            "{reason}"
        );
        assert!(
            result.explanation.contains("2 parts"),
            "{}",
            result.explanation
        );
        assert_eq!(
            result.parts.iter().map(|p| p.verdict).collect::<Vec<_>>(),
            [Verdict::Supported, Verdict::Supported]
        );
    }

    #[tokio::test]
    async fn one_false_part_refutes_the_bundle_even_beside_an_unchecked_part() {
        let result = check_bundle(&[
            ("2 + 2 is 4", true, "", "arithmetic", "2 + 2 == 4", ""),
            ("2 + 2 is 5", true, "", "arithmetic", "2 + 2 == 5", ""),
            ("the result is elegant", false, "a judgment", "", "", ""),
        ])
        .await;
        assert_eq!(result.verdict, Verdict::Refuted);
        assert_eq!(
            result.parts.iter().map(|p| p.verdict).collect::<Vec<_>>(),
            [Verdict::Supported, Verdict::Refuted, Verdict::NotCheckable]
        );
        // Not every part ran on the same engine (one ran on none).
        assert!(result.engine.is_none());
        assert_eq!(
            result.engine_result.as_deref(),
            Some("[1] true\n[2] false\n[3] (not checked)")
        );
        assert!(result.reason.is_none());
    }

    /// The owner's rule: parts that hold beside a part that cannot be checked
    /// leave the bundle `not_checkable` — never `supported`.
    #[tokio::test]
    async fn held_parts_beside_an_unchecked_part_leave_the_bundle_not_checkable() {
        let result = check_bundle(&[
            ("2 + 2 is 4", true, "", "arithmetic", "2 + 2 == 4", ""),
            (
                "the derivative of f is g",
                false,
                "needs symbolic calculus",
                "",
                "",
                "",
            ),
        ])
        .await;
        assert_eq!(result.verdict, Verdict::NotCheckable);
        assert_eq!(result.parts[0].verdict, Verdict::Supported);
        assert_eq!(result.parts[1].verdict, Verdict::NotCheckable);
        assert_eq!(
            result.parts[1].reason.as_deref(),
            Some("needs symbolic calculus")
        );
        assert_eq!(
            result.formal_form.as_deref(),
            Some("[1] 2 + 2 == 4\n[2] (not checked)")
        );
        let reason = result.reason.as_deref().unwrap();
        assert!(
            reason.starts_with("the parts were checked separately")
                && reason.ends_with("\n[2] needs symbolic calculus"),
            "{reason}"
        );
        assert!(
            result.explanation.contains("1 supported")
                && result.explanation.contains("1 not checked"),
            "{}",
            result.explanation
        );
    }

    #[tokio::test]
    async fn a_bundle_may_mix_the_two_engines() {
        let result = check_bundle(&[
            ("2 + 2 is 4", true, "", "arithmetic", "2 + 2 == 4", ""),
            (
                "some x exceeds 2",
                true,
                "",
                "constraints",
                "(declare-const x Int)\n(assert (> x 2))",
                "satisfiable",
            ),
        ])
        .await;
        assert_eq!(result.verdict, Verdict::NotCheckable);
        assert!(result.engine.is_none(), "two engines, so no single one");
        assert_eq!(result.parts[1].engine, Some(Engine::Constraints));
        // The script sits on one line at the top level; the part keeps it exact.
        assert_eq!(
            result.formal_form.as_deref(),
            Some("[1] 2 + 2 == 4\n[2] (declare-const x Int) (assert (> x 2))")
        );
        assert_eq!(
            result.parts[1].formal_form.as_deref(),
            Some("(declare-const x Int)\n(assert (> x 2))")
        );
        assert_eq!(result.engine_result.as_deref(), Some("[1] true\n[2] sat"));
        let witness = result.witness.expect("the satisfying assignment");
        assert!(
            witness.starts_with("[2] ") && !witness.contains('\n'),
            "{witness}"
        );
    }

    #[tokio::test]
    async fn a_bundle_of_only_unchecked_parts_has_no_top_level_form() {
        let result = check_bundle(&[
            ("it is elegant", false, "a judgment", "", "", ""),
            ("it will rain", false, "a prediction", "", "", ""),
        ])
        .await;
        assert_eq!(result.verdict, Verdict::NotCheckable);
        assert!(result.formal_form.is_none() && result.engine_result.is_none());
        assert!(
            result
                .reason
                .as_deref()
                .unwrap()
                .ends_with("\n[1] a judgment\n[2] a prediction"),
            "{:?}",
            result.reason
        );
    }

    /// One part the engine rejects sends the WHOLE bundle back for the single
    /// retry; a part that still fails afterwards fails the call. No part is
    /// downgraded to `not_checkable` to rescue the others.
    #[tokio::test]
    async fn a_part_the_engine_rejects_twice_fails_the_whole_call() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(2).returning(|_, _| {
            Ok(Completion {
                value: translation_of(&[
                    ("2 + 2 is 4", true, "", "arithmetic", "2 + 2 == 4", ""),
                    (
                        "nonsense",
                        true,
                        "",
                        "arithmetic",
                        "((( not an expression",
                        "",
                    ),
                ]),
                input_tokens: 10,
                output_tokens: 5,
                reported_cost_micros: None,
            })
        });
        let err = {
            let d = &deps_with(client);
            run_with(d, &params("a bundle"), d.model_client.as_ref()).await
        }
        .unwrap_err();
        assert!(
            matches!(err.root(), AppError::ValidationFailure(_)),
            "{err}"
        );
        assert!(err.to_string().contains("part 2"), "{err}");
    }

    /// A translator that answers `first`, then `second`; the retry prompt is
    /// handed to `on_retry` for inspection.
    fn client_answering_twice(
        first: Value,
        second: Value,
        on_retry: impl Fn(&str) + Send + Sync + 'static,
    ) -> MockModelClient {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut client = MockModelClient::new();
        client
            .expect_complete()
            .times(2)
            .returning(move |prompt, _| {
                let value = if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    first.clone()
                } else {
                    on_retry(prompt);
                    second.clone()
                };
                Ok(Completion {
                    value,
                    input_tokens: 10,
                    output_tokens: 5,
                    reported_cost_micros: None,
                })
            });
        client
    }

    const TRUE_SUM: PartSpec<'static> = ("2 + 2 is 4", true, "", "arithmetic", "2 + 2 == 4", "");
    /// `y` is never declared, so the solver rejects the script — and its
    /// violation does not echo the script back.
    const UNDECLARED: PartSpec<'static> = (
        "some y exceeds 2",
        true,
        "",
        "constraints",
        "(assert (> y 2))",
        "satisfiable",
    );
    const DECLARED: PartSpec<'static> = (
        "some y exceeds 2",
        true,
        "",
        "constraints",
        "(declare-const y Int)\n(assert (> y 2))",
        "satisfiable",
    );

    /// A solver violation does not show the script, so the retry must be told
    /// which statement "part 2" was — not left to re-derive its own split.
    #[tokio::test]
    async fn a_bundle_retry_quotes_the_rejected_part() {
        let client = client_answering_twice(
            translation_of(&[TRUE_SUM, UNDECLARED]),
            translation_of(&[TRUE_SUM, DECLARED]),
            |prompt| {
                assert!(
                    prompt.contains("part 2 (\"some y exceeds 2\"): "),
                    "retry does not quote the part: {prompt}"
                );
                assert!(!prompt.contains("part 1"), "part 1 was fine: {prompt}");
            },
        );
        let d = &deps_with(client);
        run_with(d, &params("a bundle"), d.model_client.as_ref())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_bundle_retry_that_succeeds_returns_every_part_and_meters_both_calls() {
        let client = client_answering_twice(
            translation_of(&[TRUE_SUM, UNDECLARED]),
            translation_of(&[TRUE_SUM, DECLARED]),
            |_| {},
        );
        let d = &deps_with(client);
        let (result, inp, out) = run_with(d, &params("a bundle"), d.model_client.as_ref())
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::NotCheckable);
        assert_eq!(result.translation_attempts, 2);
        assert_eq!(result.parts.len(), 2);
        assert_eq!(result.engine_result.as_deref(), Some("[1] true\n[2] sat"));
        assert_eq!((inp, out), (20, 10));
    }

    /// Violations are reported one stage at a time: a reply the validator
    /// rejects goes straight back, and no engine runs on any of its parts. So
    /// the retry is told about the malformed part 1 and NOT about part 2,
    /// whose script only the solver would have rejected.
    ///
    /// Running the valid parts anyway was tried and removed: their results
    /// are discarded on a rejected reply, so a solver that gave up there had
    /// to be either ignored (a failure set aside) or fatal (a call failed by a
    /// part that was only run to gather complaints).
    #[tokio::test]
    async fn a_reply_the_validator_rejects_goes_back_before_any_engine_runs() {
        let malformed: PartSpec<'static> = ("2 + 2 is 4", true, "", "arithmetic", "", "");
        let client = client_answering_twice(
            translation_of(&[malformed, UNDECLARED]),
            translation_of(&[TRUE_SUM, DECLARED]),
            |prompt| {
                assert!(
                    prompt.contains("part 1 (\"2 + 2 is 4\"): engine=arithmetic requires"),
                    "validation violation missing: {prompt}"
                );
                assert!(
                    !prompt.contains("part 2"),
                    "an engine ran on a rejected reply: {prompt}"
                );
            },
        );
        let d = &deps_with(client);
        let (result, _, _) = run_with(d, &params("a bundle"), d.model_client.as_ref())
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::NotCheckable);
        assert_eq!(result.translation_attempts, 2);
    }

    /// The retry is a fresh translation: nothing of the first split survives,
    /// so a different number of parts is taken on its own terms.
    #[tokio::test]
    async fn a_retry_may_return_a_different_number_of_parts() {
        let client = client_answering_twice(
            translation_of(&[TRUE_SUM, UNDECLARED]),
            translation_of(&[("2 + 2 is 5", true, "", "arithmetic", "2 + 2 == 5", "")]),
            |_| {},
        );
        let d = &deps_with(client);
        let (result, _, _) = run_with(d, &params("a bundle"), d.model_client.as_ref())
            .await
            .unwrap();
        assert_eq!(result.parts.len(), 1);
        assert_eq!(result.verdict, Verdict::Refuted);
        assert_eq!(result.formal_form.as_deref(), Some("2 + 2 == 5"));
        assert_eq!(result.translation_attempts, 2);
    }

    /// Parts run at the same time, but no more than `PARTS_CONCURRENCY` of
    /// them: the solver's timeout is clock time, so every extra simultaneous
    /// run makes the others likelier to exceed it, while one at a time would
    /// outlast the request timeout.
    ///
    /// The most parts ever in flight must equal the cap exactly — fewer means
    /// they ran one after another, more means the cap leaked. Earlier parts
    /// are the slowest, so parts finish out of order and the outcomes must
    /// still come back in part order.
    #[tokio::test]
    async fn at_most_the_cap_of_parts_execute_at_once_and_outcomes_stay_in_order() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let count = PARTS_CONCURRENCY * 2 + 1;
        let texts: Vec<String> = (0..count).map(|n| format!("part {n}")).collect();
        let parts: Vec<Part> = texts
            .iter()
            .map(|text| Part {
                text: text.clone(),
                translation: Translation::NotCheckable {
                    reason: "unused".to_string(),
                },
            })
            .collect();
        let in_flight = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let position = AtomicUsize::new(0);

        let outcomes = execute_all(parts, |part| {
            let in_flight = Arc::clone(&in_flight);
            let most = Arc::clone(&most);
            let nth = position.fetch_add(1, Ordering::SeqCst);
            async move {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                // Earlier parts sleep longest, so later ones finish first.
                let millis = 10 * u64::try_from(count - nth).unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                part.text
            }
        })
        .await;

        assert_eq!(most.load(Ordering::SeqCst), PARTS_CONCURRENCY);
        assert_eq!(outcomes, texts);
    }

    /// Two solver runs in one bundle, in flight together: each keeps its own
    /// script, result and witness.
    #[tokio::test]
    async fn two_constraint_parts_in_one_bundle_are_each_settled_by_the_solver() {
        let result = check_bundle(&[
            DECLARED,
            (
                "no x is below itself",
                true,
                "",
                "constraints",
                "(declare-const x Int)\n(assert (< x x))",
                "unsatisfiable",
            ),
        ])
        .await;
        assert_eq!(result.verdict, Verdict::NotCheckable);
        assert_eq!(result.engine, Some(Engine::Constraints));
        assert_eq!(result.engine_result.as_deref(), Some("[1] sat\n[2] unsat"));
        assert!(result.parts[0].witness.as_deref().unwrap().contains('y'));
        assert!(result.parts[1].witness.is_none());
        // Only part 1 has a witness, so only part 1 is listed.
        let witness = result.witness.unwrap();
        assert!(
            witness.starts_with("[1] ") && !witness.contains("[2]"),
            "{witness}"
        );
    }

    // ---- FR-008: input validation before any model call ---------------------

    /// The bound is on `claim + context`, not on each alone.
    ///
    /// Both build one prompt (`translate.rs`: `Claim: <<claim>>\nContext:
    /// <<context>>`), so validating them separately admitted two fields of
    /// `INPUT_MAX_CHARS` each and sent twice the configured maximum — again on
    /// every violation-fed retry. `verify` and `diverge` had the same shape and
    /// were fixed alongside this.
    ///
    /// `save` keeps its per-field checks and is right to — not because its
    /// consumer separates the fields, but because they build two artifacts:
    /// `content` becomes an embedding, `origin` becomes a stored metadata row.
    /// Two artifacts, two bounds.
    #[tokio::test]
    async fn claim_and_context_are_bounded_together_not_separately() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(0);
        let deps = deps_with(client);

        // Each field is under the 50 000 cap; together they exceed it.
        let p = CheckParams {
            context: Some("x".repeat(30_000)),
            ..params(&"y".repeat(30_000))
        };
        let err = run_with(&deps, &p, deps.model_client.as_ref())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
        assert!(
            err.to_string().contains("60000") && err.to_string().contains("50000"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn invalid_inputs_are_rejected_before_any_model_call() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(0);
        let deps = deps_with(client);

        let err = run_with(&deps, &params("   "), deps.model_client.as_ref())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");

        let err = run_with(
            &deps,
            &params(&"x".repeat(50_001)),
            deps.model_client.as_ref(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("INPUT_MAX_CHARS"), "{err}");
    }
}
