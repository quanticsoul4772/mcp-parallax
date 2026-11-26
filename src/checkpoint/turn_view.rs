//! Turn data for the 066 turn signals (FR-002, research D2/D3).
//!
//! Reads the raw tail lines the trajectory reader returned, at turn end only,
//! and finds the turn's real user prompt and the tool activity after it. The
//! result stays on the machine: it is never sent or stored.

use crate::checkpoint::review::REVIEW_FLAG_PREFIX;
use crate::checkpoint::run::{BATCH_FLAG_PREFIX, HOLD_PREFIX};
use crate::memory::push::SURFACE_PREFIX;
use serde_json::Value;

/// Every exported flag prefix (FR-014). A line that contains one anywhere is
/// never a turn start, so Parallax's own flags cannot start a turn.
pub const FLAG_PREFIXES: [&str; 4] = [
    REVIEW_FLAG_PREFIX,
    BATCH_FLAG_PREFIX,
    HOLD_PREFIX,
    SURFACE_PREFIX,
];

/// What [`parse_turn`] finds for the turn that just ended. Raw; never sent
/// or stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnData {
    /// Index in the tail lines of the turn's real user prompt; `None` when
    /// it is not in the lines read.
    pub start: Option<usize>,
    /// The prompt's text (string content, or its text blocks joined).
    pub user_prompt: String,
    /// Each tool input after the prompt, as JSON text.
    pub tool_inputs: Vec<String>,
    /// Each tool result's text after the prompt.
    pub tool_outputs: Vec<String>,
    /// Tool calls after the prompt (for the pushback structure check).
    pub tool_calls_after_prompt: u32,
}

/// Find the turn's real user prompt in `lines` and the tool activity after it.
///
/// The turn start is the last line that is a `user` line with
/// `origin.kind: "human"`, is not meta, a compaction summary or a subagent
/// line, holds no tool result, and contains no flag prefix (research D1/D2).
#[must_use]
pub fn parse_turn(lines: &[String]) -> TurnData {
    let parsed: Vec<Option<Value>> = lines
        .iter()
        .map(|line| serde_json::from_str(line).ok())
        .collect();
    let Some(start) = (0..lines.len()).rev().find(|&i| {
        parsed[i]
            .as_ref()
            .is_some_and(|v| is_turn_start(&lines[i], v))
    }) else {
        return TurnData::default();
    };
    let mut turn = TurnData {
        start: Some(start),
        user_prompt: parsed[start]
            .as_ref()
            .map(|v| text_of(&v["message"]["content"]))
            .unwrap_or_default(),
        ..TurnData::default()
    };
    for value in parsed[start + 1..].iter().flatten() {
        if value["isSidechain"] == true {
            continue;
        }
        let Some(blocks) = value["message"]["content"].as_array() else {
            continue;
        };
        for block in blocks {
            match block["type"].as_str() {
                Some("tool_use") if value["type"] == "assistant" => {
                    turn.tool_inputs.push(block["input"].to_string());
                    turn.tool_calls_after_prompt = turn.tool_calls_after_prompt.saturating_add(1);
                }
                Some("tool_result") => turn.tool_outputs.push(text_of(&block["content"])),
                _ => {}
            }
        }
    }
    turn
}

fn is_turn_start(raw: &str, value: &Value) -> bool {
    let holds_tool_result = value["message"]["content"]
        .as_array()
        .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"));
    value["type"] == "user"
        && value["origin"]["kind"] == "human"
        && value["isMeta"] != true
        && value["isCompactSummary"] != true
        && value["isSidechain"] != true
        && !holds_tool_result
        && !FLAG_PREFIXES.iter().any(|prefix| raw.contains(prefix))
}

