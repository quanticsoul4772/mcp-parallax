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
            "expression evaluated to the non-boolean value {value}; the formal target \
             must be a boolean-valued claim encoding (e.g. a comparison)"
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
            ("math::abs(2^32 - 4294967296) <= 0.5", true),
            ("2^31 - 1.0 >= 2147483648.0", false), // i32::MAX is smaller
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
            ("-7 % 2 == -1 && 7 % -2 == 1 && -7 % 3 == -1", true), // truncated, as in C
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
                violation.0.contains("== or != over"),
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
            "0^-0.5 > 0",
            "true || 1/0 > 0", // no short circuit hides it
            "2^0.5 / (2 - 2) > 0",
            "-7 % 0 == 0",
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(violation.0.contains("by zero"), "{expression}: {violation}");
        }
    }

    // Constructs evalexpr would evaluate but the prompted dialect lacks.
    #[test]
    fn constructs_outside_the_dialect_are_violations() {
        for (expression, reason) in [
            ("a = 1/3; true && a*3 == 1", "not in the dialect"),
            ("if(true, 1, 2) == 1", "`if` is not in the dialect"),
            ("len(\"1/2\") < 4", "not in the dialect"),
            ("x > 1", "not in the dialect"),
            ("math::sqrt(4) == 2", "`math::sqrt` is not in the dialect"),
            ("abs(1) == 1", "`abs` is not in the dialect"),
            ("(1, 2) == (1, 2)", "not in the dialect"),
            ("\"a\" == \"a\"", "not in the dialect"),
            ("1 + true > 0", "boolean was used where a number"),
            ("!3", "number was used where a boolean"),
            ("floor(1, 2) > 0", "was given 2 arguments"),
            ("", "empty expression"),
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(violation.0.contains(reason), "{expression}: {violation}");
        }
    }

    // evalexpr would round each of these to an f64 before its tree exists;
    // literals are read from the text instead, so every digit counts.
    #[test]
    fn literals_are_exact_at_any_length_and_magnitude() {
        for (expression, expected) in [
            ("1.00000000000000001 == 1", false),
            ("1.00000000000000001 > 1", true),
            ("0.30000000000000004 > 0.3", true), // Python's 0.1 + 0.2
            ("13.572000000000001 - 13.572 == 0.000000000000001", true),
            ("9223372036854775808 - 9223372036854775807 == 1", true), // past i64
            ("-9223372036854775808 < 0", true),
            ("123456789012345678901234567890 % 10 == 0", true),
            ("1e-400 > 0 && 1e-400 * 1e400 == 1", true),
            ("4e-324 == 5e-324", false),
            ("1e3 == 1000 && 1.5E+3 == 1500 && 25e-1 == 2.5", true),
            (".5 == 1/2 && 5. == 5 && 0.50 == .5", true),
            (
                "1.000000000000000 == 1 && 100000000000000000000.0 > 0",
                true,
            ),
        ] {
            assert_eq!(
                evaluate(expression).unwrap().holds,
                expected,
                "{expression}"
            );
        }
        for (expression, reason) in [
            ("0x10 == 16", "not a decimal number"),
            ("1.2.3 > 0", "not a decimal number"),
            ("2e > 0", "not a decimal number"),
            ("1_000 > 0", "not a decimal number"),
            ("1e999999999 > 0", "engine can hold"),
            ("max(1,000, 2) == 2", "leading zero"),
            ("007 == 7", "leading zero"),
            ("1e2400 * 1e2400 > 0", "size limit"),
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(violation.0.contains(reason), "{expression}: {violation}");
        }
    }

    // Size and depth are bounded, so no input within the length cap can
    // hang the engine or overflow its stack.
    #[test]
    fn runaway_inputs_are_violations_not_hangs_or_crashes() {
        for (expression, reason) in [
            ("2^5000 > 0".to_owned(), "beyond the engine's range"),
            ("((2^4096)^4096)^4096 > 0".to_owned(), "size limit"),
            // Only the per-value check sees these: no single power is too big.
            ("7^1000 * 7^1000 * 7^1000 > 0".to_owned(), "size limit"),
            ("(2^0.5 * 7^1000)^8 > 0".to_owned(), "size limit"),
            ("(9^4096)^80 * (9^4096)^80 > 0".to_owned(), "size limit"),
            (
                "2 ^ -2147483648 > 0".to_owned(),
                "beyond the engine's range",
            ),
            (format!("{}1 < 0", "-".repeat(1990)), "nested too deeply"),
            (format!("{}true", "!".repeat(1990)), "nested too deeply"),
            (format!("{}1 > 0", "1+".repeat(990)), "nested too deeply"),
        ] {
            let violation = evaluate(&expression).unwrap_err();
            assert!(violation.0.contains(reason), "{violation}");
        }
        // A long sum within the depth bound is fine.
        assert!(
            evaluate(&format!("{}1 == 401", "1+".repeat(400)))
                .unwrap()
                .holds
        );
        // A huge non-boolean result is reported, not printed in full.
        let violation = evaluate("7^1400").unwrap_err();
        assert!(violation.0.contains("characters)"), "{violation}");
        assert!(violation.0.len() < 400, "{}", violation.0.len());
        let violation = evaluate("2^(7^1400) > 0").unwrap_err();
        assert!(violation.0.len() < 400, "{}", violation.0.len());
    }

    // Second review: f64 noise left by cancelling two roots was scaled up
    // and answered. A root is a bracket now, and the bracket is scaled with it.
    #[test]
    fn cancelled_roots_are_never_answered_from_noise() {
        for expression in [
            "((2^0.5)^2 - 2) * 10^20 > 1",
            "(2^0.5 * 2^0.5 - 2) * 10^16 < 1",
            "math::abs((2^0.5 * 2^0.5 - 2) * 10^12) <= 0.0001",
            "math::abs((3^0.5)^2 - 3) * 10^15 >= 0.4",
            "(5^0.5)^2 * 10^16 - 5 * 10^16 < 1",
            "(2^0.5 + 10^-20 - 2^0.5) * 10^30 > 1",
            "floor(((2^0.5)^2 - 2) * 10^16 + 0.5) == 4",
            "min(2, (2^0.5)^2 - 10^-20) >= 2",
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(
                violation.0.contains("too close"),
                "{expression}: {violation}"
            );
        }
        let violation = evaluate("1/((2^0.5)^2 - 2) > 1000").unwrap_err();
        assert!(violation.0.contains("by zero"), "{violation}");
        // min/max keep the bracket, whatever the argument order.
        for expression in [
            "max(1414213562373095048/10^18, 2^0.5) == 1414213562373095048/10^18",
            "min(2, (2^0.5)^2 - 10^-20) == 2",
            "max(2, 4^0.5) == 2",
            "max(4^0.5, 2) == 2",
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(
                violation.0.contains("== or != over"),
                "{expression}: {violation}"
            );
        }
    }

    // The bracket is exact interval arithmetic: every sign case, and wide
    // enough to hold the true value.
    #[test]
    fn root_brackets_follow_signs_and_contain_the_truth() {
        for expression in [
            "-(2^0.5) < -1.41 && -(2^0.5) > -1.42",
            "math::abs(-(2^0.5)) > 1.41 && math::abs(-(2^0.5)) < 1.42",
            "math::abs(2^0.5 - 3^0.5) > 0.31 && math::abs(2^0.5 - 3^0.5) < 0.32",
            "math::abs((2^0.5)^2 - 2) <= 0.0001",
            "-(2^0.5) * 3^0.5 < -2.44 && -(2^0.5) * 3^0.5 > -2.45",
            "(-(2^0.5)) * (-(3^0.5)) > 2.44 && (-(2^0.5)) * (-(3^0.5)) < 2.45",
            "2^0.5 / -(3^0.5) < -0.81 && 2^0.5 / -(3^0.5) > -0.82",
            "(-(2^0.5))^3 < -2.82 && (-(2^0.5))^3 > -2.83",
            "(-(2^0.5))^2 > 1.99 && (-(2^0.5))^-2 < 0.51",
            "min(-(2^0.5), 3^0.5) < -1.41 && max(-(2^0.5), 3^0.5) > 1.73",
        ] {
            assert!(evaluate(expression).unwrap().holds, "{expression}");
        }
        // f64 is off by about 1e-13 here (third review, worst case found):
        // (3^640)^(641/640) is exactly 3^641, so neither strict comparison
        // may be answered. A bracket narrower than the error answers one.
        for expression in ["(3^640)^(641/640) > 3^641", "(3^640)^(641/640) < 3^641"] {
            let violation = evaluate(expression).unwrap_err();
            assert!(
                violation.0.contains("too close"),
                "{expression}: {violation}"
            );
        }
    }

    // Roots that floating point separates clearly are answered, at any scale.
    #[test]
    fn clearly_separated_roots_are_answered() {
        for (expression, expected) in [
            ("2^0.5 / 10^20 > 0", true),
            ("2^0.5 / 10^10 > 1 / 10^10", true),
            ("2^0.5 > 1.4142135", true),
            ("2^0.5 > 1.4142136", false),
            ("10^400 > 2^0.5", true),
            ("10^-400 * 2^0.5 * 10^300 * 10^200 > 1", true),
            ("(2^0.5)^2 > 1.9999 && (2^0.5)^-2 < 0.50001", true),
            ("math::abs(2^0.5 - 1.41421356) <= 0.00000001", true),
            ("0^0.5 == 0", true),
        ] {
            assert_eq!(
                evaluate(expression).unwrap().holds,
                expected,
                "{expression}"
            );
        }
        for (expression, reason) in [
            ("(-8)^(1/3) < 0", "negative"),
            ("(2^0.5)^0.5 > 1", "fractional power of a fractional power"),
            ("2^(2^0.5) > 2", "used as an exponent"),
            (
                "(10^-300)^1.5 * 10^300 * 10^300 > 1",
                "outside the range of a 64-bit float",
            ),
            ("2^64.5 > 0", "beyond ±64"),
            ("2^0.5 % 1 > 0", "% over a fractional power"),
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(violation.0.contains(reason), "{expression}: {violation}");
        }
    }

    // evalexpr reads these differently from mathematics, or drops text.
    #[test]
    fn forms_evalexpr_would_misread_are_violations() {
        for (expression, reason) in [
            ("3 < 7 // 2", "comment"),
            ("1 == 1 // && 1 == 2", "comment"),
            ("2 + 2 == 5 /* || true */", "comment"),
            ("2^3^2 == 64", "ambiguous"),
            ("2^3^2 == 512", "ambiguous"),
            ("floor 7/2 == 3", "must be called with parentheses"),
            ("round 2.5*2 == 5", "must be called with parentheses"),
        ] {
            let violation = evaluate(expression).unwrap_err();
            assert!(violation.0.contains(reason), "{expression}: {violation}");
        }
        assert!(
            evaluate("(2^3)^2 == 64 && 2^(3^2) == 512 && -2^2 == -4")
                .unwrap()
                .holds
        );
        assert!(
            evaluate("0.000000000000001 == 1/10^15 && 1e-7 * 1E+7 == 1")
                .unwrap()
                .holds
        );
    }

    #[test]
    fn non_boolean_results_are_violations() {
        let violation = evaluate("1 + 1").unwrap_err();
        assert!(violation.0.contains("non-boolean"), "{violation}");
    }

    #[test]
    fn parse_errors_are_violations_with_the_engine_message() {
        for expression in ["(1 > 0", "1 +* 2 > 0", "1 > 0)"] {
            let violation = evaluate(expression).unwrap_err();
            assert!(
                violation.0.contains("rejected by the engine"),
                "{expression}: {violation}"
            );
        }
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
