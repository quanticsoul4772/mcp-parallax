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
    // form via the feedback loop. Pure-integer equality stays exact. The
    // guard is per comparison, so an integer equality may sit beside a
    // tolerance comparison in one bundled claim.
    for comparison in comparisons(expression) {
        let has_equality = comparison.contains("==") || comparison.contains("!=");
        let produces_floats =
            comparison.contains('.') || comparison.contains('/') || comparison.contains('^');
        if has_equality && produces_floats {
            return Err(Violation(format!(
                "the comparison `{}` in the expression `{expression}` uses exact == or !=                  over float-producing arithmetic (a decimal point, /, or ^), which is not                  allowed - rewrite the comparison as math::abs(a - b) <= 0.0001 (an                  explicit numeric tolerance), or rewrite division-free (a/b == c becomes                  a == b*c)",
                comparison.trim()
            )));
        }
    }
    // The expression as written decides whether evaluation is an error at
    // all (integer division by zero stays a violation, never `inf > 0`);
    // the real-division form decides the verdict.
    let written = eval_boolean(expression)?;
    if !expression.contains('/') {
        return Ok(written);
    }
    eval_boolean(&with_real_division(expression))
}

fn eval_boolean(expression: &str) -> Result<ArithmeticOutcome, Violation> {
    match evalexpr::eval(expression) {
        Ok(evalexpr::Value::Boolean(holds)) => Ok(ArithmeticOutcome {
            holds,
            result_text: holds.to_string(),
        }),
        Ok(other) => Err(Violation(format!(
            "expression evaluated to the non-boolean value {other:?}; the formal target              must be a boolean-valued claim encoding (e.g. a comparison)"
        ))),
        Err(e) => Err(Violation(format!("expression rejected by the engine: {e}"))),
    }
}

/// The text between boolean connectives. No arithmetic operand spans `&&` or
/// `||`, so each piece holds whole comparisons; parentheses may be unbalanced
/// within a piece, which neither caller minds.
fn comparisons(expression: &str) -> impl Iterator<Item = &str> {
    expression.split("&&").flat_map(|piece| piece.split("||"))
}

/// Rewrite integer literals as floats in every comparison that divides, so
/// `/` is real division there (evalexpr truncates `int / int`: `90/144` is
/// `0`, and `x^(1/3)` is `x^0`). Comparisons without a `/` are left as
/// written, which keeps their integer arithmetic exact.
fn with_real_division(expression: &str) -> String {
    let mut out = String::with_capacity(expression.len() + 16);
    let mut rest = expression;
    while !rest.is_empty() {
        let end = [rest.find("&&"), rest.find("||")]
            .into_iter()
            .flatten()
            .min()
            .map_or(rest.len(), |at| at + 2);
        let (piece, tail) = rest.split_at(end);
        if piece.contains('/') {
            promote_integer_literals(piece, &mut out);
        } else {
            out.push_str(piece);
        }
        rest = tail;
    }
    out
}

/// Append `piece` to `out` with `.0` after each integer literal. A numeric
/// token that is already a float (`1.5`, `1e-3`) or part of an identifier
/// is copied unchanged.
fn promote_integer_literals(piece: &str, out: &mut String) {
    let word_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.';
    let mut chars = piece.chars().peekable();
    let mut in_word = false;
    while let Some(c) = chars.next() {
        if in_word || !c.is_ascii_digit() {
            out.push(c);
            in_word = word_char(c);
            continue;
        }
        // A token starting with a digit: consume it whole.
        let mut token = String::from(c);
        while let Some(&next) = chars.peek() {
            let exponent_sign =
                matches!(next, '+' | '-') && token.ends_with(['e', 'E']) && !token.contains('x');
            if !(word_char(next) || exponent_sign) {
                break;
            }
            token.push(next);
            chars.next();
        }
        let is_integer = token.chars().all(|d| d.is_ascii_digit());
        out.push_str(&token);
        if is_integer {
            out.push_str(".0");
        }
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

    // Field report 2026-11-14: `25149/3600` with integer operands evaluated to
    // 6 (evalexpr integer division) and refuted a true claim; `x^(1/3)` became
    // `x^0`. Division is real-valued in the dialect.
    #[test]
    fn integer_operands_divide_as_reals_not_truncating() {
        for (expression, expected) in [
            ("math::abs(90/144 - 0.625) <= 0.005", true),
            ("25149/3600 > 6.9", true),
            ("25149/3600 > 7", false),
            ("math::abs(27^(1/3) - 3) <= 0.0001", true),
            ("math::abs(1e-3 - 1/1000) <= 0.0000001", true),
            ("floor(7/2) >= 3 && floor(7/2) < 3.5", true), // explicit floor still truncates
        ] {
            let outcome = evaluate(expression).unwrap();
            assert_eq!(outcome.holds, expected, "{expression}");
        }
    }

    // The equality guard is per comparison: an exact integer equality beside a
    // tolerance comparison is the common bundled-claim shape, and rejecting it
    // was the most frequent `check` failure in the field.
    #[test]
    fn integer_equality_beside_a_float_comparison_is_evaluated() {
        for (expression, expected) in [
            ("(60+48==108) && (math::abs(90/144 - 0.625) <= 0.005)", true),
            (
                "(60+48==109) && (math::abs(90/144 - 0.625) <= 0.005)",
                false,
            ),
            ("(60+48==108) || (0.84-0.27 > 1)", true),
        ] {
            let outcome = evaluate(expression).unwrap();
            assert_eq!(outcome.holds, expected, "{expression}");
        }
    }

    #[test]
    fn float_equality_inside_a_bundle_is_still_a_violation() {
        let violation = evaluate("(3 == 3) && (0.15 * 240 == 36)").unwrap_err();
        assert!(violation.0.contains("tolerance"), "{violation}");
        assert!(violation.0.contains("0.15 * 240 == 36"), "{violation}");
    }

    // Real division must not leak into a neighbouring integer comparison:
    // above 2^53 an f64 cannot tell these two integers apart.
    #[test]
    fn integer_comparisons_stay_exact_beside_a_division() {
        let outcome = evaluate("9007199254740993 != 9007199254740992 && 1/2 < 1").unwrap();
        assert!(outcome.holds);
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
