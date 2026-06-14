//! Modes are data, not bespoke machinery.
//!
//! A corrective is `{ id, description, prompt template, output schema,
//! ensemble k }`. Each mode has a thin `run` function sharing the server's
//! recorded execution path; a fully generic executor is deferred until mode #3
//! makes the pattern visible (research.md 002 D3). Adding a mode is a registry
//! entry plus its run function — not a new subsystem.

pub mod grounded_compute;
pub mod grounded_verify;
pub mod unstick;
pub mod verify;

use crate::error::AppError;
use crate::schema::sanitize;
use serde_json::Value;
use std::collections::HashMap;

/// One corrective mode — the registry entry contract for every current and
/// future mode (data-model.md §1).
#[derive(Debug, Clone)]
pub struct CorrectiveMode {
    /// Tool name as exposed over MCP.
    pub id: &'static str,
    /// The MCP tool description — this does the routing work.
    pub description: &'static str,
    /// Instruction template. Placeholders exist for the mode's declared
    /// inputs only: blindness is structural, not behavioral.
    pub prompt_template: &'static str,
    /// The unsanitized per-pass output schema (validator-side).
    pub output_schema: Value,
    /// The grammar-legal form sent to the provider, derived once at
    /// registration (stable schemas keep the provider's grammar cache warm).
    pub sanitized_schema: Value,
    /// Parallel passes per invocation.
    pub ensemble_k: u8,
}

/// The set of registered modes. Registration enforces the schema invariant at
/// boot — a mode with an illegal schema fails startup, not its first call.
#[derive(Debug, Default)]
pub struct ModeRegistry {
    modes: HashMap<&'static str, CorrectiveMode>,
}

