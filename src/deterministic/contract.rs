//! The check tool's wire types (contract:
//! `specs/005-deterministic-layer/contracts/check.tool.json`). MCP-side only.
//!
//! Field-consistency invariant (server-guaranteed, data-model.md §2):
//! verdict ≠ not_checkable ⇒ engine/formal_form/engine_result present;
//! verdict = not_checkable ⇒ reason present, the engine fields null.

use crate::deterministic::{Engine, Verdict};
use serde::{Deserialize, Serialize};

/// `check` input.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CheckParams {
    /// The claim to check, in natural language. Best for arithmetic,
    /// quantitative comparisons, and logical/constraint consistency.
    pub claim: String,
    /// Optional background the claim depends on (definitions, given values).
    pub context: Option<String>,
}

/// `check` output.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct CheckResult {
    /// supported | refuted | not_checkable — engine-decided or an honest
    /// decline (FR-002/FR-004).
    pub verdict: Verdict,
    /// Which engine executed the check; null iff not_checkable.
    pub engine: Option<Engine>,
    /// The executed expression or constraint script — audit the translation
    /// without re-running it (FR-007).
    pub formal_form: Option<String>,
    /// The engine's raw result (evaluated value, or sat/unsat).
    pub engine_result: Option<String>,
    /// Solver model when one exists: the satisfying assignment, or the
    /// counterexample refuting an impossibility claim.
    pub witness: Option<String>,
    /// Deterministic, server-assembled explanation tying the engine result
    /// to the claim (research.md 005 D4 — never model-phrased).
    pub explanation: String,
    /// Why the claim is not checkable (verdict = not_checkable only).
    pub reason: Option<String>,
    /// 1 or 2 (the violation-fed retry — FR-005).
    pub translation_attempts: u32,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The derived schemas and the checked-in contract file share exactly the
    /// same property sets, both directions (003/004 pattern).
    #[test]
    fn derived_schemas_match_the_contract_file() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../specs/005-deterministic-layer/contracts/check.tool.json"
        ))
        .unwrap();
        let props = |schema: &Value| -> Vec<String> {
            schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };

        let input = serde_json::to_value(schemars::schema_for!(CheckParams)).unwrap();
        assert_eq!(props(&input), props(&contract["inputSchema"]));
        let output = serde_json::to_value(schemars::schema_for!(CheckResult)).unwrap();
        assert_eq!(props(&output), props(&contract["outputSchema"]));

        // The verdict enum matches the contract's values.
        let verdict_values: Vec<String> = contract["outputSchema"]["properties"]["verdict"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(verdict_values, ["supported", "refuted", "not_checkable"]);
    }
}
