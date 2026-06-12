//! The memory tools' wire types (contracts: `specs/003-memory-layer/contracts/`).
//!
//! These schemas are MCP-side only — there is no model hop for memory tool
//! outputs, so the grammar subset and the flat+closed invariant do not apply
//! (research.md 003 D6); the nested `recall` array is legal here.

use crate::memory::{Kind, Trust};
use serde::{Deserialize, Serialize};

/// `save` input (contract: `contracts/save.tool.json`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SaveParams {
    /// The memory itself, self-contained.
    pub content: String,
    /// skill | lesson | fact.
    pub kind: Kind,
    /// Where this knowledge came from.
    pub origin: String,
    /// true if sourced from external content rather than first-hand experience.
    pub external: bool,
    /// Optional tags.
    pub tags: Option<Vec<String>>,
    /// Run independent verification before admitting an external memory as
    /// trusted.
    pub verify: Option<bool>,
}

/// `save` output.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SaveResult {
    /// The new memory's id.
    pub id: String,
    /// Derived trust standing.
    pub trust: Trust,
    /// Verification findings when verification ran; empty otherwise.
    pub findings: Vec<String>,
}

/// `recall` input (contract: `contracts/recall.tool.json`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecallParams {
    /// What you need, in natural language.
    pub query: String,
    /// Optional kind filter.
    pub kind: Option<Kind>,
    /// Optional result limit (1..=20; default from config).
    pub limit: Option<u32>,
}

/// One recalled memory.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RecalledMemory {
    /// Memory id (usable with `forget`).
    pub id: String,
    /// The memory content.
    pub content: String,
    /// skill | lesson | fact.
    pub kind: Kind,
    /// Stated provenance.
    pub origin: String,
    /// External-content provenance.
    pub external: bool,
    /// Trust standing.
    pub trust: Trust,
    /// RFC 3339 creation time.
    pub created_at: String,
    /// Raw relevance to the query.
    pub score: f32,
}

/// `recall` output — nested array is legal here (no model hop; D6).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RecallResult {
    /// Ranked memories, most relevant first.
    pub memories: Vec<RecalledMemory>,
}

/// `forget` input (contract: `contracts/forget.tool.json`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ForgetParams {
    /// The memory id to permanently delete.
    pub id: String,
}

/// `forget` output.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ForgetResult {
    /// Always true on success (an unknown id is an error, not `false`).
    pub forgotten: bool,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The derived schemas and the checked-in contract files share exactly the
    /// same property sets, both directions.
    #[test]
    fn derived_schemas_match_the_contract_files() {
        let props = |schema: &Value, key: &str| -> Vec<String> {
            schema[key]["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };
        let derived_props = |schema: &Value| -> Vec<String> {
            schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };

        let save_contract: Value = serde_json::from_str(include_str!(
            "../../specs/003-memory-layer/contracts/save.tool.json"
        ))
        .unwrap();
        let save_in = serde_json::to_value(schemars::schema_for!(SaveParams)).unwrap();
        let save_out = serde_json::to_value(schemars::schema_for!(SaveResult)).unwrap();
        assert_eq!(
            derived_props(&save_in),
            props(&save_contract, "inputSchema")
        );
        assert_eq!(
            derived_props(&save_out),
            props(&save_contract, "outputSchema")
        );

        let recall_contract: Value = serde_json::from_str(include_str!(
            "../../specs/003-memory-layer/contracts/recall.tool.json"
        ))
        .unwrap();
        let recall_in = serde_json::to_value(schemars::schema_for!(RecallParams)).unwrap();
        let recall_out = serde_json::to_value(schemars::schema_for!(RecallResult)).unwrap();
        assert_eq!(
            derived_props(&recall_in),
            props(&recall_contract, "inputSchema")
        );
        assert_eq!(
            derived_props(&recall_out),
            props(&recall_contract, "outputSchema")
        );

        let forget_contract: Value = serde_json::from_str(include_str!(
            "../../specs/003-memory-layer/contracts/forget.tool.json"
        ))
        .unwrap();
        let forget_in = serde_json::to_value(schemars::schema_for!(ForgetParams)).unwrap();
        let forget_out = serde_json::to_value(schemars::schema_for!(ForgetResult)).unwrap();
        assert_eq!(
            derived_props(&forget_in),
            props(&forget_contract, "inputSchema")
        );
        assert_eq!(
            derived_props(&forget_out),
            props(&forget_contract, "outputSchema")
        );
    }
}
