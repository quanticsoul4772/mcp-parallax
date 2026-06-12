//! The constraint engine: Z3 over SMT-LIB 2 (research.md 005 D1). Pure-ish,
//! in-process, deterministic — tested directly against ground truth.
//!
//! Parse-failure detection without unsafe FFI (spike S1 finding): the crate's
//! `from_string` records Z3 errors in the context but returns `()`; counting
//! parsed assertions against the script's `(assert` count surfaces full AND
//! partial parse failures deterministically. NUL bytes are rejected first —
//! the crate's internal `CString::new` would panic on them.

use crate::deterministic::{Violation, SOLVER_TIMEOUT_MS};
use z3::{Params, SatResult, Solver};

/// A completed solver run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolverOutcome {
    /// Satisfiable, with the model rendered as a readable assignment.
    Sat {
        /// The satisfying assignment (the unforgeable witness).
        witness: String,
    },
    /// Proven unsatisfiable.
    Unsat,
    /// The solver gave up — timeout or theory incompleteness (mapped to the
    /// `timeout` class by the orchestrator; the message stays honest about
    /// both causes).
    Unknown,
}

/// Check an SMT-LIB 2 script (declares + asserts; `check-sat` is appended by
/// the engine) under [`SOLVER_TIMEOUT_MS`].
///
/// # Errors
///
/// Returns a [`Violation`] (retryable — research.md D5) for NUL bytes,
/// scripts with no assertions, and full or partial parse failures.
pub fn check(smtlib: &str) -> Result<SolverOutcome, Violation> {
    if smtlib.contains('\0') {
        return Err(Violation("the script contains a NUL byte".to_string()));
    }
    let expected = smtlib.matches("(assert").count();
    if expected == 0 {
        return Err(Violation(
            "the script contains no (assert ...) form".to_string(),
        ));
    }

    let solver = Solver::new();
    let mut params = Params::new();
    params.set_u32("timeout", SOLVER_TIMEOUT_MS);
    solver.set_params(&params);
    solver.from_string(smtlib);

    let parsed = solver.get_assertions().len();
    if parsed < expected {
        return Err(Violation(format!(
            "SMT-LIB parse error: only {parsed} of {expected} assertions were accepted — \
             check declarations and syntax"
        )));
    }

    match solver.check() {
        SatResult::Sat => {
            let witness = solver
                .get_model()
                .map_or_else(|| "(model unavailable)".to_string(), |m| m.to_string());
            Ok(SolverOutcome::Sat {
                witness: witness.trim().to_string(),
            })
        }
        SatResult::Unsat => Ok(SolverOutcome::Unsat),
        SatResult::Unknown => Ok(SolverOutcome::Unknown),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    // T005 ground-truth table: the solver, not opinion, decides.
    #[test]
    fn satisfiable_system_yields_sat_with_a_witness_naming_the_variables() {
        let outcome = check(
            "(declare-const x Int)\n(declare-const y Int)\n\
             (assert (> x 2))\n(assert (< y 10))\n(assert (= (+ x y) 11))",
        )
        .unwrap();
        let SolverOutcome::Sat { witness } = outcome else {
            panic!("expected Sat, got {outcome:?}");
        };
        assert!(witness.contains('x') && witness.contains('y'), "{witness}");
    }

    #[test]
    fn contradiction_yields_unsat() {
        let outcome = check("(declare-const a Bool)\n(assert a)\n(assert (not a))").unwrap();
        assert_eq!(outcome, SolverOutcome::Unsat);
    }

    #[test]
    fn malformed_script_is_a_violation_via_the_assertion_count() {
        let violation =
            check("(declare-const z Int)\n(assert (> z 0))\n(assert (this is not smtlib")
                .unwrap_err();
        assert!(violation.0.contains("parse error"), "{violation}");
    }

    #[test]
    fn undeclared_variable_is_a_violation_not_a_crash() {
        let violation = check("(assert (> undeclared 0))").unwrap_err();
        assert!(violation.0.contains("parse error"), "{violation}");
    }

    #[test]
    fn nul_bytes_and_assertion_free_scripts_are_violations() {
        assert!(check("(assert true)\0").unwrap_err().0.contains("NUL"));
        assert!(check("(declare-const x Int)")
            .unwrap_err()
            .0
            .contains("no (assert"));
    }

    // SC-007: determinism.
    #[test]
    fn identical_script_twice_yields_identical_outcomes() {
        let script = "(declare-const n Int)\n(assert (> n 100))\n(assert (< n 102))";
        assert_eq!(check(script).unwrap(), check(script).unwrap());
    }
}
