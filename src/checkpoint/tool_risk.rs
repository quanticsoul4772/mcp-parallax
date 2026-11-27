//! The tool-risk pre-filter (067, research D2): which matched calls are
//! settled in code and which go to Jev.
//!
//! Order: risk patterns first, then an allowlist of whole commands, then
//! Jev for the remainder (§6.3.3).

use crate::checkpoint::gate;

/// Why a matched call is settled in code, with no Jev request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// It matched the gate's risk patterns.
    RiskPattern,
    /// It is an allowlisted whole command.
    Allowlist,
}

impl Exclusion {
    /// The reason as recorded in `error_class`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RiskPattern => "risk_pattern",
            Self::Allowlist => "allowlist",
        }
    }
}

/// What the pre-filter decides for one matched call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefilter {
    /// Settled in code; an `excluded` row, no Jev request.
    Excluded(Exclusion),
    /// Sent to Jev.
    Classify,
}

/// Allowlisted command words, each followed by any arguments (research D2).
const ALLOWLIST: [&str; 2] = ["cargo test", "cargo build"];

/// Operators that make a command more than one whole command (Clarifications R);
/// a newline or carriage return starts another command in bash.
const CHAINING: [&str; 13] = [
    "&&", ";", "|", "$(", "`", "&", ">", ">>", "<", "<(", ">(", "\n", "\r",
];

/// Decide one matched call: risk pattern, allowlist, or Jev.
#[must_use]
pub fn prefilter(tool_name: &str, tool_input: &str, extra_patterns: &[String]) -> Prefilter {
    if gate::risk_matched(tool_name, tool_input, extra_patterns) {
        return Prefilter::Excluded(Exclusion::RiskPattern);
    }
    if tool_name == "Bash" && bash_command(tool_input).is_some_and(|c| allowlisted(&c)) {
        return Prefilter::Excluded(Exclusion::Allowlist);
    }
    Prefilter::Classify
}

fn bash_command(tool_input: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(tool_input).ok()?;
    value.get("command")?.as_str().map(str::to_string)
}

fn allowlisted(command: &str) -> bool {
    if CHAINING.iter().any(|op| command.contains(op)) {
        return false;
    }
    let command = command.trim();
    ALLOWLIST.iter().any(|allowed| {
        command
            .strip_prefix(allowed)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    })
}

/// Tools whose allowlisted argument values are sent (research D3).
const VALUE_TOOLS: [&str; 3] = ["Read", "Grep", "Glob"];

/// The argument fields of [`VALUE_TOOLS`] whose values are sent.
const VALUE_FIELDS: [&str; 3] = ["file_path", "path", "pattern"];

/// The projection of one pending call, before redaction (research D3).
///
/// `Bash` sends its command with heredoc bodies dropped; `Write` and `Edit`
/// their `file_path`; every other tool its name and argument names, with
/// values only for the allowlisted fields of `Read`, `Grep` and `Glob`.
#[must_use]
pub fn projection(tool_name: &str, tool_input: &str) -> serde_json::Value {
    use serde_json::{json, Value};
    let input: Value = serde_json::from_str(tool_input).unwrap_or(Value::Null);
    let field = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match tool_name {
        "Bash" => json!({ "tool": tool_name, "command": drop_heredoc_bodies(&field("command")) }),
        "Write" | "Edit" => json!({ "tool": tool_name, "file_path": field("file_path") }),
        _ => {
            let object = input.as_object();
            let mut names: Vec<&str> = object
                .map(|o| o.keys().map(String::as_str).collect())
                .unwrap_or_default();
            names.sort_unstable();
            let mut values = serde_json::Map::new();
            if let (true, Some(o)) = (VALUE_TOOLS.contains(&tool_name), object) {
                for key in VALUE_FIELDS {
                    if let Some(value) = o.get(key) {
                        values.insert(key.to_string(), value.clone());
                    }
                }
            }
            json!({ "tool": tool_name, "argument_names": names, "values": values })
        }
    }
}

/// The projection through the mandatory redactor; nothing else is sent.
///
/// # Errors
///
/// A redaction failure, in which case nothing is sent.
pub fn redacted_projection(
    redactor: &crate::jev::redact::Redactor,
    tool_name: &str,
    tool_input: &str,
) -> Result<crate::jev::redact::RedactedState, crate::jev::redact::RedactError> {
    redactor.redact_json(&projection(tool_name, tool_input))
}

