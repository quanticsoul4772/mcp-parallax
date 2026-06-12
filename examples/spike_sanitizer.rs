//! Spike 1 — schema sanitizer fidelity (T004, research.md).
//!
//! Derives a Verdict-shaped schema via `schemars` (the real pipeline's first
//! hop), sanitizes it, and asserts the output is Anthropic-grammar-legal while
//! the unsanitized schema still enforces value constraints locally.
//!
//! Run: `cargo run --example spike_sanitizer` (no key, no network)

// Spikes are dev tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::schema::{sanitize, validate};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;

/// Stand-in for the aggregated tool output (data-model.md §4).
#[derive(Serialize, JsonSchema)]
#[allow(dead_code)]
struct SpikeVerdict {
    /// supported | refuted
    verdict: VerdictKind,
    /// Specific findings; each refutation names a concrete error.
    findings: Vec<String>,
    /// Agreement ratio in [0, 1] — range is validator-enforced, not grammar.
    confidence: f64,
    /// Passes completed.
    passes: u32,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
enum VerdictKind {
    Supported,
    Refuted,
}

fn main() {
    let derived = schemars::schema_for!(SpikeVerdict);
    let unsanitized = serde_json::to_value(derived).expect("schema serializes");
    println!(
        "=== schemars output ===\n{}",
        serde_json::to_string_pretty(&unsanitized).unwrap()
    );

    let sanitized = sanitize(&unsanitized);
    println!(
        "\n=== sanitized (Anthropic grammar subset) ===\n{}",
        serde_json::to_string_pretty(&sanitized).unwrap()
    );

    // Exit criteria (research.md spike 1):
    // 1. Grammar-legal: every object closed + fully required, constraints gone.
    assert_no_unsupported(&sanitized);
    assert_objects_closed(&sanitized);

    // 2. The UNSANITIZED schema still does the validator's job locally.
    let in_range = json!({ "verdict": "refuted", "findings": ["1066, not 1067"], "confidence": 1.0, "passes": 3 });
    validate(&unsanitized, &in_range).expect("conforming value validates");

    let out_of_range =
        json!({ "verdict": "refuted", "findings": [], "confidence": 7.0, "passes": 3 });
    // schemars emitted no range for confidence (we add ranges in the real mode
    // type via schemars attributes); prove the validator path works with an
    // explicitly-constrained schema:
    let mut constrained = unsanitized.clone();
    constrained["properties"]["confidence"]["minimum"] = json!(0.0);
    constrained["properties"]["confidence"]["maximum"] = json!(1.0);
    validate(&constrained, &out_of_range).expect_err("out-of-range confidence is rejected locally");
    // ...and that sanitize() strips exactly those keywords:
    let resanitized = sanitize(&constrained);
    assert!(resanitized["properties"]["confidence"]
        .get("maximum")
        .is_none());

    println!("\nSPIKE 1 PASS: sanitizer output is grammar-legal; validator covers the stripped constraints.");
}

fn assert_no_unsupported(node: &serde_json::Value) {
    let banned = [
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "multipleOf",
        "pattern",
        "$schema",
        "title",
    ];
    if let Some(map) = node.as_object() {
        for key in banned {
            assert!(
                !map.contains_key(key),
                "unsupported keyword {key} survived sanitization"
            );
        }
        for value in map.values() {
            assert_no_unsupported(value);
        }
    } else if let Some(items) = node.as_array() {
        items.iter().for_each(assert_no_unsupported);
    }
}

fn assert_objects_closed(node: &serde_json::Value) {
    if let Some(map) = node.as_object() {
        if map.get("type").and_then(serde_json::Value::as_str) == Some("object") {
            assert_eq!(
                map.get("additionalProperties"),
                Some(&json!(false)),
                "open object survived"
            );
            let n_props = map
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .map_or(0, serde_json::Map::len);
            let n_req = map
                .get("required")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            assert_eq!(n_props, n_req, "required does not list every property");
        }
        for value in map.values() {
            assert_objects_closed(value);
        }
    } else if let Some(items) = node.as_array() {
        items.iter().for_each(assert_objects_closed);
    }
}
