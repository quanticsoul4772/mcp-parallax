//! The `claude_cli` backend (060): each model call runs as one isolated
//! `claude -p` process on the operator's own signed-in Claude Code.

use crate::error::AppError;
use crate::traits::runner::{CliOutput, CliRequest, CliRunner};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

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
