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
    max_retries: u32,
    backoff_base_ms: u64,
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
            max_retries: config.max_retries,
            backoff_base_ms: 500,
        }
    }
}

impl ClaudeCliClient {
    /// Set the pause before the first retry (each later one grows by the
    /// same step). Tests shorten it.
    #[must_use]
    pub const fn with_backoff_base_ms(mut self, ms: u64) -> Self {
        self.backoff_base_ms = ms;
        self
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
        // A provider 5xx is retried under `MAX_RETRIES`, with a growing pause;
        // every other result, a usage limit included, is final (D8, D10).
        let mut attempts: u32 = 0;
        loop {
            attempts += 1;
            let output = self.runner.run(&request).await?;
            if !classify::is_server_error(&output) {
                return classify::classify(&output);
            }
            if attempts > self.max_retries {
                let last = classify::classify(&output)
                    .err()
                    .map_or_else(String::new, |e| e.to_string());
                return Err(AppError::RetriesExhausted { attempts, last });
            }
            tokio::time::sleep(Duration::from_millis(
                self.backoff_base_ms.saturating_mul(u64::from(attempts)),
            ))
            .await;
        }
    }
}

/// A [`CliRunner`] that lets at most `max` runs through at once; the rest
/// wait their turn (research.md D5). Waiting does not count against a run's
/// own timeout, which starts when the process does.
pub struct LimitedRunner {
    inner: Arc<dyn CliRunner>,
    permits: Arc<tokio::sync::Semaphore>,
}

impl LimitedRunner {
    /// Wrap `inner` with a cap of `max` concurrent runs.
    #[must_use]
    pub fn new(inner: Arc<dyn CliRunner>, max: usize) -> Self {
        Self {
            inner,
            permits: Arc::new(tokio::sync::Semaphore::new(max)),
        }
    }
}

#[async_trait::async_trait]
impl CliRunner for LimitedRunner {
    async fn run(&self, request: &CliRequest) -> Result<CliOutput, AppError> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|e| AppError::Client(format!("claude CLI process limit closed: {e}")))?;
        self.inner.run(request).await
    }
}

/// The one runner every `claude_cli` client in this process shares.
///
/// Capped at `CLAUDE_CLI_CONCURRENCY` live processes server-wide (060
/// FR-009). Built on first use from that call's configuration; a process has
/// one.
#[must_use]
pub fn process_runner(config: &Config) -> Arc<dyn CliRunner> {
    static RUNNER: std::sync::OnceLock<Arc<dyn CliRunner>> = std::sync::OnceLock::new();
    Arc::clone(RUNNER.get_or_init(|| {
        Arc::new(LimitedRunner::new(
            Arc::new(TokioCliRunner),
            usize::from(config.claude_cli_concurrency),
        ))
    }))
}