impl ModeRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a mode, deriving its sanitized schema.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::ValidationFailure`] if the mode's output schema is
    /// not flat + closed (Constitution II / FR-006).
    pub fn register(
        &mut self,
        id: &'static str,
        description: &'static str,
        prompt_template: &'static str,
        output_schema: Value,
        ensemble_k: u8,
    ) -> Result<(), AppError> {
        assert_flat(id, &output_schema)?;
        let sanitized_schema = sanitize(&output_schema);
        let previous = self.modes.insert(
            id,
            CorrectiveMode {
                id,
                description,
                prompt_template,
                output_schema,
                sanitized_schema,
                ensemble_k,
            },
        );
        if previous.is_some() {
            return Err(AppError::ValidationFailure(format!(
                "duplicate mode id '{id}' would shadow an existing tool"
            )));
        }
        Ok(())
    }

    /// Look up a mode by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&CorrectiveMode> {
        self.modes.get(id)
    }
}

/// Enforce the flat + closed invariant: the root is an object whose properties
/// are scalars (string/number/integer/boolean, optionally enum-constrained),
/// nullable scalars (`type: [scalar, "null"]`), or arrays of scalars — one
/// level of named fields, nothing deeper, no `$ref`, no non-scalar unions.
/// Closure (`additionalProperties: false` everywhere) is then the sanitizer's
/// structural guarantee.
fn assert_flat(id: &str, schema: &Value) -> Result<(), AppError> {
    let illegal = |what: &str| {
        Err(AppError::ValidationFailure(format!(
            "mode '{id}' output schema is not flat+closed: {what}"
        )))
    };

    let Some(root) = schema.as_object() else {
        return illegal("root is not an object schema");
    };
    if root.get("type").and_then(Value::as_str) != Some("object") {
        return illegal("root type must be 'object'");
    }
    if root.contains_key("$ref") || root.contains_key("$defs") || root.contains_key("anyOf") {
        return illegal(
            "references and object/union schemas are not allowed (nullable scalars are)",
        );
    }
    let Some(properties) = root.get("properties").and_then(Value::as_object) else {
        return illegal("root has no properties");
    };

    for (name, prop) in properties {
        // An enum of scalars is a scalar. This covers both the plain `enum`
        // form and schemars' doc-commented-enum encoding (`oneOf` of scalar
        // `const`s, which the sanitizer collapses to `enum`).
        if is_scalar_enum(prop) {
            continue;
        }
        // A nullable scalar (`type: ["string","null"]` — schemars' encoding of
        // Option<T>) is still flat; the grammar's null type covers it
        // (verified live, feature 002). The gate is exactly that shape —
        // heterogeneous unions stay illegal until one is validated.
        if let Some(union) = prop.get("type").and_then(Value::as_array) {
            let is_nullable_scalar = union.len() == 2
                && union.iter().any(|t| t.as_str() == Some("null"))
                && union.iter().any(|t| {
                    matches!(
                        t.as_str(),
                        Some("string" | "number" | "integer" | "boolean")
                    )
                });
            if is_nullable_scalar {
                continue;
            }
            return illegal(&format!(
                "property '{name}' has a type union that is not a nullable scalar"
            ));
        }
        let Some(type_name) = prop.get("type").and_then(Value::as_str) else {
            return illegal(&format!("property '{name}' has no concrete type"));
        };
        match type_name {
            "string" | "number" | "integer" | "boolean" => {}
            "array" => {
                let item_type = prop
                    .get("items")
                    .and_then(|items| items.get("type"))
                    .and_then(Value::as_str);
                match item_type {
                    Some("string" | "number" | "integer" | "boolean") => {}
                    _ => return illegal(&format!("property '{name}' is not an array of scalars")),
                }
            }
            other => {
                return illegal(&format!(
                    "property '{name}' has non-scalar type '{other}' (one level only)"
                ))
            }
        }
    }
    Ok(())
}

/// True when `prop` is an enumeration of scalar values — either `enum: [..]`
/// or `oneOf: [{const: ..}, ..]` with every branch a scalar const.
fn is_scalar_enum(prop: &Value) -> bool {
    if let Some(variants) = prop.get("enum").and_then(Value::as_array) {
        return variants.iter().all(|v| !v.is_object() && !v.is_array());
    }
    if let Some(branches) = prop.get("oneOf").and_then(Value::as_array) {
        return !branches.is_empty()
            && branches.iter().all(|branch| {
                branch
                    .get("const")
                    .is_some_and(|c| !c.is_object() && !c.is_array())
            });
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn flat_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "verdict": { "type": "string", "enum": ["supported", "refuted"] },
                "findings": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["verdict", "findings"]
        })
    }

    #[test]
    fn registers_a_flat_mode_and_derives_the_sanitized_schema() {
        let mut registry = ModeRegistry::new();
        registry
            .register("verify", "desc", "template {claim}", flat_schema(), 3)
            .unwrap();

        let mode = registry.get("verify").unwrap();
        assert_eq!(mode.ensemble_k, 3);
        assert_eq!(mode.sanitized_schema["additionalProperties"], json!(false));
        // Sanitized derivation happened once, at registration.
        assert!(mode.output_schema.get("additionalProperties").is_none());
    }

    #[test]
    fn rejects_a_nested_object_schema_at_boot() {
        let nested = json!({
            "type": "object",
            "properties": {
                "inner": { "type": "object", "properties": { "x": { "type": "string" } } }
            }
        });
        let err = ModeRegistry::new()
            .register("bad", "d", "t", nested, 1)
            .unwrap_err();
        assert!(err.to_string().contains("not flat+closed"), "{err}");
        assert!(err.to_string().contains("'inner'"));
    }

    #[test]
    fn rejects_arrays_of_objects_refs_and_unions() {
        let array_of_objects = json!({
            "type": "object",
            "properties": {
                "list": { "type": "array", "items": { "type": "object" } }
            }
        });
        assert!(ModeRegistry::new()
            .register("a", "d", "t", array_of_objects, 1)
            .is_err());

        let with_ref = json!({
            "type": "object",
            "$defs": { "X": { "type": "string" } },
            "properties": { "x": { "type": "string" } }
        });
        assert!(ModeRegistry::new()
            .register("b", "d", "t", with_ref, 1)
            .is_err());

        let non_object_root = json!({ "type": "array", "items": { "type": "string" } });
        assert!(ModeRegistry::new()
            .register("c", "d", "t", non_object_root, 1)
            .is_err());
    }

    #[test]
    fn unknown_mode_is_none() {
        assert!(ModeRegistry::new().get("nope").is_none());
    }

    #[test]
    fn nullable_scalar_unions_are_flat_but_object_unions_are_not() {
        let nullable_scalar = json!({
            "type": "object",
            "properties": {
                "x": { "type": ["string", "null"] }
            }
        });
        assert!(ModeRegistry::new()
            .register("ok", "d", "t", nullable_scalar, 1)
            .is_ok());

        let object_union = json!({
            "type": "object",
            "properties": {
                "x": { "type": ["object", "null"] }
            }
        });
        assert!(ModeRegistry::new()
            .register("bad", "d", "t", object_union, 1)
            .is_err());

        // Heterogeneous scalar unions are not yet validated against the
        // grammar — the gate admits exactly nullable scalars.
        let heterogeneous = json!({
            "type": "object",
            "properties": {
                "x": { "type": ["string", "integer"] }
            }
        });
        assert!(ModeRegistry::new()
            .register("het", "d", "t", heterogeneous, 1)
            .is_err());
    }

    #[test]
    fn duplicate_mode_id_fails_boot_instead_of_shadowing() {
        let mut registry = ModeRegistry::new();
        registry
            .register("verify", "d", "t", flat_schema(), 3)
            .unwrap();
        let err = registry
            .register("verify", "other", "t2", flat_schema(), 1)
            .unwrap_err();
        assert!(err.to_string().contains("duplicate mode id"), "{err}");
    }
}
