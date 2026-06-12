//! Transform a `schemars`-derived JSON Schema into the subset Anthropic's
//! structured-output grammar accepts.
//!
//! The grammar **requires** `additionalProperties: false` and a complete
//! `required` list on every object, and **rejects or ignores** numeric/length
//! constraints, recursion, and draft metadata. The constraints stripped here
//! are exactly what [`crate::schema::validate`] re-checks on the returned
//! value — strip-and-revalidate is the contract, not a loss.

use serde_json::{Map, Value};

/// Keywords the Anthropic grammar does not support; stripped from every level.
/// They remain in the unsanitized schema, where the local validator enforces
/// them on returned values.
const UNSUPPORTED_KEYWORDS: &[&str] = &[
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "pattern",
    "maxItems",
    "minProperties",
    "maxProperties",
];

/// Top-level-only metadata the grammar has no use for.
const METADATA_KEYWORDS: &[&str] = &["$schema", "title"];

/// Produce the Anthropic-grammar-subset form of `schema`.
///
/// Guarantees on the output:
/// - every `type: object` level has `additionalProperties: false`
/// - every `type: object` level has `required` listing **all** of its
///   `properties` keys (the grammar treats every declared property as
///   mandatory)
/// - no unsupported constraint keywords remain at any level
/// - `minItems` survives only as `0` or `1` (the grammar's limit)
#[must_use]
pub fn sanitize(schema: &Value) -> Value {
    let mut out = schema.clone();
    if let Value::Object(map) = &mut out {
        for key in METADATA_KEYWORDS {
            map.remove(*key);
        }
    }
    sanitize_node(&mut out);
    out
}

fn sanitize_node(node: &mut Value) {
    let Value::Object(map) = node else { return };

    for key in UNSUPPORTED_KEYWORDS {
        map.remove(*key);
    }

    // The grammar supports minItems only as 0 or 1.
    if let Some(min_items) = map.get("minItems").and_then(Value::as_u64) {
        if min_items > 1 {
            map.remove("minItems");
        }
    }

    if map.get("type").and_then(Value::as_str) == Some("object") {
        map.insert("additionalProperties".into(), Value::Bool(false));
        let property_names: Vec<Value> = map
            .get("properties")
            .and_then(Value::as_object)
            .map(|props| props.keys().cloned().map(Value::String).collect())
            .unwrap_or_default();
        map.insert("required".into(), Value::Array(property_names));
    }

    // Recurse into every schema-bearing position.
    recurse_map_values(map, "properties");
    recurse_map_values(map, "$defs");
    recurse_map_values(map, "definitions");
    for key in ["items", "additionalItems", "contains"] {
        if let Some(child) = map.get_mut(key) {
            sanitize_node(child);
        }
    }
    for key in ["anyOf", "allOf", "oneOf", "prefixItems"] {
        if let Some(Value::Array(children)) = map.get_mut(key) {
            for child in children {
                sanitize_node(child);
            }
        }
    }
}

/// Recurse into each value of an object-valued keyword (`properties`, `$defs`).
fn recurse_map_values(map: &mut Map<String, Value>, key: &str) {
    if let Some(Value::Object(children)) = map.get_mut(key) {
        for child in children.values_mut() {
            sanitize_node(child);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assert_grammar_legal(node: &Value) {
        let Value::Object(map) = node else { return };
        for key in UNSUPPORTED_KEYWORDS {
            assert!(!map.contains_key(*key), "unsupported keyword {key} remains");
        }
        if map.get("type").and_then(Value::as_str) == Some("object") {
            assert_eq!(
                map.get("additionalProperties"),
                Some(&Value::Bool(false)),
                "object lacks additionalProperties:false"
            );
            let props: Vec<&String> = map
                .get("properties")
                .and_then(Value::as_object)
                .map(|p| p.keys().collect())
                .unwrap_or_default();
            let required: Vec<&str> = map
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            for p in props {
                assert!(
                    required.contains(&p.as_str()),
                    "property {p} not in required"
                );
            }
        }
        if let Some(Value::Object(children)) = map.get("properties") {
            for child in children.values() {
                assert_grammar_legal(child);
            }
        }
        for key in ["items", "anyOf", "allOf", "$defs"] {
            match map.get(key) {
                Some(Value::Array(children)) => children.iter().for_each(assert_grammar_legal),
                Some(child @ Value::Object(_)) => assert_grammar_legal(child),
                _ => {}
            }
        }
    }

    #[test]
    fn strips_metadata_and_constraints_and_closes_objects() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Verdict",
            "type": "object",
            "properties": {
                "verdict": { "type": "string", "enum": ["supported", "refuted"] },
                "findings": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1 },
                    "maxItems": 50
                },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
            },
            "required": ["verdict"]
        });

        let out = sanitize(&schema);
        assert_grammar_legal(&out);

        // Stripped everywhere, including nested levels.
        assert!(out.get("$schema").is_none());
        assert!(out.get("title").is_none());
        assert!(out["properties"]["confidence"].get("minimum").is_none());
        assert!(out["properties"]["findings"]["items"]
            .get("minLength")
            .is_none());
        assert!(out["properties"]["findings"].get("maxItems").is_none());

        // Closed and fully required.
        assert_eq!(out["additionalProperties"], json!(false));
        let required: Vec<&str> = out["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"verdict"));
        assert!(required.contains(&"findings"));
        assert!(required.contains(&"confidence"));

        // Supported keywords survive.
        assert_eq!(
            out["properties"]["verdict"]["enum"],
            json!(["supported", "refuted"])
        );
    }

    #[test]
    fn min_items_survives_only_as_zero_or_one() {
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                "b": { "type": "array", "items": { "type": "string" }, "minItems": 4 }
            }
        });
        let out = sanitize(&schema);
        assert_eq!(out["properties"]["a"]["minItems"], json!(1));
        assert!(out["properties"]["b"].get("minItems").is_none());
    }

    #[test]
    fn nested_objects_in_defs_and_unions_are_sanitized() {
        let schema = json!({
            "type": "object",
            "properties": {
                "x": { "anyOf": [
                    { "type": "object", "properties": { "n": { "type": "integer", "minimum": 0 } } },
                    { "type": "null" }
                ]}
            },
            "$defs": {
                "Inner": { "type": "object", "properties": { "s": { "type": "string", "maxLength": 9 } } }
            }
        });
        let out = sanitize(&schema);
        assert_grammar_legal(&out);
        assert!(out["properties"]["x"]["anyOf"][0]["properties"]["n"]
            .get("minimum")
            .is_none());
        assert_eq!(out["$defs"]["Inner"]["additionalProperties"], json!(false));
        assert!(out["$defs"]["Inner"]["properties"]["s"]
            .get("maxLength")
            .is_none());
    }

    #[test]
    fn does_not_mutate_the_input() {
        let schema =
            json!({ "type": "object", "properties": { "n": { "type": "number", "minimum": 0 } } });
        let _ = sanitize(&schema);
        assert_eq!(schema["properties"]["n"]["minimum"], json!(0));
    }
}
