//! How one model call becomes one `claude -p` process (060 research.md D1–D3,
//! D6; `contracts/claude-cli-call.md`). Pure: no process, no environment read.

use crate::routing::Effort;
use crate::traits::runner::CliRequest;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

/// The system prompt every call runs under, replacing Claude Code's own.
pub const SYSTEM_PROMPT: &str =
    "Answer the request using only the required structured output. Do not use tools.";

/// Variables removed from the child's environment, by name only (D3): the
/// ones that would bill an API key or a cloud account, and any inherited
/// effort level, which Parallax then sets itself (D6).
const ENV_REMOVE: [&str; 6] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_EFFORT_LEVEL",
];

/// The effort variable the CLI reads ahead of `--effort` and its settings.
const EFFORT_VAR: &str = "CLAUDE_CODE_EFFORT_LEVEL";

/// The turn cap (D2): structured output took two turns in the spike; four
/// leaves room for the CLI's own schema retry without letting a call loop.
const MAX_TURNS: &str = "4";

/// One model call, as the client knows it.
#[derive(Debug, Clone, Copy)]
pub struct CallSpec<'a> {
    /// `CLAUDE_CLI_PATH`.
    pub program: &'a str,
    /// The routed model id.
    pub model: &'a str,
    /// The routed or per-call effort; `None` sends no effort level.
    pub effort: Option<Effort>,
    /// The sanitized output schema.
    pub schema: &'a Value,
    /// The prompt; written to stdin, never an argument.
    pub prompt: &'a str,
    /// The process's empty working folder.
    pub cwd: &'a Path,
    /// `REQUEST_TIMEOUT_MS`.
    pub timeout: Duration,
}

