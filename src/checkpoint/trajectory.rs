//! The bounded trajectory window detectors consume (data-model.md §2).
//!
//! Harness transcript JSONL is parsed into normalized entries here — the
//! detectors never see the raw file. Normalization is the precision lever
//! (D5): exact match after normalization, no fuzzy similarity anywhere.

use serde_json::Value;

/// A bounded, oldest-to-newest slice of one session's trajectory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrajectoryWindow {
    /// The session this window belongs to.
    pub session_id: String,
    /// Entries, oldest → newest, capped at [`crate::checkpoint::WINDOW_ENTRIES`].
    pub entries: Vec<TrajectoryEntry>,
}

/// One normalized trajectory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrajectoryEntry {
    /// One tool invocation (with its eventual outcome).
    ToolCall {
        /// Which tool batch (assistant inference step) issued it.
        batch_index: u32,
        /// The tool's name as the harness reports it.
        tool_name: String,
        /// The invocation input, normalized (see [`normalize_input`]).
        normalized_input: String,
        /// Whether the tool result reported an error.
        failed: bool,
    },
    /// One assistant message text block.
    Assistant {
        /// The message text.
        text: String,
    },
}

/// Normalize a tool input for exact-match comparison.
///
/// Serializes compactly with volatile fields dropped and whitespace runs
/// collapsed. Volatile fields (ids, timestamps, absolute temp paths) would
/// make every invocation unique and blind the repetition detector.
#[must_use]
pub fn normalize_input(input: &Value) -> String {
    let mut out = String::new();
    write_normalized(input, &mut out);
    collapse_whitespace(&out)
}

/// Keys dropped during normalization — volatile per-invocation noise.
const VOLATILE_KEYS: &[&str] = &[
    "id",
    "tool_use_id",
    "session_id",
    "sessionId",
    "timestamp",
    "request_id",
];

fn write_normalized(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            out.push('{');
            // BTreeMap-style ordering: serde_json with default features keeps
            // insertion order, so sort keys for a stable normalized form.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                if VOLATILE_KEYS.contains(&key.as_str()) {
                    continue;
                }
                out.push_str(key);
                out.push('=');
                if let Some(inner) = map.get(key) {
                    write_normalized(inner, out);
                }
                out.push(';');
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for item in items {
                write_normalized(item, out);
                out.push(',');
            }
            out.push(']');
        }
        Value::String(s) => out.push_str(s),
        other => out.push_str(&other.to_string()),
    }
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out.trim().to_string()
}

