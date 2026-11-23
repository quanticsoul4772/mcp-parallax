//! The `claude_cli` backend (060): each model call runs as one isolated
//! `claude -p` process on the operator's own signed-in Claude Code.

use crate::config::Config;
use crate::error::AppError;
use crate::routing::Effort;
use crate::traits::client::{Completion, ModelClient};
use crate::traits::runner::{CliOutput, CliRequest, CliRunner};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub mod classify;
pub mod invocation;

/// One routed `(model, effort)` on the `claude_cli` backend: each
/// [`ModelClient::complete`] runs one isolated `claude -p` process through the
/// runner and classifies its reply.
pub struct ClaudeCliClient {
    runner: Arc<dyn CliRunner>,
    program: String,
    model: String,
    effort: Option<Effort>,
    cwd: PathBuf,
    timeout: Duration,
}

impl ClaudeCliClient {
    /// Build the client for one routed `(model, effort)`, running
    /// `CLAUDE_CLI_PATH` in `cwd` under `REQUEST_TIMEOUT_MS`.
    #[must_use]
    pub fn with_runner(
        config: &Config,
        runner: Arc<dyn CliRunner>,
        cwd: &Path,
        model: &str,
        effort: Option<Effort>,
    ) -> Self {
        Self {
            runner,
            program: config.claude_cli_path.clone(),
            model: model.to_string(),
            effort,
            cwd: cwd.to_path_buf(),
            timeout: Duration::from_millis(config.request_timeout_ms),
        }
    }
}

/// The empty working folder this server process gives every CLI call: under
/// the OS temp folder, named for the process, created on first use and never
/// written to by Parallax (research.md D2).
#[must_use]
pub fn process_folder() -> PathBuf {
    std::env::temp_dir().join(format!("parallax-claude-cli-{}", std::process::id()))
}

#[async_trait::async_trait]
impl ModelClient for ClaudeCliClient {
    async fn complete(&self, prompt: &str, schema: &Value) -> Result<Completion, AppError> {
        std::fs::create_dir_all(&self.cwd).map_err(|e| {
            AppError::Client(format!(
                "could not create the claude CLI working folder {}: {e}",
                self.cwd.display()
            ))
        })?;
        let request = invocation::build(&invocation::CallSpec {
            program: &self.program,
            model: &self.model,
            effort: self.effort,
            schema,
            prompt,
            cwd: &self.cwd,
            timeout: self.timeout,
        });
        let output = self.runner.run(&request).await?;
        classify::classify(&output)
    }
}

/// What a runner timeout names (research.md D7).
const TIMEOUT_WHAT: &str = "claude CLI";

/// The process-backed [`CliRunner`]: the child's stdin, stdout and stderr are
/// piped, never inherited (stdout is the MCP channel), and the child is
/// killed when its timeout passes or the call is dropped.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioCliRunner;

#[async_trait::async_trait]
impl CliRunner for TokioCliRunner {
    async fn run(&self, request: &CliRequest) -> Result<CliOutput, AppError> {
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(&request.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for name in &request.env_remove {
            command.env_remove(name);
        }
        for (name, value) in &request.env_set {
            command.env(name, value);
        }
        let mut child = command
            .spawn()
            .map_err(|e| AppError::Client(format!("could not start `{}`: {e}", request.program)))?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            AppError::Client(format!("`{}` stdin was not piped", request.program))
        })?;
        let input = request.stdin.clone();
        let write = async move {
            if !input.is_empty() {
                stdin.write_all(input.as_bytes()).await?;
            }
            // Closing stdin is what tells the child the prompt is complete.
            drop(stdin);
            Ok::<(), std::io::Error>(())
        };
        // Write and read together: a child that fills its stdout pipe before
        // reading all of stdin would otherwise deadlock against the writer.
        let both = async { tokio::join!(write, child.wait_with_output()) };
        let (written, output) =
            tokio::time::timeout(request.timeout, both)
                .await
                .map_err(|_| AppError::Timeout {
                    what: TIMEOUT_WHAT,
                    ms: u64::try_from(request.timeout.as_millis()).unwrap_or(u64::MAX),
                })?;
        written.map_err(|e| {
            AppError::Client(format!(
                "writing stdin to `{}` failed: {e}",
                request.program
            ))
        })?;
        let output = output.map_err(|e| {
            AppError::Client(format!("waiting for `{}` failed: {e}", request.program))
        })?;
        Ok(CliOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::traits::runner::{CliRequest, CliRunner};
    use std::time::{Duration, Instant};

    fn standin(program: &str, args: &[&str], timeout: Duration) -> CliRequest {
        CliRequest {
            program: program.into(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            stdin: String::new(),
            env_remove: Vec::new(),
            env_set: Vec::new(),
            cwd: std::env::temp_dir(),
            timeout,
        }
    }

    fn cli_client(runner: crate::traits::runner::MockCliRunner) -> ClaudeCliClient {
        ClaudeCliClient::with_runner(
            &crate::config::Config {
                claude_cli_path: "C:/tools/claude.exe".into(),
                request_timeout_ms: 90_000,
                ..crate::config::test_config()
            },
            std::sync::Arc::new(runner),
            std::path::Path::new("empty-folder"),
            "claude-haiku-5-5",
            Some(crate::routing::Effort::Low),
        )
    }

    /// 060 T010: `complete` sends exactly the contract's request for the
    /// client's program, model, effort, folder and timeout, with the prompt
    /// and schema of the call.
    #[tokio::test]
    async fn complete_sends_the_contract_request() {
        let schema = serde_json::json!({"type": "object"});
        let expected = invocation::build(&invocation::CallSpec {
            program: "C:/tools/claude.exe",
            model: "claude-haiku-5-5",
            effort: Some(crate::routing::Effort::Low),
            schema: &schema,
            prompt: "the prompt",
            cwd: std::path::Path::new("empty-folder"),
            timeout: Duration::from_secs(90),
        });
        let mut runner = crate::traits::runner::MockCliRunner::new();
        runner
            .expect_run()
            .withf(move |req| *req == expected)
            .times(1)
            .returning(|_| {
                Ok(crate::traits::runner::CliOutput {
                    exit_code: Some(0),
                    stdout: serde_json::json!({
                        "subtype": "success",
                        "structured_output": {"ok": true},
                        "usage": {"input_tokens": 3, "output_tokens": 4},
                        "total_cost_usd": 0.002
                    })
                    .to_string(),
                    stderr: String::new(),
                })
            });

        let out = cli_client(runner)
            .complete("the prompt", &schema)
            .await
            .unwrap();
        assert_eq!(out.value, serde_json::json!({"ok": true}));
        assert_eq!((out.input_tokens, out.output_tokens), (3, 4));
        assert_eq!(out.reported_cost_micros, Some(2_000));
    }

    /// 060 T011: on `claude_cli`, `build_model_client` returns the CLI client:
    /// a call runs the configured program (one that does not exist, so no
    /// real CLI starts).
    #[tokio::test]
    async fn build_model_client_on_claude_cli_runs_the_configured_cli() {
        let cfg = crate::config::Config {
            backend: crate::config::Backend::ClaudeCli,
            claude_cli_path: "parallax-no-such-cli".into(),
            ..crate::config::test_config()
        };
        let client = crate::client::build_model_client(
            &cfg,
            &reqwest::Client::new(),
            "claude-sonnet-5-5",
            None,
        );
        let err = client
            .complete("p", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::Client(m) if m.contains("parallax-no-such-cli")),
            "{err:?}"
        );
    }

    /// 060 T010: a runner failure is the call's failure, unchanged.
    #[tokio::test]
    async fn a_runner_failure_is_the_calls_failure() {
        let mut runner = crate::traits::runner::MockCliRunner::new();
        runner.expect_run().returning(|_| {
            Err(AppError::Timeout {
                what: "claude CLI",
                ms: 90_000,
            })
        });
        let err = cli_client(runner)
            .complete("p", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::Timeout {
                    what: "claude CLI",
                    ms: 90_000
                }
            ),
            "{err:?}"
        );
    }