/// The startup check on `claude_cli` (research.md D9).
///
/// `CLAUDE_CLI_PATH` must run `--version` within 10 s. Sign-in is not checked
/// here; it would spend a model call, and a signed-out CLI fails its first
/// call by name.
///
/// # Errors
///
/// [`AppError::Client`] naming `CLAUDE_CLI_PATH` when the program cannot be
/// started or exits non-zero.
pub async fn check_cli(config: &Config, runner: &dyn CliRunner) -> Result<String, AppError> {
    let request = CliRequest {
        program: config.claude_cli_path.clone(),
        args: vec!["--version".into()],
        stdin: String::new(),
        env_remove: Vec::new(),
        env_set: Vec::new(),
        cwd: std::env::temp_dir(),
        timeout: Duration::from_secs(10),
    };
    let named = |why: String| {
        AppError::Client(format!(
            "CLAUDE_CLI_PATH `{}` could not run `--version`: {why}",
            config.claude_cli_path
        ))
    };
    let output = runner
        .run(&request)
        .await
        .map_err(|e| named(e.to_string()))?;
    if output.exit_code != Some(0) {
        let stderr: String = output.stderr.chars().take(500).collect();
        return Err(named(format!(
            "exit {:?}; stderr: {stderr}",
            output.exit_code
        )));
    }
    Ok(output.stdout.trim().to_string())
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

    /// A runner double that holds each call briefly and records the most
    /// calls it ever had in flight at once.
    struct HoldingRunner {
        live: std::sync::atomic::AtomicUsize,
        peak: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl CliRunner for HoldingRunner {
        async fn run(&self, _: &CliRequest) -> Result<crate::traits::runner::CliOutput, AppError> {
            use std::sync::atomic::Ordering;
            let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(40)).await;
            self.live.fetch_sub(1, Ordering::SeqCst);
            Ok(crate::traits::runner::CliOutput {
                exit_code: Some(0),
                stdout: serde_json::json!({"subtype": "success", "structured_output": {}})
                    .to_string(),
                stderr: String::new(),
            })
        }
    }

    /// 060 T013 (US2, research.md D5): calls through one limited runner never
    /// exceed its cap, across two routed models, and all of them complete.
    #[tokio::test]
    async fn the_process_cap_holds_across_models_and_every_call_completes() {
        let holding = Arc::new(HoldingRunner {
            live: std::sync::atomic::AtomicUsize::new(0),
            peak: std::sync::atomic::AtomicUsize::new(0),
        });
        let limited: Arc<dyn CliRunner> = Arc::new(LimitedRunner::new(holding.clone(), 2));
        let config = crate::config::test_config();
        let folder = std::env::temp_dir();
        let a = Arc::new(ClaudeCliClient::with_runner(
            &config,
            limited.clone(),
            &folder,
            "claude-sonnet-5-5",
            None,
        ));
        let b = Arc::new(ClaudeCliClient::with_runner(
            &config,
            limited,
            &folder,
            "claude-haiku-5-5",
            None,
        ));
        let schema = serde_json::json!({});
        let calls = (0..8).map(|i| {
            let client = if i % 2 == 0 { a.clone() } else { b.clone() };
            let schema = schema.clone();
            async move { client.complete("p", &schema).await }
        });
        let results = futures::future::join_all(calls).await;
        assert!(results.iter().all(Result::is_ok), "every call completes");
        assert_eq!(
            holding.peak.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the cap is reached and never exceeded"
        );
    }

    /// 060 T014: every client the backend builds shares one process-wide
    /// runner, so the cap is server-wide, not per client.
    #[test]
    fn every_built_client_shares_the_process_runner() {
        let config = crate::config::test_config();
        assert!(Arc::ptr_eq(
            &process_runner(&config),
            &process_runner(&config)
        ));
    }

    fn status_reply(status: u16) -> crate::traits::runner::CliOutput {
        crate::traits::runner::CliOutput {
            exit_code: Some(1),
            stdout: serde_json::json!({
                "subtype": "success", "is_error": true,
                "api_error_status": status, "result": "upstream trouble"
            })
            .to_string(),
            stderr: String::new(),
        }
    }

    fn ok_reply() -> crate::traits::runner::CliOutput {
        crate::traits::runner::CliOutput {
            exit_code: Some(0),
            stdout: serde_json::json!({"subtype": "success", "structured_output": {"ok": true}})
                .to_string(),
            stderr: String::new(),
        }
    }

    fn retrying_client(runner: crate::traits::runner::MockCliRunner) -> ClaudeCliClient {
        ClaudeCliClient::with_runner(
            &crate::config::Config {
                max_retries: 1,
                ..crate::config::test_config()
            },
            Arc::new(runner),
            std::path::Path::new("empty-folder"),
            "claude-sonnet-5-5",
            None,
        )
        .with_backoff_base_ms(1)
    }

    /// 060 T021 (US4): a 5xx is retried, and a success on the retry is the
    /// result.
    #[tokio::test]
    async fn a_server_error_is_retried() {
        let mut runner = crate::traits::runner::MockCliRunner::new();
        let mut seq = mockall::Sequence::new();
        runner
            .expect_run()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| Ok(status_reply(529)));
        runner
            .expect_run()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| Ok(ok_reply()));
        let out = retrying_client(runner)
            .complete("p", &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(out.value, serde_json::json!({"ok": true}));
    }

    /// 060 T021 (US4): 5xx on every attempt ends as retries exhausted, after
    /// `MAX_RETRIES` retries.
    #[tokio::test]
    async fn server_errors_on_every_attempt_exhaust_the_retries() {
        let mut runner = crate::traits::runner::MockCliRunner::new();
        runner
            .expect_run()
            .times(2)
            .returning(|_| Ok(status_reply(500)));
        let err = retrying_client(runner)
            .complete("p", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::RetriesExhausted { attempts: 2, .. }),
            "{err:?}"
        );
    }

    /// 060 T021 (US4, D10): a usage limit is not retried; it lasts hours.
    #[tokio::test]
    async fn a_usage_limit_is_not_retried() {
        let mut runner = crate::traits::runner::MockCliRunner::new();
        runner
            .expect_run()
            .times(1)
            .returning(|_| Ok(status_reply(429)));
        let err = retrying_client(runner)
            .complete("p", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::UsageLimit(_)), "{err:?}");
    }

    fn version_config() -> crate::config::Config {
        crate::config::Config {
            claude_cli_path: "C:/tools/claude.exe".into(),
            ..crate::config::test_config()
        }
    }

    /// 060 T023 (US4 scenario 1, D9): a CLI that runs `--version` passes the
    /// startup check, which reports its version; the check asks for nothing
    /// else and is bounded at 10 s.
    #[tokio::test]
    async fn the_startup_check_reports_the_cli_version() {
        let mut runner = crate::traits::runner::MockCliRunner::new();
        runner
            .expect_run()
            .withf(|req| {
                req.program == "C:/tools/claude.exe"
                    && req.args == ["--version"]
                    && req.timeout == Duration::from_secs(10)
            })
            .times(1)
            .returning(|_| {
                Ok(crate::traits::runner::CliOutput {
                    exit_code: Some(0),
                    stdout: "2.1.330 (Claude Code)\n".into(),
                    stderr: String::new(),
                })
            });
        let version = check_cli(&version_config(), &runner).await.unwrap();
        assert_eq!(version, "2.1.330 (Claude Code)");
    }

    /// 060 T023 (US4 scenario 1, FR-007): a CLI that cannot start, or exits
    /// non-zero on `--version`, fails startup with an error naming
    /// `CLAUDE_CLI_PATH`.
    #[tokio::test]
    async fn a_cli_that_cannot_run_fails_startup_naming_the_setting() {
        let mut missing = crate::traits::runner::MockCliRunner::new();
        missing
            .expect_run()
            .returning(|_| Err(AppError::Client("could not start".into())));
        let mut failing = crate::traits::runner::MockCliRunner::new();
        failing.expect_run().returning(|_| {
            Ok(crate::traits::runner::CliOutput {
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "boom".into(),
            })
        });
        for runner in [missing, failing] {
            let err = check_cli(&version_config(), &runner).await.unwrap_err();
            assert!(err.to_string().contains("CLAUDE_CLI_PATH"), "{err}");
        }
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
