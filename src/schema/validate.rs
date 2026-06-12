//! Defense-in-depth validation against the **unsanitized** schema.
//!
//! The Anthropic grammar guarantees shape but drops value constraints
//! (numeric ranges, string/array lengths). This validator re-checks returned
//! values against the original schema — covering exactly the API's blind spot.

use crate::error::AppError;
use serde_json::Value;

/// Validate `value` against the (unsanitized) JSON Schema `schema`.
///
/// # Errors
///
/// Returns [`AppError::ValidationFailure`] listing every violation, or
/// [`AppError::ValidationFailure`] describing the schema itself if it does not
/// compile (a programming error surfaced loudly rather than skipped).
pub fn validate(schema: &Value, value: &Value) -> Result<(), AppError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| AppError::ValidationFailure(format!("schema does not compile: {e}")))?;

    let violations: Vec<String> = validator
        .iter_errors(value)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();

    if violations.is_empty() {
        Ok(())
    } else {
        Err(AppError::ValidationFailure(violations.join("; ")))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn verdict_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "verdict": { "type": "string", "enum": ["supported", "refuted"] },
                "findings": { "type": "array", "items": { "type": "string" } },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
            },
            "required": ["verdict", "findings", "confidence"],
            "additionalProperties": false
        })
    }

    #[test]
    fn accepts_a_conforming_value() {
        let value = json!({ "verdict": "refuted", "findings": ["wrong year"], "confidence": 1.0 });
        validate(&verdict_schema(), &value).unwrap();
    }

    #[test]
    fn reimposes_the_range_constraint_the_sanitizer_strips() {
        // After sanitize(), the grammar would happily emit confidence: 7.
        let sanitized = crate::schema::sanitize(&verdict_schema());
        assert!(sanitized["properties"]["confidence"]
            .get("maximum")
            .is_none());

        let out_of_range = json!({ "verdict": "supported", "findings": [], "confidence": 7.0 });
        let err = validate(&verdict_schema(), &out_of_range).unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)));
        assert!(err.to_string().contains("confidence"));
    }

    #[test]
    fn rejects_undeclared_fields_and_wrong_enum() {
        let bad = json!({ "verdict": "maybe", "findings": [], "confidence": 0.5, "extra": 1 });
        let err = validate(&verdict_schema(), &bad).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("maybe") || msg.contains("enum"));
        assert!(msg.contains("extra") || msg.contains("additional"));
    }

    #[test]
    fn a_noncompiling_schema_is_a_loud_error() {
        let broken = json!({ "type": 12 });
        let err = validate(&broken, &json!({})).unwrap_err();
        assert!(err.to_string().contains("schema does not compile"));
    }
}