    /// 060 T003 (analyze A1): stdout is captured and the exit code returned,
    /// against a stand-in program that exists wherever the suite builds.
    #[tokio::test]
    async fn the_runner_captures_stdout_and_the_exit_code() {
        let out = TokioCliRunner
            .run(&standin("rustc", &["--version"], Duration::from_secs(60)))
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0), "{out:?}");
        assert!(out.stdout.starts_with("rustc "), "{out:?}");
    }

    /// 060 T003 (analyze A1, FR-004): removed variables are absent from the
    /// child's environment and set ones present. The removed variable is one
    /// the test process has, so the removal is real, not vacuous.
    #[tokio::test]
    async fn the_runner_removes_and_sets_environment_variables() {
        let removed = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        assert!(
            std::env::var_os(removed).is_some(),
            "{removed} must be set in the test process for the removal to mean anything"
        );
        let mut req = if cfg!(windows) {
            standin("cmd", &["/C", "set"], Duration::from_secs(60))
        } else {
            standin("env", &[], Duration::from_secs(60))
        };
        req.env_remove = vec![removed.to_string()];
        req.env_set = vec![("PARALLAX_RUNNER_TEST".into(), "set-by-runner".into())];

        let out = TokioCliRunner.run(&req).await.unwrap();
        let lines: Vec<&str> = out.stdout.lines().collect();
        let prefix = format!("{removed}=");
        assert!(
            !lines
                .iter()
                .any(|l| l.to_ascii_uppercase().starts_with(&prefix)),
            "{removed} reached the child"
        );
        assert!(
            lines.contains(&"PARALLAX_RUNNER_TEST=set-by-runner"),
            "the set variable did not reach the child"
        );
    }

    /// 060 T003 (analyze A1, FR-011): a run past its timeout is killed and
    /// reported as a timeout, without waiting for the program to finish.
    #[tokio::test]
    async fn the_runner_kills_a_run_past_its_timeout() {
        let req = if cfg!(windows) {
            standin(
                "ping",
                &["-n", "30", "127.0.0.1"],
                Duration::from_millis(500),
            )
        } else {
            standin("sleep", &["30"], Duration::from_millis(500))
        };
        let started = Instant::now();
        let err = TokioCliRunner.run(&req).await.unwrap_err();
        assert!(
            matches!(
                err,
                AppError::Timeout {
                    what: "claude CLI",
                    ms: 500
                }
            ),
            "{err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waited for the program"
        );
    }

    /// 060 T003: a program that cannot be started is a client error naming it.
    #[tokio::test]
    async fn a_missing_program_is_a_client_error_naming_it() {
        let err = TokioCliRunner
            .run(&standin(
                "parallax-no-such-program",
                &[],
                Duration::from_secs(5),
            ))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::Client(m) if m.contains("parallax-no-such-program")),
            "{err:?}"
        );
    }
}
