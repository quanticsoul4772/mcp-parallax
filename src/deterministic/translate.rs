//! The classify+translate hop (research.md 005 D3).
//!
//! One constrained call that either declines (decline-biased — FR-004) or
//! produces a small typed formal target for exactly one engine. Cross-field
//! consistency is enforced by a pure validator; its failures are translation
//! violations, fed to the single retry exactly like an engine parse error
//! (D5).

use crate::deterministic::{Polarity, Violation, EXPRESSION_MAX_CHARS, SMTLIB_MAX_CHARS};
use crate::error::AppError;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::Deserialize;

/// Registry id of the translation mode.
pub const TRANSLATE_MODE_ID: &str = "deterministic_translate";

/// The decline-bias sentence — prompt-borne and pinned by test (FR-004).
pub const DECLINE_BIAS: &str = "When uncertain whether a claim is checkable, decline: a crisp \
answer to the wrong question is worse than no answer.";

/// The evalexpr dialect whitelist — prompt-borne and pinned by test
/// (analysis A1: syntax mistakes must not burn the semantic retry).
pub const EVALEXPR_WHITELIST: &str = "operators + - * / % ^ (exponentiation), comparisons \
< <= > >= == !=, boolean && || !, parentheses, and ONLY these functions: math::abs, min, max, \
floor, ceil, round";

const TRANSLATE_PROMPT_TEMPLATE: &str = "\
You classify and translate one claim for deterministic checking. The claim is \
checkable only if its truth is computable by one of two engines, with no \
judgment, world knowledge, or prediction required. <<decline_bias>>\n\
\n\
Engine 'arithmetic': produce a single BOOLEAN-VALUED expression that is true \
exactly when the claim is true. Dialect (use nothing outside it): \
<<whitelist>>. Vague quantities ('roughly', 'about') must become an explicit \
tolerance in the expression (e.g. math::abs(x - y) <= bound); if no \
reasonable bound exists, the claim is not checkable.\n\
\n\
Engine 'constraints': produce an SMT-LIB 2 script — (declare-const ...) for \
each variable over Int, Real, or Bool, then (assert ...) forms using linear \
arithmetic; do NOT emit (check-sat). Set 'asserted' to what the claim says \
about the system: 'satisfiable' if the claim says an assignment exists, \
'unsatisfiable' if the claim says none exists.\n\
\n\
A claim mixing a checkable core with judgment may be checked ONLY for the \
core — the formal form is the statement of what was checked; if the core \
cannot stand alone, decline.<<violation_clause>>\n\
\n\
Claim: <<claim>>\n\
Context: <<context>>";

/// The hop's constrained output (flat + closed — Principle II;
/// data-model.md §3).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TranslateOut {
    /// Whether the claim is honestly checkable (decline-biased).
    pub checkable: bool,
    /// Why not checkable (required when checkable is false).
    pub reason: Option<String>,
    /// The chosen engine when checkable: "arithmetic" or "constraints".
    pub engine: Option<String>,
    /// Boolean-valued evalexpr expression (engine = arithmetic).
    pub arithmetic_expression: Option<String>,
    /// SMT-LIB 2 declares + asserts, no check-sat (engine = constraints).
    pub smtlib_constraints: Option<String>,
    /// The claim's polarity about the constraint system when engine =
    /// constraints: "satisfiable" or "unsatisfiable".
    pub asserted: Option<String>,
}

/// A validated translation, ready for exactly one engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Translation {
    /// Honest decline (a successful outcome — FR-004).
    NotCheckable {
        /// The classifier's stated reason.
        reason: String,
    },
    /// Boolean-valued expression for the arithmetic engine.
    Arithmetic {
        /// The expression.
        expression: String,
    },
    /// SMT-LIB 2 script + polarity for the solver.
    Constraints {
        /// The script (declares + asserts).
        smtlib: String,
        /// What the claim asserts about the system.
        asserted: Polarity,
    },
}

/// One translation attempt's outcome: a usable translation, or a REAL
/// violation to feed the single retry.
pub type TranslateAttempt = Result<Translation, Violation>;

/// Register the translation mode (boot-time; enforces flat+closed).
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(TranslateOut))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        TRANSLATE_MODE_ID,
        "internal: deterministic-check translation",
        TRANSLATE_PROMPT_TEMPLATE,
        schema,
        1,
    )
}

