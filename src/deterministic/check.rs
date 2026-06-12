//! `check` orchestration: validate → translate (single violation-fed retry)
//! → execute → server-assembled result (research.md 005 D4/D5).
//!
//! The verdict and its explanation are pure functions of engine output — the
//! model classifies and translates, nothing more (FR-002).

use crate::deterministic::contract::{CheckParams, CheckResult};
use crate::deterministic::solver::SolverOutcome;
use crate::deterministic::translate::{self, Translation};
use crate::deterministic::{
    arithmetic, solver, Engine, Polarity, Verdict, Violation, SOLVER_TIMEOUT_MS,
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
#[allow(clippy::too_many_lines)] // the translate-execute-assemble spine reads best unbroken
pub async fn run(
    deps: &CheckDeps,
    params: &CheckParams,
) -> Result<(CheckResult, u64, u64), AppError> {
    check_text("claim", &params.claim, deps.input_max_chars)?;
    if let Some(context) = &params.context {
        check_text("context", context, deps.input_max_chars)?;
    }

    let (mut input_tokens, mut output_tokens) = (0_u64, 0_u64);
    let mut violation: Option<Violation> = None;

    for attempt in 1..=TRANSLATION_ATTEMPTS_MAX {
        let (outcome, inp, out) = translate::translate_once(
            deps.model_client.as_ref(),
            &deps.translate_mode,
            &params.claim,
            params.context.as_deref(),
            violation.as_ref(),
        )
        .await?;
        input_tokens += inp;
        output_tokens += out;

        let translation = match outcome {
            Ok(translation) => translation,
            Err(v) => {
                violation = Some(v);
                continue;
            }
        };

        match translation {
            Translation::NotCheckable { reason } => {
                return Ok((
                    CheckResult {
                        verdict: Verdict::NotCheckable,
                        engine: None,
                        formal_form: None,
                        engine_result: None,
                        witness: None,
                        explanation: "The claim was not formalized: its truth is not \
                                      computable by the available engines. See reason; \
                                      route judgment claims to verify."
                            .to_string(),
                        reason: Some(reason),
                        translation_attempts: attempt,
                    },
                    input_tokens,
                    output_tokens,
                ));
            }
            Translation::Arithmetic { expression } => match arithmetic::evaluate(&expression) {
                Ok(outcome) => {
                    let verdict = if outcome.holds {
                        Verdict::Supported
                    } else {
                        Verdict::Refuted
                    };
                    return Ok((
                        CheckResult {
                            explanation: explain_arithmetic(&expression, verdict),
                            verdict,
                            engine: Some(Engine::Arithmetic),
                            formal_form: Some(expression),
                            engine_result: Some(outcome.result_text),
                            witness: None,
                            reason: None,
                            translation_attempts: attempt,
                        },
                        input_tokens,
                        output_tokens,
                    ));
                }
                Err(v) => {
                    violation = Some(v);
                }
            },
            Translation::Constraints { smtlib, asserted } => {
                // Z3 is blocking CPU work — keep it off the async workers.
                let script = smtlib.clone();
                let solved = tokio::task::spawn_blocking(move || solver::check(&script))
                    .await
                    .map_err(|e| AppError::Client(format!("solver task failed: {e}")))?;
                match solved {
                    Ok(SolverOutcome::Unknown) => {
                        return Err(AppError::Timeout {
                            what: "solver (timeout or incompleteness)",
                            ms: u64::from(SOLVER_TIMEOUT_MS),
                        });
                    }
                    Ok(outcome) => {
                        let (verdict, witness) = constraint_verdict(&outcome, asserted);
                        return Ok((
                            CheckResult {
                                explanation: explain_constraints(asserted, &outcome, verdict),
                                verdict,
                                engine: Some(Engine::Constraints),
                                formal_form: Some(smtlib),
                                engine_result: Some(
                                    match outcome {
                                        SolverOutcome::Sat { .. } => "sat",
                                        SolverOutcome::Unsat => "unsat",
                                        SolverOutcome::Unknown => "unknown",
                                    }
                                    .to_string(),
                                ),
                                witness,
                                reason: None,
                                translation_attempts: attempt,
                            },
                            input_tokens,
                            output_tokens,
                        ));
                    }
                    Err(v) => {
                        violation = Some(v);
                    }
                }
            }
        }
    }

    // Both attempts failed on REAL violations — no verdict is synthesized.
    Err(AppError::ValidationFailure(format!(
        "translation failed after the retry; last engine violation: {}",
        violation.map_or_else(|| "(none recorded)".to_string(), |v| v.0)
    )))
}

/// Pure verdict mapping for the solver (data-model.md §4): the engine result
/// crossed with the claim's asserted polarity; the witness rides on
/// whichever side holds a model.
fn constraint_verdict(outcome: &SolverOutcome, asserted: Polarity) -> (Verdict, Option<String>) {
    match (outcome, asserted) {
        (SolverOutcome::Sat { witness }, Polarity::Satisfiable) => {
            (Verdict::Supported, Some(witness.clone()))
        }
        (SolverOutcome::Sat { witness }, Polarity::Unsatisfiable) => {
            (Verdict::Refuted, Some(witness.clone()))
        }
        (SolverOutcome::Unsat, Polarity::Unsatisfiable) => (Verdict::Supported, None),
        // Unknown never reaches here — the orchestrator surfaces it as
        // Timeout; mapped like a refutation defensively.
        (SolverOutcome::Unsat, Polarity::Satisfiable) | (SolverOutcome::Unknown, _) => {
            (Verdict::Refuted, None)
        }
    }
}

/// Deterministic explanation templates (D4) — never model-phrased.
fn explain_arithmetic(expression: &str, verdict: Verdict) -> String {
    match verdict {
        Verdict::Supported => format!(
            "The expression `{expression}` evaluated to true: the claim holds as formalized."
        ),
        _ => format!(
            "The expression `{expression}` evaluated to false: the claim does not hold as \
             formalized."
        ),
    }
}

fn explain_constraints(asserted: Polarity, outcome: &SolverOutcome, verdict: Verdict) -> String {
    let proved = match outcome {
        SolverOutcome::Sat { .. } => "satisfiable (a concrete assignment exists — see witness)",
        SolverOutcome::Unsat => "unsatisfiable (proven: no assignment exists)",
        SolverOutcome::Unknown => "undetermined",
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
            })
        });
        client
    }

    fn arithmetic_translation(expression: &str) -> Value {
        json!({
            "checkable": true, "reason": null, "engine": "arithmetic",
            "arithmetic_expression": expression,
            "smtlib_constraints": null, "asserted": null
        })
    }

    fn constraints_translation(smtlib: &str, asserted: &str) -> Value {
        json!({
            "checkable": true, "reason": null, "engine": "constraints",
            "arithmetic_expression": null,
            "smtlib_constraints": smtlib, "asserted": asserted
        })
    }

    fn params(claim: &str) -> CheckParams {
        CheckParams {
            claim: claim.to_string(),
            context: None,
        }
    }

    // ---- T006: ground truth through the REAL engines -----------------------

    #[tokio::test]
    async fn true_arithmetic_claims_are_supported_with_the_form_shown() {
        let client = client_translating(arithmetic_translation("1840 * 0.63 == 1159.2"));
        let (result, inp, out) = run(&deps_with(client), &params("63% of 1840 is 1159.2"))
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::Supported);
        assert_eq!(result.engine, Some(Engine::Arithmetic));
        assert_eq!(result.formal_form.as_deref(), Some("1840 * 0.63 == 1159.2"));
        assert_eq!(result.engine_result.as_deref(), Some("true"));
        assert!(result.explanation.contains("evaluated to true"));
        assert_eq!(result.translation_attempts, 1);
        assert_eq!((inp, out), (10, 5));
    }

    #[tokio::test]
    async fn false_arithmetic_claims_are_refuted() {
        let client = client_translating(arithmetic_translation("2^31 - 1 >= 2147483648"));
        let (result, _, _) = run(&deps_with(client), &params("i32::MAX is at least 2^31"))
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::Refuted);
        assert!(result.explanation.contains("evaluated to false"));
    }

    #[tokio::test]
    async fn refuted_impossibility_claims_carry_the_solver_witness() {
        // Claim: "x>2 and x<10 cannot both hold" — asserted unsatisfiable,
        // but the system is satisfiable: refuted WITH a counterexample.
        let client = client_translating(constraints_translation(
            "(declare-const x Int)\n(assert (> x 2))\n(assert (< x 10))",
            "unsatisfiable",
        ));
        let (result, _, _) = run(&deps_with(client), &params("no x is both >2 and <10"))
            .await
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
        let (result, _, _) = run(&deps_with(client), &params("a and not-a cannot both hold"))
            .await
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
        let (result, _, _) = run(&deps_with(client), &params("x+y=11 with x>2 is solvable"))
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::Supported);
        assert!(result.witness.is_some());
    }

    // ---- US2 (T009): the honest decline path -------------------------------

    #[tokio::test]
    async fn uncheckable_claims_return_not_checkable_with_reason_and_null_engine_fields() {
        let client = client_translating(json!({
            "checkable": false, "reason": "elegance is a judgment, not a computation",
            "engine": null, "arithmetic_expression": null,
            "smtlib_constraints": null, "asserted": null
        }));
        let (result, _, _) = run(&deps_with(client), &params("Rust is more elegant than C++"))
            .await
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
                    })
                }
            });
        let (result, inp, _) = run(&deps_with(client), &params("63% of 1840 is about 1159"))
            .await
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
            })
        });
        let err = run(&deps_with(client), &params("c")).await.unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err
            .to_string()
            .contains("translation failed after the retry"));
    }

    // ---- FR-008: input validation before any model call ---------------------

    #[tokio::test]
    async fn invalid_inputs_are_rejected_before_any_model_call() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(0);
        let deps = deps_with(client);

        let err = run(&deps, &params("   ")).await.unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");

        let err = run(&deps, &params(&"x".repeat(50_001))).await.unwrap_err();
        assert!(err.to_string().contains("INPUT_MAX_CHARS"), "{err}");
    }
}
