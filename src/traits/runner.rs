//! The process-runner seam (060): one external program run to completion.
//!
//! The `claude_cli` backend runs each model call as a child process. The
//! process itself is the external effect, so it sits behind [`CliRunner`] and
//! every test of the backend uses a double; no test starts the real CLI.

use crate::error::AppError;
use std::path::PathBuf;
use std::time::Duration;

/// One program run: everything the child sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRequest {
    /// The program to start (a command name or a path).
    pub program: String,
    /// Its arguments, in order. Never carries the prompt (Windows caps a
    /// command line at 32,767 characters).
    pub args: Vec<String>,
    /// Written to the child's stdin, which is then closed.
    pub stdin: String,
    /// Variables removed from the inherited environment, by name only.
    pub env_remove: Vec<String>,
    /// Variables set in the child's environment, after the removals.
    pub env_set: Vec<(String, String)>,
    /// The child's working folder.
    pub cwd: PathBuf,
    /// The child is killed when this elapses.
    pub timeout: Duration,
}

/// What a finished program produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    /// The exit code; `None` when the process was ended by a signal.
    pub exit_code: Option<i32>,
    /// Everything written to stdout (lossy UTF-8).
    pub stdout: String,
    /// Everything written to stderr (lossy UTF-8).
    pub stderr: String,
}

/// Runs one program to completion.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait CliRunner: Send + Sync {
    /// Start `request.program`, feed it `request.stdin`, and wait for it.
    ///
    /// # Errors
    ///
    /// [`AppError::Timeout`] when the run passes `request.timeout` (the child
    /// is killed); [`AppError::Client`] when the program cannot be started or
    /// its stdin cannot be written.
    async fn run(&self, request: &CliRequest) -> Result<CliOutput, AppError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn request() -> CliRequest {
        CliRequest {
            program: "claude".into(),
            args: vec!["-p".into(), "--output-format".into(), "json".into()],
            stdin: "the prompt".into(),
            env_remove: vec!["ANTHROPIC_API_KEY".into()],
            env_set: vec![("CLAUDE_CODE_EFFORT_LEVEL".into(), "unset".into())],
            cwd: PathBuf::from("empty"),
            timeout: Duration::from_secs(120),
        }
    }

    /// 060 T002: a request reaches the runner whole, and its output comes back
    /// as exit code, stdout and stderr.
    #[tokio::test]
    async fn a_request_returns_the_programs_output() {
        let mut mock = MockCliRunner::new();
        mock.expect_run()
            .withf(|req| {
                req.program == "claude"
                    && req.args == ["-p", "--output-format", "json"]
                    && req.stdin == "the prompt"
                    && req.env_remove == ["ANTHROPIC_API_KEY"]
                    && req.env_set
                        == [("CLAUDE_CODE_EFFORT_LEVEL".to_string(), "unset".to_string())]
                    && req.cwd == std::path::Path::new("empty")
                    && req.timeout == Duration::from_secs(120)
            })
            .returning(|_| {
                Ok(CliOutput {
                    exit_code: Some(0),
                    stdout: "{\"ok\":true}".into(),
                    stderr: String::new(),
                })
            });

        let out = mock.run(&request()).await.unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout, "{\"ok\":true}");
        assert!(out.stderr.is_empty());
    }

    /// 060 T002: a run past its timeout is a timeout error, not an output.
    #[tokio::test]
    async fn a_run_past_its_timeout_is_a_timeout_error() {
        let mut mock = MockCliRunner::new();
        mock.expect_run().returning(|req| {
            Err(AppError::Timeout {
                what: "claude CLI",
                ms: u64::try_from(req.timeout.as_millis()).unwrap(),
            })
        });

        let err = mock.run(&request()).await.unwrap_err();
        assert!(
            matches!(
                err,
                AppError::Timeout {
                    what: "claude CLI",
                    ms: 120_000
                }
            ),
            "{err:?}"
        );
    }
}