/// Build the contract's process request for one call.
#[must_use]
pub fn build(spec: &CallSpec<'_>) -> CliRequest {
    let args = [
        "-p",
        "--output-format",
        "json",
        "--json-schema",
        &spec.schema.to_string(),
        "--model",
        spec.model,
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--tools",
        "",
        "--setting-sources",
        "project",
        // 060 review finding 1: no CLAUDE.md, auto memory, hooks, skills or
        // plugins; authentication and model selection work normally.
        "--safe-mode",
        "--no-session-persistence",
        "--max-turns",
        MAX_TURNS,
        "--system-prompt",
        SYSTEM_PROMPT,
    ]
    .iter()
    .map(|a| (*a).to_string())
    .collect();
    let level = spec.effort.map_or("unset", Effort::as_str);
    CliRequest {
        program: spec.program.to_string(),
        args,
        stdin: spec.prompt.to_string(),
        env_remove: ENV_REMOVE.iter().map(|v| (*v).to_string()).collect(),
        env_set: vec![
            (EFFORT_VAR.to_string(), level.to_string()),
            // 060 review finding 1: memory files stay off whatever the
            // operator's environment says (verified live 2026-11-22).
            (
                "CLAUDE_CODE_DISABLE_CLAUDE_MDS".to_string(),
                "1".to_string(),
            ),
            (
                "CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_string(),
                "1".to_string(),
            ),
        ],
        cwd: spec.cwd.to_path_buf(),
        timeout: spec.timeout,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::routing::Effort;
    use serde_json::json;
    use std::path::Path;
    use std::time::Duration;

    fn build_for(effort: Option<Effort>) -> crate::traits::runner::CliRequest {
        build(&CallSpec {
            program: "claude",
            model: "claude-sonnet-5-5",
            effort,
            schema: &json!({"type": "object", "properties": {"ok": {"type": "boolean"}}}),
            prompt: "Score these options: alpha, beta",
            cwd: Path::new("empty-folder"),
            timeout: Duration::from_secs(120),
        })
    }

    /// 060 T006: the arguments are exactly the contract's, in order.
    #[test]
    fn the_arguments_are_the_contracts_in_order() {
        let req = build_for(None);
        assert_eq!(req.program, "claude");
        assert_eq!(
            req.args,
            [
                "-p",
                "--output-format",
                "json",
                "--json-schema",
                r#"{"properties":{"ok":{"type":"boolean"}},"type":"object"}"#,
                "--model",
                "claude-sonnet-5-5",
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--tools",
                "",
                "--setting-sources",
                "project",
                "--safe-mode",
                "--no-session-persistence",
                "--max-turns",
                "4",
                "--system-prompt",
                SYSTEM_PROMPT,
            ]
        );
        assert_eq!(
            SYSTEM_PROMPT,
            "Answer the request using only the required structured output. Do not use tools."
        );
    }

    /// 060 T006 (D1): the prompt travels on stdin and in no argument.
    #[test]
    fn the_prompt_is_stdin_and_never_an_argument() {
        let req = build_for(None);
        assert_eq!(req.stdin, "Score these options: alpha, beta");
        assert!(
            req.args.iter().all(|a| !a.contains("alpha")),
            "{:?}",
            req.args
        );
    }

    /// 060 T006 (D3, FR-004): every variable that could bill an API key or a
    /// cloud account, and any inherited effort level, is removed by name.
    #[test]
    fn billing_and_effort_variables_are_removed_by_name() {
        let req = build_for(None);
        assert_eq!(
            req.env_remove,
            [
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
                "CLAUDE_CODE_EFFORT_LEVEL",
            ]
        );
    }

    /// 060 T006 (D6): a routed effort is the CLI's effort level; none is
    /// `unset`, which tells the CLI to send no effort level.
    #[test]
    fn the_effort_level_is_the_routed_one_or_unset() {
        let level = |req: &crate::traits::runner::CliRequest| {
            req.env_set
                .iter()
                .filter(|(name, _)| name == "CLAUDE_CODE_EFFORT_LEVEL")
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            level(&build_for(None)),
            [("CLAUDE_CODE_EFFORT_LEVEL".to_string(), "unset".to_string())]
        );
        for effort in [
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::XHigh,
            Effort::Max,
        ] {
            assert_eq!(
                level(&build_for(Some(effort))),
                [(
                    "CLAUDE_CODE_EFFORT_LEVEL".to_string(),
                    effort.as_str().to_string()
                )]
            );
        }
    }

    /// 060 review finding 1 (FR-003): no CLAUDE.md, auto memory, hooks,
    /// skills or plugins reach a call. `--safe-mode` turns customizations off
    /// while keeping the subscription login; the two variables turn memory
    /// files off whatever the operator's environment says. Verified live on
    /// 2026-11-22: a planted parent-folder CLAUDE.md steered the answer
    /// without these and did not with them.
    #[test]
    fn memory_files_and_customizations_are_off() {
        let req = build_for(None);
        assert!(
            req.args.contains(&"--safe-mode".to_string()),
            "{:?}",
            req.args
        );
        for (name, value) in [
            ("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1"),
            ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1"),
        ] {
            assert!(
                req.env_set.contains(&(name.to_string(), value.to_string())),
                "{name} not set: {:?}",
                req.env_set
            );
        }
    }

    /// 060 T006: the working folder and timeout are the call's.
    #[test]
    fn the_folder_and_timeout_are_the_calls() {
        let req = build_for(None);
        assert_eq!(req.cwd, Path::new("empty-folder"));
        assert_eq!(req.timeout, Duration::from_secs(120));
    }

    /// 060 T006 (FR-005, analyze A5): the request names variables but holds
    /// no value from the operator's environment, so logging it leaks none.
    #[test]
    fn the_request_holds_no_environment_values() {
        let shown = format!("{:?}", build_for(Some(Effort::High)));
        for (name, value) in std::env::vars() {
            if value.len() >= 8 && !shown.contains(&name) {
                assert!(
                    !shown.contains(&value),
                    "the value of {name} reached the request"
                );
            }
        }
    }
}
