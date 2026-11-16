//! The check tool's wire types (contract:
//! `specs/005-deterministic-layer/contracts/check.tool.json`). MCP-side only.
//!
//! Field-consistency invariant (server-guaranteed, data-model.md §2), which
//! holds for every entry of `parts` and for the top level of a one-part call:
//! verdict ≠ not_checkable ⇒ engine/formal_form/engine_result present;
//! verdict = not_checkable ⇒ reason present, the engine fields null.
//!
//! For a bundle of two or more parts the top level is a summary
//! (`bundle.rs`): `formal_form` and `engine_result` hold one `[n] …` line per
//! part whenever at least one part was executed — including under a
//! `not_checkable` verdict — and `engine` is set only when every part ran on
//! the same engine.

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
    /// Reasoning effort for this call alone. Omit to use the deployment's
    /// configured level. Not every model family accepts this; when one rejects
    /// it the error names the model, the level, and the remedies.
    ///
    /// Typed rather than a free string so the accepted levels appear in the
    /// published schema — the consumer of this server is a model reading that
    /// schema, and a prose-only list is not something it can be constrained by.
    #[serde(default)]
    pub effort: Option<crate::routing::Effort>,
}

/// `check` output.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct CheckResult {
    /// supported | refuted | not_checkable — engine-decided or an honest
    /// decline (FR-002/FR-004).
    pub verdict: Verdict,
    /// Which engine executed the check. For a single claim: null iff
    /// not_checkable. For a bundle of two or more parts: set only when every
    /// part ran on the same engine, so it can be null beside any verdict; read
    /// each part's engine in parts.
    pub engine: Option<Engine>,
    /// The executed expression or constraint script — audit the translation
    /// without re-running it (FR-007). For a bundle of two or more parts: one
    /// `[n] form` line per part in part order, `(not checked)` for a part no
    /// engine ran, a script collapsed onto its line; null when no part ran.
    /// The exact forms are in parts.
    pub formal_form: Option<String>,
    /// The engine's raw result (evaluated value, or sat/unsat). For a bundle:
    /// one `[n] result` line per part, `(not checked)` for a part no engine
    /// ran; null when no part ran.
    pub engine_result: Option<String>,
    /// Solver model when one exists: the satisfying assignment, or the
    /// counterexample refuting an impossibility claim. For a bundle: one
    /// `[n] witness` line for each part that has one, collapsed onto its
    /// line; null when no part has one.
    pub witness: Option<String>,
    /// Deterministic, server-assembled explanation tying the engine result
    /// to the claim (research.md 005 D4 — never model-phrased).
    pub explanation: String,
    /// Why the claim is not checkable (verdict = not_checkable only). For a
    /// bundle whose top-level verdict is not_checkable: first a sentence
    /// saying nothing can confirm the parts cover every statement of the
    /// claim, then one `[n] reason` line for each part that was not checked.
    pub reason: Option<String>,
    /// 1 or 2 (the violation-fed retry — FR-005).
    pub translation_attempts: u32,
    /// One entry per separately checked part, in the order the claim states
    /// them. A single claim has exactly one part. For a bundle, read each
    /// part's verdict here: the top-level verdict of a bundle is `refuted` if
    /// any part is refuted and `not_checkable` otherwise — never `supported`,
    /// because nothing can confirm the parts cover every statement of the
    /// claim — so which parts held is visible only in this list.
    #[schemars(length(min = 1, max = crate::deterministic::PARTS_MAX))]
    pub parts: Vec<PartResult>,
}

/// One separately checked part of the claim.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct PartResult {
    /// The part, restated by the translator so it stands alone.
    pub claim: String,
    /// This part's verdict.
    pub verdict: Verdict,
    /// Which engine executed this part; null iff it was not checked.
    pub engine: Option<Engine>,
    /// The exact expression or constraint script executed for this part.
    pub formal_form: Option<String>,
    /// The engine's raw result for this part.
    pub engine_result: Option<String>,
    /// Solver model for this part, when one exists.
    pub witness: Option<String>,
    /// Why this part is not checkable (verdict = not_checkable only).
    pub reason: Option<String>,
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
        // 029: names alone are not the contract. Two defects shipped past a
        // name-only comparison — `effort` as an untyped string, and `passes`
        // publishing `minimum: 0` while the server rejects zero.
        crate::schema::assert_constraints_agree(&contract["inputSchema"], &input, "check");
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

    /// A `parts` entry is bound to the contract structurally, not by one
    /// sample response: the same field names both directions, and the same
    /// item limits on the list — which are the engine's own `PARTS_MAX`.
    #[test]
    fn the_parts_entry_and_its_limits_match_the_contract_file() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../specs/005-deterministic-layer/contracts/check.tool.json"
        ))
        .unwrap();
        let contract_parts = &contract["outputSchema"]["properties"]["parts"];
        let output = serde_json::to_value(schemars::schema_for!(CheckResult)).unwrap();
        let derived_parts = &output["properties"]["parts"];

        let keys = |schema: &Value| -> Vec<String> {
            schema["properties"]
                .as_object()
                .expect("an object schema with properties")
                .keys()
                .cloned()
                .collect()
        };
        let entry = serde_json::to_value(schemars::schema_for!(PartResult)).unwrap();
        assert_eq!(keys(&entry), keys(&contract_parts["items"]));
        // The list really is a list of that entry, not some other shape.
        assert_eq!(derived_parts["type"], "array");
        assert_eq!(derived_parts["items"]["$ref"], "#/$defs/PartResult");

        for limit in ["minItems", "maxItems"] {
            assert!(contract_parts[limit].is_u64(), "contract lacks {limit}");
            assert_eq!(
                derived_parts[limit], contract_parts[limit],
                "published {limit} differs from the contract"
            );
        }
        assert_eq!(
            contract_parts["maxItems"].as_u64().unwrap(),
            crate::deterministic::PARTS_MAX as u64
        );
    }
}
