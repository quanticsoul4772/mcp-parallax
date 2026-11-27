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

/// Operators that make a command more than one whole command (Clarifications R).
const CHAINING: [&str; 11] = ["&&", ";", "|", "$(", "`", "&", ">", ">>", "<", "<(", ">("];

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{prefilter, Exclusion, Prefilter};

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
        for op in ["&&", ";", "|", "$(", "`", "&", ">", ">>", "<", "<(", ">("] {
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
}
