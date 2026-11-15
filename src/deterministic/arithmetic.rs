//! The arithmetic engine: a boolean-valued expression, evaluated exactly.
//!
//! evalexpr parses (research.md 005 D2) and [`exact`] evaluates; exact
//! evaluation replaced evalexpr's i64/f64 arithmetic after field wrong
//! verdicts. Pure, deterministic, in-process — tested directly against ground
//! truth, never mocked.

use crate::deterministic::{exact, Violation, EXPRESSION_MAX_CHARS};

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
/// errors, a non-boolean result, an oversized expression, or a comparison
/// over a fractional power that floating point cannot decide.
pub fn evaluate(expression: &str) -> Result<ArithmeticOutcome, Violation> {
    let len = expression.chars().count();
    if len > EXPRESSION_MAX_CHARS {
        return Err(Violation(format!(
            "expression is {len} characters; the maximum is {EXPRESSION_MAX_CHARS}"
        )));
    }
    match exact::evaluate(expression) {
        Ok(exact::Outcome::Boolean(holds)) => Ok(ArithmeticOutcome {
            holds,
            result_text: holds.to_string(),
        }),
        Ok(exact::Outcome::NonBoolean(value)) => Err(Violation(format!(
            "expression evaluated to the non-boolean value {value}; the formal target              must be a boolean-valued claim encoding (e.g. a comparison)"
        ))),
        Err(reason) => Err(Violation(format!(
            "expression `{expression}` rejected by the engine: {reason}"
        ))),
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

    // Acceptance run 1: `0.15 * 240 == 36` was false in f64. Decimal literals
    // are exact fractions now, so exact equality over them is simply right.
    #[test]
    fn equality_over_decimals_and_division_is_exact() {
        for (expression, expected) in [
            ("0.15 * 240 == 36", true),
            ("2^32 == 4294967296", true),
            ("7 / 2 != 3.5", false),
            ("0.1 + 0.2 == 0.3", true),
            ("1e3 == 1000", true),
            ("floor(7) == 7 && round(5) == 5 && ceil(7) != 8", true),
            ("(60+48==108) && (math::abs(90/144 - 0.62) <= 0.005)", true),
            ("(60+48==109) && (math::abs(90/144 - 0.62) <= 0.005)", false),
        ] {
            assert_eq!(
                evaluate(expression).unwrap().holds,
                expected,
                "{expression}"
            );
        }
    }

    // Field report 2026-11-14: `25149/3600` with integer operands evaluated to
    // 6 (evalexpr integer division) and refuted a true claim.
    #[test]
    fn division_is_exact_not_truncating() {
        for (expression, expected) in [
            ("math::abs(90/144 - 0.625) <= 0.005", true),
            ("90/144 == 0.625", true),
            ("25149/3600 > 6.9", true),
            ("25149/3600 > 7", false),
            ("1/(1/2) > 1", true),
            ("10/(3/4) > 13", true),
            ("5 % (1/2) < 1", true),
            ("-7 % 2 == -1 && 7 % -2 == 1", true), // truncated, as in Rust
            (
                "floor(7/2) == 3 && floor(-7/2) == -4 && round(5/2) == 3",
                true,
            ),
            ("round(-1/2) == -1", true), // half away from zero
            ("2^-2 == 0.25", true),
        ] {
            assert_eq!(
                evaluate(expression).unwrap().holds,
                expected,
                "{expression}"
            );
        }
    }

    // f64 gets every one of these wrong at the boundary.
    #[test]
    fn boundary_comparisons_are_exact() {
        for expression in [
            "0.1 + 0.2 <= 0.3",
            "1/10 + 2/10 <= 3/10",
            "3 * (1/10) <= 3/10",
            "29/100*100 >= 29",
            "floor(29/100*100) >= 29",
            "(1/49)*49 >= 1",
        ] {
            assert!(evaluate(expression).unwrap().holds, "{expression}");
        }
    }

    // Above 2^53 an f64 cannot tell neighbouring integers apart.
    #[test]
    fn large_integers_keep_every_digit() {
        for expression in [
            "9007199254740993 != 9007199254740992 && 1/2 < 1",
            "9007199254740993/1 > 9007199254740992",
            "123456789 * 987654321 / 1 > 121932631112635268",
            "max(9007199254740993, 1/2) > 9007199254740992",
            "9223372036854775807/2 < 4611686018427387904",
            "9223372036854775807 + 1 > 9223372036854775807", // no i64 overflow
            "2^200 + 1 > 2^200",
        ] {
            assert!(evaluate(expression).unwrap().holds, "{expression}");
        }
    }

    // A fractional exponent has no exact value: it is computed in f64, and
    // the engine answers only when f64 can.
    #[test]
    fn fractional_powers_are_decided_only_when_floating_point_can() {
        for (expression, expected) in [
            ("math::abs(27^(1/3) - 3) <= 0.0001", true),
            ("2^(1/2) > 1.4 && 2^0.5 < 1.5", true),
            ("2^(1/3) > 1.3", false),
            ("floor(10^0.5) == 3", true),
        ] {
            assert_eq!(
                evaluate(expression).unwrap().holds,
                expected,
                "{expression}"
            );
        }
        for expression in ["2^0.5 * 2^0.5 == 2", "27^(1/3) != 3"] {
            let violation = evaluate(expression).unwrap_err();
            assert!(
                violation.0.contains("tolerance"),
                "{expression}: {violation}"
            );
        }
        for expression in ["64^(1/3) >= 4", "1000^(1/3) >= 10", "floor(64^(1/3)) >= 4"] {
            let violation = evaluate(expression).unwrap_err();
            assert!(
                violation.0.contains("too close"),
                "{expression}: {violation}"
            );
        }
    }

    #[test]
    fn division_by_zero_is_a_violation_not_a_verdict() {
        for expression in [
            "1 / 0 > 0",
            "1.0 / 0 > 0",
            "1 / 0.0 > 0",
            "1/(1/2 - 0.5) > 0",
            "0/(1/2 - 0.5) <= 0",
            "10 % (7/2 - 3.5) >= 0",
            "0^-1 > 0",
            "true || 1/0 > 0", // no short circuit hides it
            "2^0.5 / (2 - 2) > 0",
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(violation.0.contains("by zero"), "{expression}: {violation}");
        }
    }

    // Nothing outside the prompted dialect is evaluated.
    #[test]
    fn everything_outside_the_dialect_is_a_violation() {
        for expression in [
            "a = 1/3; true && a*3 == 1",
            "(1/3)*3 * if(true && true, 1, 1) == 1",
            "len(\"1/2\") < 4",
            "x > 1",
            "math::sqrt(4) == 2",
            "(1, 2) == (1, 2)",
            "\"a\" == \"a\"",
            "1 + true > 0",
            "!3",
            "min() > 0",
            "floor(1, 2) > 0",
            "",
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(
                violation.0.contains("rejected by the engine"),
                "{expression}: {violation}"
            );
        }
    }

    // evalexpr parses these to a rounded f64 before the tree exists.
    #[test]
    fn literals_the_parser_would_round_are_violations() {
        for expression in [
            "9223372036854775808 > 0",
            "0.12345678901234567 > 0",
            "1.00000000000000001 == 1",
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(violation.0.contains("literal"), "{expression}: {violation}");
        }
        assert!(
            evaluate("math::abs(1e-3 - 1/1000) == 0 && 1.5e3 == 1500")
                .unwrap()
                .holds
        );
    }

    #[test]
    fn runaway_powers_are_violations() {
        for expression in [
            "2^5000 > 0",
            "((2^4096)^4096)^4096 > 0",
            "10^400 * 1.5 ^ 0.5 > 0",
        ] {
            assert!(evaluate(expression).is_err(), "{expression}");
        }
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