/// Build the attempt prompt.
///
/// `violation` is the prior REAL violation fed back verbatim on the retry
/// (D5).
fn build_prompt(
    mode: &CorrectiveMode,
    claim: &str,
    context: Option<&str>,
    violation: Option<&Violation>,
) -> String {
    let violation_clause = violation.map_or(String::new(), |v| {
        format!(
            "\n\nYOUR PREVIOUS TRANSLATION WAS REJECTED by the engine with this exact \
             violation: {v}. Produce a corrected translation (or decline if the claim \
             cannot be faithfully formalized)."
        )
    });
    mode.prompt_template
        .replace("<<decline_bias>>", DECLINE_BIAS)
        .replace("<<whitelist>>", EVALEXPR_WHITELIST)
        .replace("<<violation_clause>>", &violation_clause)
        .replace("<<claim>>", claim)
        .replace("<<context>>", context.unwrap_or("(none)"))
}

/// One classify+translate call.
///
/// Outer errors are hard failure classes (refusal, timeout, …); the inner
/// [`TranslateAttempt`] distinguishes a usable translation from a retryable
/// violation. Returns token usage too.
///
/// # Errors
///
/// Provider classes from the model call; schema violations on the hop are
/// `ValidationFailure` (the constrained-output contract itself failed).
pub async fn translate_once(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    claim: &str,
    context: Option<&str>,
    violation: Option<&Violation>,
) -> Result<(TranslateAttempt, u64, u64), AppError> {
    let prompt = build_prompt(mode, claim, context, violation);
    let completion = client.complete(&prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let out: TranslateOut = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("translation shape: {e}")))?;
    Ok((
        cross_field_validate(&out),
        completion.input_tokens,
        completion.output_tokens,
    ))
}

