//! The deterministic layer: checkable claims settled by execution, not
//! judgment (`DETERMINISTIC_LAYER.md`).
//!
//! The model's roles are exactly three — classify checkability, translate to
//! a small typed formal target, explain nothing (even the explanation is a
//! server-assembled template). The verdict comes only from engine execution:
//! no judge to fool, no calibration knob, no sycophancy (Principle V).

pub mod arithmetic;
pub mod bundle;
pub mod check;
pub mod contract;
mod exact;
pub mod solver;
pub mod translate;

use serde::{Deserialize, Serialize};

/// In-engine solver timeout (research.md 005 D6 — a constant, not config).
pub const SOLVER_TIMEOUT_MS: u32 = 10_000;
/// Maximum arithmetic expression length.
pub const EXPRESSION_MAX_CHARS: usize = 2_000;
/// Maximum SMT-LIB 2 script length.
pub const SMTLIB_MAX_CHARS: usize = 10_000;
/// Initial translation + one violation-fed retry (FR-005).
pub const TRANSLATION_ATTEMPTS_MAX: u32 = 2;
/// Maximum separately checked parts in one claim.
///
/// This bounds the size of the translator's reply and a call's total engine
/// work; how many parts execute at once is [`PARTS_CONCURRENCY`].
pub const PARTS_MAX: usize = 12;
/// How many parts of one claim execute at the same time.
///
/// The solver's timeout is clock time, so every simultaneous run makes the
/// others likelier to exceed it; one after another, twelve slow parts would
/// outlast the request timeout. Four bounds both: a part competes with at
/// most three others, and twelve slow parts take about three timeouts.
pub const PARTS_CONCURRENCY: usize = 4;

/// The check verdict (contract `check.tool.json`). `NotCheckable` is a
/// successful, honest outcome (FR-004) — not an error class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The engine's execution supports the claim.
    Supported,
    /// The engine's execution refutes the claim.
    Refuted,
    /// The claim cannot be honestly formalized (decline-biased classifier).
    NotCheckable,
}

/// Which deterministic engine executed the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// Boolean-valued expression evaluation (evalexpr).
    Arithmetic,
    /// Constraint satisfiability (Z3, SMT-LIB 2).
    Constraints,
}

/// The claim's asserted polarity for constraint problems: what the claim
/// says about the constraint system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Polarity {
    /// The claim asserts the constraints can be satisfied.
    Satisfiable,
    /// The claim asserts the constraints cannot be satisfied.
    Unsatisfiable,
}

/// A REAL engine/validation violation — the only signal that triggers the
/// single re-translation (research.md 005 D5). Ground truth, not opinion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation(pub String);

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn verdict_and_engine_serialize_to_the_contract_strings() {
        assert_eq!(
            serde_json::to_value(Verdict::NotCheckable).unwrap(),
            serde_json::Value::String("not_checkable".into())
        );
        assert_eq!(
            serde_json::to_value(Engine::Constraints).unwrap(),
            serde_json::Value::String("constraints".into())
        );
        let polarity: Polarity =
            serde_json::from_value(serde_json::Value::String("unsatisfiable".into())).unwrap();
        assert_eq!(polarity, Polarity::Unsatisfiable);
    }
}