/// Drop every heredoc body, keeping the line that opens it and its closing
/// delimiter.
fn drop_heredoc_bodies(command: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut pending: std::collections::VecDeque<(String, bool)> = std::collections::VecDeque::new();
    for line in command.split('\n') {
        if let Some((delimiter, strip_tabs)) = pending.front() {
            let candidate = if *strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if candidate == delimiter {
                kept.push(line);
                pending.pop_front();
            }
            continue;
        }
        kept.push(line);
        pending.extend(heredoc_delimiters(line));
    }
    kept.join("\n")
}

/// The heredoc delimiters a line opens, in order, each with whether `<<-`
/// strips leading tabs. A here-string (`<<<`) opens none.
fn heredoc_delimiters(line: &str) -> Vec<(String, bool)> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("<<") {
        let after = &rest[at + 2..];
        if after.starts_with('<') {
            rest = after.trim_start_matches('<');
            continue;
        }
        let (strip_tabs, after) = after
            .strip_prefix('-')
            .map_or((false, after), |a| (true, a));
        let after = after.trim_start_matches([' ', '\t']);
        let (delimiter, quoted, end) = delimiter_word(after);
        // `<<''` is a heredoc ending at the first empty line; a bare `<<`
        // with no word is not a heredoc.
        if !delimiter.is_empty() || quoted {
            found.push((delimiter, strip_tabs));
        }
        rest = &after[end..];
    }
    found
}