/// The pure cross-field validator (D3): enforce the consistency the flat
/// schema cannot express. Failures are translation violations (retryable).
fn cross_field_validate(out: &TranslateOut) -> TranslateAttempt {
    if !out.checkable {
        let reason = out
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .ok_or_else(|| Violation("checkable=false requires a non-empty reason".to_string()))?;
        return Ok(Translation::NotCheckable {
            reason: reason.to_string(),
        });
    }
    match out.engine.as_deref() {
        Some("arithmetic") => {
            let expression = out
                .arithmetic_expression
                .as_deref()
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .ok_or_else(|| {
                    Violation(
                        "engine=arithmetic requires a non-empty arithmetic_expression".to_string(),
                    )
                })?;
            if expression.chars().count() > EXPRESSION_MAX_CHARS {
                return Err(Violation(format!(
                    "arithmetic_expression exceeds {EXPRESSION_MAX_CHARS} characters"
                )));
            }
            Ok(Translation::Arithmetic {
                expression: expression.to_string(),
            })
        }
        Some("constraints") => {
            let smtlib = out
                .smtlib_constraints
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    Violation(
                        "engine=constraints requires non-empty smtlib_constraints".to_string(),
                    )
                })?;
            if smtlib.chars().count() > SMTLIB_MAX_CHARS {
                return Err(Violation(format!(
                    "smtlib_constraints exceeds {SMTLIB_MAX_CHARS} characters"
                )));
            }
            if smtlib.contains('\0') {
                return Err(Violation(
                    "smtlib_constraints contains a NUL byte".to_string(),
                ));
            }
            if !smtlib.contains("(assert") {
                return Err(Violation(
                    "smtlib_constraints contains no (assert ...) form".to_string(),
                ));
            }
            if smtlib.contains("(check-sat") {
                return Err(Violation(
                    "smtlib_constraints must not contain (check-sat) — the engine appends it"
                        .to_string(),
                ));
            }
            let asserted = match out.asserted.as_deref() {
                Some("satisfiable") => Polarity::Satisfiable,
                Some("unsatisfiable") => Polarity::Unsatisfiable,
                other => {
                    return Err(Violation(format!(
                        "engine=constraints requires asserted to be exactly \"satisfiable\"                          or \"unsatisfiable\"; got {other:?}"
                    )))
                }
            };
            Ok(Translation::Constraints {
                smtlib: smtlib.to_string(),
                asserted,
            })
        }
        Some(other) => Err(Violation(format!(
            "engine must be exactly \"arithmetic\" or \"constraints\"; got {other:?}"
        ))),
        None => Err(Violation(
            "checkable=true requires an engine choice".to_string(),
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::json;

    fn mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        register(&mut registry).unwrap();
        registry.get(TRANSLATE_MODE_ID).unwrap().clone()
    }

    fn client_returning(value: serde_json::Value) -> MockModelClient {
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

    // Analysis A1 + FR-004: both prompt-borne guarantees are pinned.
    #[tokio::test]
    async fn the_prompt_carries_the_decline_bias_and_the_whitelist_verbatim() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            assert!(prompt.contains(DECLINE_BIAS), "decline bias missing");
            assert!(prompt.contains(EVALEXPR_WHITELIST), "whitelist missing");
            assert!(prompt.contains("Claim: the claim text"));
            Ok(Completion {
                value: json!({
                    "checkable": false, "reason": "judgment", "engine": null,
                    "arithmetic_expression": null, "smtlib_constraints": null,
                    "asserted": null
                }),
                input_tokens: 1,
                output_tokens: 1,
            })
        });
        let (attempt, _, _) = translate_once(&client, &mode(), "the claim text", None, None)
            .await
            .unwrap();
        assert_eq!(
            attempt.unwrap(),
            Translation::NotCheckable {
                reason: "judgment".into()
            }
        );
    }

    #[tokio::test]
    async fn the_retry_prompt_carries_the_violation_verbatim() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            assert!(
                prompt.contains(
                    "REJECTED by the engine with this exact violation: parse \
                                 error at token 3"
                ),
                "{prompt}"
            );
            Ok(Completion {
                value: json!({
                    "checkable": true, "reason": null, "engine": "arithmetic",
                    "arithmetic_expression": "1 + 1 == 2",
                    "smtlib_constraints": null, "asserted": null
                }),
                input_tokens: 1,
                output_tokens: 1,
            })
        });
        let violation = Violation("parse error at token 3".into());
        let (attempt, _, _) = translate_once(&client, &mode(), "c", None, Some(&violation))
            .await
            .unwrap();
        assert!(matches!(attempt, Ok(Translation::Arithmetic { .. })));
    }

    #[tokio::test]
    async fn clean_constraint_translation_passes_the_cross_field_validator() {
        let client = client_returning(json!({
            "checkable": true, "reason": null, "engine": "constraints",
            "arithmetic_expression": null,
            "smtlib_constraints": "(declare-const x Int)\n(assert (> x 0))",
            "asserted": "satisfiable"
        }));
        let (attempt, input, output) = translate_once(&client, &mode(), "c", Some("ctx"), None)
            .await
            .unwrap();
        assert!(matches!(
            attempt,
            Ok(Translation::Constraints {
                asserted: Polarity::Satisfiable,
                ..
            })
        ));
        assert_eq!((input, output), (10, 5));
    }

    #[tokio::test]
    async fn cross_field_violations_are_retryable_not_hard_errors() {
        for (value, marker) in [
            (
                json!({ "checkable": true, "reason": null, "engine": null,
                        "arithmetic_expression": null, "smtlib_constraints": null,
                        "asserted": null }),
                "requires an engine",
            ),
            (
                json!({ "checkable": true, "reason": null, "engine": "arithmetic",
                        "arithmetic_expression": "  ", "smtlib_constraints": null,
                        "asserted": null }),
                "non-empty arithmetic_expression",
            ),
            (
                json!({ "checkable": true, "reason": null, "engine": "constraints",
                        "arithmetic_expression": null,
                        "smtlib_constraints": "(declare-const x Int)", "asserted": "satisfiable" }),
                "no (assert",
            ),
            (
                json!({ "checkable": true, "reason": null, "engine": "constraints",
                        "arithmetic_expression": null,
                        "smtlib_constraints": "(assert true)(check-sat)", "asserted": "satisfiable" }),
                "must not contain (check-sat",
            ),
            (
                json!({ "checkable": true, "reason": null, "engine": "constraints",
                        "arithmetic_expression": null,
                        "smtlib_constraints": "(assert true)", "asserted": null }),
                "requires asserted",
            ),
            (
                json!({ "checkable": false, "reason": "  ", "engine": null,
                        "arithmetic_expression": null, "smtlib_constraints": null,
                        "asserted": null }),
                "non-empty reason",
            ),
        ] {
            let client = client_returning(value);
            let (attempt, _, _) = translate_once(&client, &mode(), "c", None, None)
                .await
                .unwrap();
            let violation = attempt.unwrap_err();
            assert!(violation.0.contains(marker), "{marker}: {violation}");
        }
    }

    #[test]
    fn oversized_targets_are_violations() {
        let big_expr = cross_field_validate(&TranslateOut {
            checkable: true,
            reason: None,
            engine: Some("arithmetic".to_string()),
            arithmetic_expression: Some("1".repeat(EXPRESSION_MAX_CHARS + 1)),
            smtlib_constraints: None,
            asserted: None,
        });
        assert!(big_expr.unwrap_err().0.contains("exceeds"));

        let big_smt = cross_field_validate(&TranslateOut {
            checkable: true,
            reason: None,
            engine: Some("constraints".to_string()),
            arithmetic_expression: None,
            smtlib_constraints: Some("(assert true)".repeat(SMTLIB_MAX_CHARS / 10)),
            asserted: Some("satisfiable".to_string()),
        });
        assert!(big_smt.unwrap_err().0.contains("exceeds"));
    }
}
