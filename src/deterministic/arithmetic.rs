//! The arithmetic engine: evalexpr evaluation of a boolean-valued expression
//! (research.md 005 D2). Pure, deterministic, in-process — tested directly
//! against ground truth, never mocked.

use crate::deterministic::{Violation, EXPRESSION_MAX_CHARS};

/// A completed arithmetic evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArithmeticOutcome {
    /// The boolean the expression evaluated to — the verdict carrier.
    pub holds: bool,
    /// The raw engine result text (for `engine_result` — FR-007).
    pub result_text: String,
}

/// Evaluate a boolean-valued expression.
///
/// # Errors
///
/// Returns a [`Violation`] (retryable — research.md D5) for parse/eval
/// errors, a non-boolean result, or an oversized expression. The known f64
/// bound is named in research.md D2.
pub fn evaluate(expression: &str) -> Result<ArithmeticOutcome, Violation> {
    let len = expression.chars().count();
    if len > EXPRESSION_MAX_CHARS {
        return Err(Violation(format!(
            "expression is {len} characters; the maximum is {EXPRESSION_MAX_CHARS}"
        )));
    }
    // The equality guard (acceptance run 1 finding): exact == / != over
    // float-producing arithmetic yields confidently wrong verdicts (f64
    // rounding; evalexpr's int != float strictness). Force the tolerance
    // form via the feedback loop. Pure-integer equality stays exact.
    let has_equality = expression.contains("==") || expression.contains("!=");
    let produces_floats =
        expression.contains('.') || expression.contains('/') || expression.contains('^');
    if has_equality && produces_floats {
        return Err(Violation(format!(
            "the expression `{expression}` uses exact == or != over float-producing \
             arithmetic (a decimal point, /, or ^), which is not allowed - rewrite the \
             comparison as math::abs(a - b) <= 0.0001 (an explicit numeric tolerance), \
             or rewrite division-free (a/b == c becomes a == b*c)"
        )));
    }
    match evalexpr::eval(expression) {
        Ok(evalexpr::Value::Boolean(holds)) => Ok(ArithmeticOutcome {
            holds,
            result_text: holds.to_string(),
        }),
        Ok(other) => Err(Violation(format!(
            "expression evaluated to the non-boolean value {other:?}; the formal target \
             must be a boolean-valued claim encoding (e.g. a comparison)"
        ))),
        Err(e) => Err(Violation(format!("expression rejected by the engine: {e}"))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // T004 ground-truth table: the engine, not opinion, decides.
    #[test]
    fn ground_truth_comparisons() {
        for (expression, expected) in [
            ("math::abs(1840 * 0.63 - 1159.2) <= 0.001", true),
            ("math::abs(1840 * 0.63 - 1159) <= 0.5", true), // explicit tolerance
            ("math::abs(1840 * 0.63 - 1200) <= 0.5", false),
            ("math::abs(2^32 - 4294967296) <= 0.5", true), // ^ yields a float
            ("2^31 - 1.0 >= 2147483648.0", false),         // i32::MAX is smaller
            ("(3 < 5) && (5 < 7)", true),
            ("min(3, 9) == 3 && max(3, 9) == 9", true), // pure-int equality stays exact
            ("7 * 24 * 60 == 10000", false),            // pure-int equality stays exact
            ("floor(7.9) <= 7.5 && ceil(7.1) >= 7.5", true),
        ] {
            let outcome = evaluate(expression).unwrap();
            assert_eq!(outcome.holds, expected, "{expression}");
            assert_eq!(outcome.result_text, expected.to_string());
        }
    }

    // A "roughly X" claim translated with its bound made explicit (C1).
    #[test]
    fn explicit_tolerance_forms_carry_the_bound_visibly() {
        // "roughly 1159" with a ±1% bound made explicit in the form itself.
        let outcome = evaluate("math::abs(1840 * 0.63 - 1159) <= 1159 * 0.01").unwrap();
        assert!(outcome.holds);
    }

    // Acceptance run 1: exact float equality produced confidently wrong
    // refutations (0.15 * 240 == 36 is false in f64) — now a guarded
    // Violation that the feedback loop turns into a tolerance form.
    #[test]
    fn float_equality_is_a_violation_not_a_wrong_verdict() {
        for expression in ["0.15 * 240 == 36", "2^32 == 4294967296", "7 / 2 != 3.5"] {
            let violation = evaluate(expression).unwrap_err();
            assert!(
                violation.0.contains("tolerance"),
                "{expression}: {violation}"
            );
        }
    }

    #[test]
    fn division_by_zero_is_a_violation_not_a_verdict() {
        let violation = evaluate("1 / 0 > 0").unwrap_err();
        assert!(
            violation.0.contains("rejected by the engine"),
            "{violation}"
        );
    }

    #[test]
    fn non_boolean_results_are_violations() {
        let violation = evaluate("1 + 1").unwrap_err();
        assert!(violation.0.contains("non-boolean"), "{violation}");
    }

    #[test]
    fn parse_errors_are_violations_with_the_engine_message() {
        let violation = evaluate("abs(1) == 1").unwrap_err(); // not in the dialect: math::abs
        assert!(
            violation.0.contains("rejected by the engine"),
            "{violation}"
        );
    }

    #[test]
    fn oversized_expressions_are_rejected_before_evaluation() {
        let violation = evaluate(&"1+".repeat(EXPRESSION_MAX_CHARS)).unwrap_err();
        assert!(violation.0.contains("maximum"), "{violation}");
    }

    // SC-007: determinism.
    #[test]
    fn identical_expression_twice_yields_identical_results() {
        let expr = "math::abs(0.1 + 0.2 - 0.3) <= 0.0000001";
        assert_eq!(evaluate(expr).unwrap(), evaluate(expr).unwrap());
    }
}
