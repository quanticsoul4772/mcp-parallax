//! The schema pipeline — the concrete form of the constrained-output contract.
//!
//! One `schemars`-derived schema per mode output type feeds **both hops**: the
//! rmcp tool `outputSchema` (MCP client ← server) and the Anthropic
//! `output_config.format.schema` (server → model). Between "derive" and "send
//! to Anthropic" sits the [`sanitize`] transform (the API accepts only a
//! grammar subset), and on the way back the [`validate`] check re-imposes
//! exactly the constraints the sanitizer stripped.
//!
//! **API grammar guarantees shape; the local validator guarantees the value
//! constraints the grammar can't.** Neither is redundant.

pub mod sanitize;
pub mod validate;

#[cfg(test)]
use serde_json::Value;

pub use sanitize::sanitize;
pub use validate::validate;

/// Assert that a hand-written contract file and the derived schema agree on
/// **constraints**, not just on property names.
///
/// The contract tests originally compared the property-name set and the
/// `required` list. That let two real defects ship: `effort` went out as an
/// untyped string while the contract claimed an enum, and `passes` published
/// `minimum: 0` from its Rust `u8` while the server rejects `0` and the
/// contract said `minimum: 1`. A caller reading the schema was told values
/// were valid that the server refuses — and the test that exists to catch
/// exactly that drift was blind to it.
///
/// Compares only the keys the contract actually states, so a derived schema
/// may carry extra detail (`format`, `default`) without failing. What it
/// cannot do is *contradict* the contract.
///
/// Test-only: it asserts, and Principle III keeps `panic!` out of production
/// paths. Gating it with `cfg(test)` is also the truthful statement of what it
/// is — nothing on a request path calls it.
///
/// # Panics
///
/// Panics naming the property and the key when the two disagree; a mismatch is
/// a defect in the published surface.
#[cfg(test)]
#[allow(clippy::panic)]
pub fn assert_constraints_agree(contract: &Value, derived: &Value, what: &str) {
    let Some(props) = contract.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (name, spec) in props {
        let Some(spec) = spec.as_object() else {
            continue;
        };
        let Some(actual) = derived
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|p| p.get(name))
        else {
            panic!(
                "{what}: contract declares `{name}` but the derived schema has no such property"
            );
        };
        let rendered = render_with_defs(actual, derived);
        for key in ["minimum", "maximum", "enum"] {
            let Some(want) = spec.get(key) else { continue };
            let ok = if key == "enum" {
                want.as_array().is_some_and(|vs| {
                    vs.iter()
                        .filter_map(Value::as_str)
                        .all(|v| rendered.contains(v))
                })
            } else {
                rendered.contains(&format!("\"{key}\":{want}"))
            };
            assert!(
                ok,
                "{what}: property `{name}` declares {key}={want}, but the derived schema is {rendered}"
            );
        }
    }
}

/// A property plus any `$defs` it references, rendered as one string.
///
/// Test-only, like its one caller.
///
/// A property may state its constraints inline or point at `#/$defs/X`.
/// Comparing the property alone would read a `$ref` as a missing enum, so the
/// referenced definitions are appended — a `$ref` then neither hides a real
/// mismatch nor manufactures a false one.
#[cfg(test)]
fn render_with_defs(property: &Value, root: &Value) -> String {
    const MARKER: &str = "#/$defs/";
    let mut rendered = property.to_string();
    let refs: Vec<String> = rendered
        .match_indices(MARKER)
        .map(|(at, _)| {
            rendered[at + MARKER.len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect()
        })
        .collect();
    for name in refs {
        if let Some(def) = root
            .get("$defs")
            .and_then(Value::as_object)
            .and_then(|d| d.get(&name))
        {
            rendered.push_str(&def.to_string());
        }
    }
    rendered
}