/// The heredoc delimiter word at the start of `text`, with bash's quoting
/// removed (`'EOF'`, `"EOF"`, `\EOF`, `E"O"F` all give `EOF`), whether any
/// part of it was quoted, and the byte offset where the word ends.
fn delimiter_word(text: &str) -> (String, bool, usize) {
    let mut word = String::new();
    let mut quoted = false;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (at, c) in text.char_indices() {
        if escaped {
            word.push(c);
            escaped = false;
            continue;
        }
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    quoted = true;
                }
                '\\' => {
                    escaped = true;
                    quoted = true;
                }
                c if c.is_whitespace() || ";|&<>()".contains(c) => return (word, quoted, at),
                c => word.push(c),
            },
        }
    }
    (word, quoted, text.len())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{prefilter, redacted_projection, Exclusion, Prefilter};
    use crate::jev::redact::{RedactError, Redactor};
    use serde_json::json;

    fn bash(command: &str) -> String {
        serde_json::json!({ "command": command }).to_string()
    }

    #[test]
    fn a_risk_pattern_match_is_excluded_first() {
        // Risk patterns win even over an allowlisted command.
        assert_eq!(
            prefilter("Bash", &bash("git push origin main"), &[]),
            Prefilter::Excluded(Exclusion::RiskPattern)
        );
        // `release` is a risk pattern, so this allowlisted command is a risk match.
        assert_eq!(
            prefilter("Bash", &bash("cargo build --release"), &[]),
            Prefilter::Excluded(Exclusion::RiskPattern)
        );
        assert_eq!(
            prefilter("Read", r#"{"file_path":"notes/deploy.md"}"#, &[]),
            Prefilter::Excluded(Exclusion::RiskPattern)
        );
        let extra = vec!["frobnicate".to_string()];
        assert_eq!(
            prefilter("Bash", &bash("cargo test frobnicate"), &extra),
            Prefilter::Excluded(Exclusion::RiskPattern)
        );
    }

    #[test]
    fn cargo_test_and_cargo_build_with_any_arguments_are_allowlisted() {
        for command in [
            "cargo test",
            "cargo build",
            "cargo test -j 2 --all-features --tests",
            "cargo build --all-targets",
            "  cargo test config  ",
        ] {
            assert_eq!(
                prefilter("Bash", &bash(command), &[]),
                Prefilter::Excluded(Exclusion::Allowlist),
                "{command}"
            );
        }
    }

    #[test]
    fn only_whole_cargo_test_or_build_commands_match() {
        for command in [
            "cargo testx",
            "cargo builder",
            "cargo clippy",
            "echo cargo test",
            "cargotest",
        ] {
            assert_eq!(
                prefilter("Bash", &bash(command), &[]),
                Prefilter::Classify,
                "{command}"
            );
        }
    }

    #[test]
    fn a_command_with_any_chaining_or_redirection_operator_never_matches() {
        for op in [
            "&&", ";", "|", "$(", "`", "&", ">", ">>", "<", "<(", ">(", "\n", "\r",
        ] {
            let command = format!("cargo test {op} echo hi");
            assert_eq!(
                prefilter("Bash", &bash(&command), &[]),
                Prefilter::Classify,
                "{command}"
            );
        }
    }

    #[test]
    fn the_allowlist_applies_to_bash_only() {
        assert_eq!(
            prefilter("Write", r#"{"file_path":"cargo test"}"#, &[]),
            Prefilter::Classify
        );
    }

    #[test]
    fn everything_else_goes_to_jev() {
        assert_eq!(prefilter("Bash", &bash("ls -la"), &[]), Prefilter::Classify);
        assert_eq!(
            prefilter("Grep", r#"{"pattern":"fn main"}"#, &[]),
            Prefilter::Classify
        );
        assert_eq!(
            prefilter("mcp__exa__exa_search", r#"{"query":"rust"}"#, &[]),
            Prefilter::Classify
        );
    }

    #[test]
    fn exclusion_reasons_have_their_record_names() {
        assert_eq!(Exclusion::RiskPattern.as_str(), "risk_pattern");
        assert_eq!(Exclusion::Allowlist.as_str(), "allowlist");
    }

    // 067 T016 (FR-006, §11 test 3): only projections leave the machine.
    // Each test asserts on the whole serialized state that is sent.

    fn sent(tool: &str, input: &str) -> String {
        redacted_projection(&Redactor::new(vec![]), tool, input)
            .unwrap()
            .as_str()
            .to_string()
    }

    #[test]
    fn bash_sends_the_command_with_heredoc_bodies_dropped() {
        let command =
            "cat > notes.txt <<'EOF'\nprivate line one\nprivate line two\nEOF\nwc -l notes.txt";
        assert_eq!(
            sent("Bash", &bash(command)),
            json!({
                "tool": "Bash",
                "command": "cat > notes.txt <<'EOF'\nEOF\nwc -l notes.txt",
            })
            .to_string()
        );
    }

    #[test]
    fn bash_heredoc_forms_are_all_dropped() {
        for (command, kept) in [
            ("cat <<EOF\nbody\nEOF", "cat <<EOF\nEOF"),
            ("cat <<\"END\"\nbody\nEND", "cat <<\"END\"\nEND"),
            ("cat <<-EOF\n\tbody\n\tEOF", "cat <<-EOF\n\tEOF"),
            // 067 review finding 5: an empty quoted delimiter ends at the
            // first empty line; backslash and partly quoted delimiters are
            // the word with the quoting removed.
            ("cat <<''\nprivate body\n\nwc -l", "cat <<''\n\nwc -l"),
            ("cat <<\\EOF\nbody\nEOF\nnext", "cat <<\\EOF\nEOF\nnext"),
            ("cat <<E\"O\"F\nbody\nEOF\nnext", "cat <<E\"O\"F\nEOF\nnext"),
        ] {
            assert_eq!(
                sent("Bash", &bash(command)),
                json!({ "tool": "Bash", "command": kept }).to_string(),
                "{command}"
            );
        }
    }

    #[test]
    fn write_and_edit_send_the_file_path_only() {
        assert_eq!(
            sent(
                "Write",
                r#"{"file_path":"src/lib.rs","content":"secret body"}"#
            ),
            json!({ "tool": "Write", "file_path": "src/lib.rs" }).to_string()
        );
        assert_eq!(
            sent(
                "Edit",
                r#"{"file_path":"a.rs","old_string":"x","new_string":"y"}"#
            ),
            json!({ "tool": "Edit", "file_path": "a.rs" }).to_string()
        );
    }

    #[test]
    fn other_tools_send_the_name_and_argument_names_without_values() {
        assert_eq!(
            sent(
                "mcp__exa__exa_search",
                r#"{"query":"private question","numResults":5}"#
            ),
            json!({
                "tool": "mcp__exa__exa_search",
                "argument_names": ["numResults", "query"],
                "values": {},
            })
            .to_string()
        );
    }

    #[test]
    fn read_grep_and_glob_send_their_allowlisted_values() {
        assert_eq!(
            sent("Read", r#"{"file_path":"src/main.rs","limit":20}"#),
            json!({
                "tool": "Read",
                "argument_names": ["file_path", "limit"],
                "values": { "file_path": "src/main.rs" },
            })
            .to_string()
        );
        assert_eq!(
            sent(
                "Grep",
                r#"{"pattern":"fn main","path":"src","glob":"*.rs"}"#
            ),
            json!({
                "tool": "Grep",
                "argument_names": ["glob", "path", "pattern"],
                "values": { "path": "src", "pattern": "fn main" },
            })
            .to_string()
        );
        assert_eq!(
            sent("Glob", r#"{"pattern":"**/*.rs"}"#),
            json!({
                "tool": "Glob",
                "argument_names": ["pattern"],
                "values": { "pattern": "**/*.rs" },
            })
            .to_string()
        );
    }

    #[test]
    fn a_value_field_of_another_tool_is_not_sent() {
        // `path` is allowlisted for Read, Grep and Glob only.
        assert_eq!(
            sent("mcp__fs__list", r#"{"path":"/home/someone/private"}"#),
            json!({
                "tool": "mcp__fs__list",
                "argument_names": ["path"],
                "values": {},
            })
            .to_string()
        );
    }

    #[test]
    fn projections_are_redacted() {
        let redactor = Redactor::new(vec!["supersecretvalue123".into()]);
        let state = redacted_projection(
            &redactor,
            "Bash",
            &bash("curl -H 'X-Key: supersecretvalue123' localhost"),
        )
        .unwrap();
        assert!(!state.as_str().contains("supersecretvalue123"));
        assert!(state.as_str().contains(crate::jev::redact::MARKER));
        assert_eq!(state.redactions(), 1);
    }

    #[test]
    fn a_redaction_failure_sends_nothing() {
        let huge = "x".repeat(crate::jev::redact::MAX_INPUT_CHARS + 1);
        assert!(matches!(
            redacted_projection(&Redactor::new(vec![]), "Bash", &bash(&huge)),
            Err(RedactError::TooLarge { .. })
        ));
    }

    #[tokio::test]
    async fn a_redaction_failure_is_recorded_with_no_request() {
        use crate::jev::shadow::{shadow_tool, ToolIds, ToolJev};
        use crate::storage::jev::JevOutcome;
        use crate::traits::classifier::{Classifier, JevRequest, JevResponse};

        /// Counts requests; a redaction failure must leave it at zero.
        struct Counting(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        #[async_trait::async_trait]
        impl Classifier for Counting {
            async fn classify(
                &self,
                _: &JevRequest,
            ) -> Result<JevResponse, crate::error::AppError> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(crate::error::AppError::ClassifierProvider(
                    "nothing is sent when redaction fails".into(),
                ))
            }
            fn model_id(&self) -> &'static str {
                "jev-1.14.0"
            }
        }
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let jev = ToolJev {
            classifier: std::sync::Arc::new(Counting(std::sync::Arc::clone(&calls))),
            redactor: Redactor::new(vec![]),
            model: "jev-1.14.0".into(),
            server_session_id: "server-1".into(),
            deadline: std::time::Duration::from_millis(crate::checkpoint::GATE_BUDGET_MS),
            label_client: std::sync::Arc::new(crate::traits::client::MockModelClient::new()),
            label_model: "claude-opus-4-8".into(),
            jev_slot: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            label_slot: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        };
        let huge = "x".repeat(crate::jev::redact::MAX_INPUT_CHARS + 1);
        let (row, to_label) = shadow_tool(
            &jev,
            "Bash",
            &bash(&huge),
            ToolIds {
                eval_id: "eval-1",
                harness_session_id: "s1",
                transcript_path: "t.jsonl",
                now: chrono::Utc::now(),
            },
        )
        .await;
        assert_eq!(
            row.outcome,
            JevOutcome::Call(crate::error::Outcome::InvalidInput)
        );
        assert_eq!(row.error_class.as_deref(), Some("redaction:too_large"));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(row.label_json.is_none());
        assert!(to_label.is_none(), "nothing to label when nothing was sent");
    }
}
