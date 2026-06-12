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

/// Count `(assert ...)` forms tolerantly: `(`, optional whitespace, the word
/// `assert` at a word boundary (so `(assert-soft` does not count and
/// `( assert` does). Shared with the cross-field validator so the
/// parse-detection arithmetic and the validator can never disagree.
pub(crate) fn count_asserts(script: &str) -> usize {
    let bytes = script.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if script[j..].starts_with("assert") {
                let after = j + "assert".len();
                let boundary = bytes
                    .get(after)
                    .is_none_or(|c| c.is_ascii_whitespace() || *c == b'(' || *c == b')');
                if boundary {
                    count += 1;
                }
            }
        }
        i += 1;
    }
    count
}

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
    // Artifact-pollution note (review finding): Z3's Debug C++ build opens
    // `.z3-trace` in the process cwd from a GLOBAL INITIALIZER — no runtime
    // parameter can prevent it. The fix is at build level: Cargo.toml forces
    // `opt-level = 3` for z3-sys in every profile, which builds Z3's C++ as
    // Release and compiles the `_TRACE` machinery out entirely. The
    // `no_trace_file_is_written_to_the_cwd` test pins this.

    if smtlib.contains('\0') {
        return Err(Violation("the script contains a NUL byte".to_string()));
    }
    let expected = count_asserts(smtlib);
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

    #[test]
    fn assert_counting_is_tolerant_and_boundary_aware() {
        assert_eq!(count_asserts("(assert true)"), 1);
        assert_eq!(count_asserts("( assert true)(  assert false)"), 2);
        assert_eq!(count_asserts("(assert-soft true)"), 0); // word boundary
        assert_eq!(count_asserts("(asserting x)"), 0);
        assert_eq!(count_asserts("(declare-const x Int)"), 0);
    }

    // Review finding: the bundled Z3 (debug profile) writes `.z3-trace`
    // into the process cwd unless tracing is disabled — a stdio server runs
    // in the client's cwd, so that would be artifact pollution.
    #[test]
    fn no_trace_file_is_written_to_the_cwd() {
        let _ = std::fs::remove_file(".z3-trace");
        check(
            "(declare-const t Int)
(assert (> t 0))",
        )
        .unwrap();
        assert!(
            !std::path::Path::new(".z3-trace").exists(),
            "z3 tracing must stay disabled"
        );
    }

    // SC-007: determinism.
    #[test]
    fn identical_script_twice_yields_identical_outcomes() {
        let script = "(declare-const n Int)\n(assert (> n 100))\n(assert (< n 102))";
        assert_eq!(check(script).unwrap(), check(script).unwrap());
    }
}