/// String content as is; text blocks joined by newlines; anything else as JSON.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROMPT_SENTINEL: &str = "SENTINEL-USER-PROMPT-7f3a";
    const TOOL_SENTINEL: &str = "SENTINEL-TOOL-RESULT-91c2";

    fn fixture(name: &str) -> Vec<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/transcripts")
            .join(name);
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn human_prompt(text: &str) -> String {
        json!({
            "type": "user", "sessionId": "s1", "isSidechain": false,
            "origin": { "kind": "human" },
            "message": { "role": "user", "content": text }
        })
        .to_string()
    }

    fn assistant_tool(name: &str, input: &serde_json::Value) -> String {
        json!({
            "type": "assistant", "sessionId": "s1", "isSidechain": false,
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": "t1", "name": name, "input": input }
            ]}
        })
        .to_string()
    }

    #[test]
    fn the_string_prompt_fixture_finds_the_human_prompt_and_its_tool_activity() {
        let turn = parse_turn(&fixture("turn_string_prompt.jsonl"));
        // Line 2 is the only origin.kind human prompt; task notifications,
        // meta, interruption, shell, slash-command, scheduled, subagent and
        // sdk lines after it are all skipped.
        assert_eq!(turn.start, Some(2));
        assert!(turn.user_prompt.contains(PROMPT_SENTINEL));
        assert_eq!(turn.tool_calls_after_prompt, 2);
        assert_eq!(turn.tool_inputs.len(), 2);
        assert!(turn.tool_inputs.iter().any(|i| i.contains("src/lib.rs")));
        assert!(turn.tool_inputs.iter().any(|i| i.contains("cargo test")));
        assert_eq!(turn.tool_outputs.len(), 2);
        assert!(turn.tool_outputs.iter().all(|o| o.contains(TOOL_SENTINEL)));
    }

    #[test]
    fn a_prompt_in_text_blocks_beside_an_image_is_found() {
        let turn = parse_turn(&fixture("turn_block_prompt.jsonl"));
        assert_eq!(turn.start, Some(1));
        assert!(turn.user_prompt.contains(PROMPT_SENTINEL));
        assert_eq!(turn.tool_calls_after_prompt, 0);
    }

    #[test]
    fn a_slash_command_typed_by_the_user_starts_the_turn() {
        let turn = parse_turn(&fixture("turn_slash_command.jsonl"));
        assert_eq!(turn.start, Some(1));
        assert!(turn.user_prompt.contains(PROMPT_SENTINEL));
    }

    #[test]
    fn a_compaction_summary_is_not_a_turn_start() {
        let turn = parse_turn(&fixture("turn_compacted.jsonl"));
        assert_eq!(turn.start, None);
        assert!(turn.user_prompt.is_empty());
    }

    #[test]
    fn no_lines_means_no_turn_start() {
        assert_eq!(parse_turn(&[]).start, None);
    }

    #[test]
    fn tool_activity_before_the_prompt_is_not_part_of_the_turn() {
        let lines = vec![
            assistant_tool("Read", &json!({ "file_path": "before.rs" })),
            human_prompt("the prompt"),
            assistant_tool("Read", &json!({ "file_path": "after.rs" })),
        ];
        let turn = parse_turn(&lines);
        assert_eq!(turn.start, Some(1));
        assert_eq!(turn.tool_calls_after_prompt, 1);
        assert_eq!(turn.tool_inputs.len(), 1);
        assert!(turn.tool_inputs[0].contains("after.rs"));
    }

    #[test]
    fn a_human_line_carrying_a_flag_prefix_anywhere_is_skipped() {
        let flagged = format!(
            "quoting it back: {}something",
            crate::checkpoint::run::BATCH_FLAG_PREFIX
        );
        let lines = vec![human_prompt("the real prompt"), human_prompt(&flagged)];
        let turn = parse_turn(&lines);
        assert_eq!(turn.start, Some(0));
        assert_eq!(turn.user_prompt, "the real prompt");
    }

    // SC-006: every flag the templates build is skipped by the turn-start
    // finder, checked by building each flag through its template.
    #[test]
    fn every_flag_built_through_its_template_is_skipped_by_the_turn_start_finder() {
        use crate::checkpoint::preference::{violation_signal, PreferenceCandidate};
        use crate::checkpoint::{Signal, SignalKind};
        use crate::memory::push::{advisory_context, SurfacedMemory};
        use crate::memory::{Kind, Trust};

        let review_out: crate::checkpoint::review::ReviewOut = serde_json::from_value(json!({
            "contradicts": true, "statement_a": "a", "statement_b": "b", "basis": "x.",
            "violates": false, "violated_preference": "", "violation_basis": "",
            "capture_worthy": false, "capture_kind": "none",
            "capture_content": "", "capture_basis": ""
        }))
        .unwrap();
        let candidate = PreferenceCandidate {
            memory_id: "mem-1".into(),
            content: "c".into(),
            trust: Trust::FirstHand,
            score: 0.8,
        };
        let surfaced = vec![SurfacedMemory {
            id: "mem-1".into(),
            kind: Kind::Lesson,
            trust: Trust::FirstHand,
            score: 0.9,
            content: "c".into(),
        }];
        let flags = [
            crate::checkpoint::review::assemble_flag(&review_out),
            violation_signal(&candidate, "b.").1,
            crate::checkpoint::run::batch_flag_message(&[Signal::new(
                SignalKind::Repetition,
                "e".into(),
                "k",
            )]),
            crate::checkpoint::run::hold_message("c"),
            advisory_context(&surfaced),
        ];
        for flag in flags {
            let lines = vec![human_prompt("the real prompt"), human_prompt(&flag)];
            assert_eq!(parse_turn(&lines).start, Some(0), "not skipped: {flag}");
        }
    }
}