/// Parse harness transcript JSONL lines (oldest → newest) into entries.
///
/// Unknown or malformed lines are skipped — the transcript format is not
/// ours and may grow fields; only the shapes we consume are load-bearing.
/// Returns the entries plus the session id found in the lines (`None` when
/// no line carried one).
#[must_use]
pub fn parse_lines(lines: &[&str]) -> (Vec<TrajectoryEntry>, Option<String>) {
    let mut entries = Vec::new();
    let mut session_id: Option<String> = None;
    let mut batch_index: u32 = 0;
    // tool_use id → index into `entries`, for marking failures from results.
    let mut pending: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if session_id.is_none() {
            if let Some(sid) = value.get("sessionId").and_then(Value::as_str) {
                session_id = Some(sid.to_string());
            }
        }
        let Some(content) = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let is_assistant = value.get("type").and_then(Value::as_str) == Some("assistant");
        let mut batch_used = false;
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") if is_assistant => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            entries.push(TrajectoryEntry::Assistant {
                                text: text.to_string(),
                            });
                        }
                    }
                }
                Some("tool_use") if is_assistant => {
                    if !batch_used {
                        batch_index += 1;
                        batch_used = true;
                    }
                    let tool_name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("(unknown)")
                        .to_lowercase();
                    let normalized_input =
                        item.get("input").map_or_else(String::new, normalize_input);
                    entries.push(TrajectoryEntry::ToolCall {
                        batch_index,
                        tool_name,
                        normalized_input,
                        failed: false,
                    });
                    if let Some(id) = item.get("id").and_then(Value::as_str) {
                        pending.insert(id.to_string(), entries.len() - 1);
                    }
                }
                Some("tool_result") => {
                    let is_error = item
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if is_error {
                        if let Some(index) = item
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .and_then(|id| pending.get(id).copied())
                        {
                            if let Some(TrajectoryEntry::ToolCall { failed, .. }) =
                                entries.get_mut(index)
                            {
                                *failed = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    (entries, session_id)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    // D5: the normalization ground-truth table — equal and unequal pairs.
    #[test]
    fn normalization_ground_truth() {
        let equal_pairs = [
            // Volatile fields dropped.
            (
                json!({ "command": "cargo test", "id": "a1" }),
                json!({ "command": "cargo test", "id": "zz9" }),
            ),
            // Key order irrelevant.
            (json!({ "a": 1, "b": 2 }), json!({ "b": 2, "a": 1 })),
            // Whitespace runs collapse.
            (
                json!({ "command": "cargo   test\n--all" }),
                json!({ "command": "cargo test --all" }),
            ),
        ];
        for (a, b) in equal_pairs {
            assert_eq!(normalize_input(&a), normalize_input(&b), "{a} vs {b}");
        }

        let unequal_pairs = [
            // Different commands stay different.
            (
                json!({ "command": "cargo test" }),
                json!({ "command": "cargo build" }),
            ),
            // A genuinely different field value matters.
            (
                json!({ "file_path": "a.rs", "content": "x" }),
                json!({ "file_path": "b.rs", "content": "x" }),
            ),
        ];
        for (a, b) in unequal_pairs {
            assert_ne!(normalize_input(&a), normalize_input(&b), "{a} vs {b}");
        }
    }

    fn assistant_line(session: &str, items: &Value) -> String {
        json!({
            "type": "assistant",
            "sessionId": session,
            "message": { "role": "assistant", "content": items }
        })
        .to_string()
    }

    fn result_line(tool_use_id: &str, is_error: bool) -> String {
        json!({
            "type": "user",
            "sessionId": "s1",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": tool_use_id, "is_error": is_error }
            ]}
        })
        .to_string()
    }

    #[test]
    fn parses_tool_calls_batches_failures_and_assistant_text() {
        let lines = [
            assistant_line(
                "s1",
                &json!([
                    { "type": "text", "text": "Running the tests." },
                    { "type": "tool_use", "id": "t1", "name": "Bash",
                      "input": { "command": "cargo test" } },
                    { "type": "tool_use", "id": "t2", "name": "Read",
                      "input": { "file_path": "a.rs" } }
                ]),
            ),
            result_line("t1", true),
            result_line("t2", false),
            assistant_line(
                "s1",
                &json!([
                    { "type": "tool_use", "id": "t3", "name": "Bash",
                      "input": { "command": "cargo test" } }
                ]),
            ),
            "not json at all".to_string(),
        ];
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let (entries, session) = parse_lines(&refs);

        assert_eq!(session.as_deref(), Some("s1"));
        assert_eq!(entries.len(), 4); // 1 text + 3 tool calls
        let TrajectoryEntry::Assistant { text } = &entries[0] else {
            panic!("expected assistant text first");
        };
        assert_eq!(text, "Running the tests.");
        let TrajectoryEntry::ToolCall {
            batch_index,
            tool_name,
            failed,
            ..
        } = &entries[1]
        else {
            panic!("expected a tool call");
        };
        assert_eq!(
            (*batch_index, tool_name.as_str(), *failed),
            (1, "bash", true)
        );
        // Same batch for the parallel call; next assistant message is batch 2.
        let TrajectoryEntry::ToolCall { batch_index, .. } = &entries[2] else {
            panic!()
        };
        assert_eq!(*batch_index, 1);
        let TrajectoryEntry::ToolCall {
            batch_index,
            normalized_input,
            ..
        } = &entries[3]
        else {
            panic!()
        };
        assert_eq!(*batch_index, 2);
        // Identical command normalizes identically across batches.
        let TrajectoryEntry::ToolCall {
            normalized_input: first,
            ..
        } = &entries[1]
        else {
            panic!()
        };
        assert_eq!(normalized_input, first);
    }

    #[test]
    fn empty_and_malformed_input_yields_an_empty_window() {
        let (entries, session) = parse_lines(&["", "{}", "[1,2]", "nope"]);
        assert!(entries.is_empty());
        assert!(session.is_none());
    }
}
