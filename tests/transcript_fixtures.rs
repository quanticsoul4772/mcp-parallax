//! 066 T002: the committed transcript fixtures are scrubbed and carry sentinels.
//!
//! No fixture holds a path under a user folder, a session-id-shaped string, or
//! prompt text without a sentinel; every user prompt (and compaction summary)
//! carries the prompt sentinel and every tool result the tool sentinel, so the
//! nothing-sent tests can look for them in request bodies.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::Value;
use std::path::Path;

const PROMPT_SENTINEL: &str = "SENTINEL-USER-PROMPT-7f3a";
const TOOL_SENTINEL: &str = "SENTINEL-TOOL-RESULT-91c2";

fn fixtures() -> Vec<(String, String)> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&folder)
        .expect("fixture folder")
        .map(|e| {
            let path = e.unwrap().path();
            (
                path.display().to_string(),
                std::fs::read_to_string(&path).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn no_transcript_fixture_holds_a_user_folder_path_or_a_session_id() {
    let all = fixtures();
    assert!(!all.is_empty(), "no fixtures found");
    for (name, text) in &all {
        assert_eq!(scrub_violations(text), Vec::<String>::new(), "{name}");
    }
}

#[test]
fn the_scrub_check_catches_each_kind_it_claims_to() {
    assert_eq!(
        scrub_violations(r#"{"cwd":"C:\\Users\\someone\\x"}"#).len(),
        1
    );
    assert_eq!(scrub_violations(r#"{"cwd":"/Users/someone/x"}"#).len(), 1);
    assert_eq!(scrub_violations(r#"{"cwd":"/home/someone/x"}"#).len(), 1);
    assert_eq!(
        scrub_violations(r#"{"sessionId":"0f8fad5b-d9cb-469f-a165-70867728950e"}"#).len(),
        1
    );
    assert!(scrub_violations(r#"{"sessionId":"fixture-session"}"#).is_empty());
}

#[test]
fn every_prompt_and_tool_result_in_the_fixtures_carries_its_sentinel() {
    let (mut prompts, mut results) = (0, 0);
    for (name, text) in fixtures() {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            for (kind, body) in sentinel_targets(&v) {
                let sentinel = match kind {
                    Target::Prompt => {
                        prompts += 1;
                        PROMPT_SENTINEL
                    }
                    Target::ToolResult => {
                        results += 1;
                        TOOL_SENTINEL
                    }
                };
                assert!(
                    body.contains(sentinel),
                    "{name}: {kind:?} without sentinel: {body}"
                );
            }
        }
    }
    assert!(
        prompts > 0 && results > 0,
        "prompts {prompts}, results {results}"
    );
}

#[derive(Debug, Clone, Copy)]
enum Target {
    Prompt,
    ToolResult,
}

/// Each user-folder path or session-id-shaped string found in `text`.
fn scrub_violations(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for needle in [r"Users\\", "Users/", "/home/"] {
        if text.contains(needle) {
            found.push(format!("user-folder path ({needle})"));
        }
    }
    let bytes = text.as_bytes();
    let groups = [8, 4, 4, 4, 12];
    let span = groups.iter().sum::<usize>() + groups.len() - 1;
    for start in 0..bytes.len().saturating_sub(span - 1) {
        let mut at = start;
        let shaped = groups.iter().enumerate().all(|(i, &len)| {
            let hex = bytes[at..at + len].iter().all(u8::is_ascii_hexdigit);
            at += len;
            let dash = i == groups.len() - 1 || bytes[at] == b'-';
            at += 1;
            hex && dash
        });
        if shaped {
            found.push(format!("session-id-shaped string at byte {start}"));
        }
    }
    found
}

/// The text of each user prompt, compaction summary and tool result on a line.
fn sentinel_targets(v: &Value) -> Vec<(Target, String)> {
    if v["type"] != "user" {
        return Vec::new();
    }
    let content = &v["message"]["content"];
    let human = v["origin"]["kind"] == "human" || v["isCompactSummary"] == true;
    let mut out = Vec::new();
    match content {
        Value::String(s) if human => out.push((Target::Prompt, s.clone())),
        Value::Array(blocks) => {
            let texts: Vec<&str> = blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect();
            if human && !texts.is_empty() {
                out.push((Target::Prompt, texts.join("\n")));
            }
            for block in blocks.iter().filter(|b| b["type"] == "tool_result") {
                out.push((Target::ToolResult, block["content"].to_string()));
            }
        }
        _ => {}
    }
    out
}
