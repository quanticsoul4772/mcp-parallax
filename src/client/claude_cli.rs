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
    folder: Folder,
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
            folder: Folder::Given(cwd.to_path_buf()),
            timeout: Duration::from_millis(config.request_timeout_ms),
            max_retries: config.max_retries,
            backoff_base_ms: 500,
        }
    }
}

impl ClaudeCliClient {
    /// The runner this client runs calls through (tests compare it).
    #[cfg(test)]
    pub(crate) const fn runner_handle(&self) -> &Arc<dyn CliRunner> {
        &self.runner
    }

    /// Set the pause before the first retry (each later one grows by the
    /// same step). Tests shorten it.
    #[must_use]
    pub const fn with_backoff_base_ms(mut self, ms: u64) -> Self {
        self.backoff_base_ms = ms;
        self
    }
}

/// Where a client's calls run.
enum Folder {
    /// A folder the caller chose (tests); used as given, never created.
    Given(PathBuf),
    /// This process's private working folder (see [`process_folder`]).
    Process,
}

/// The working folder every CLI call of this process runs in: created once,
/// fresh and uniquely named under the OS temp folder (060 review finding 1).
fn process_folder() -> Result<&'static Path, AppError> {
    static FOLDER: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    FOLDER
        .get_or_init(|| create_private_folder(&std::env::temp_dir()).map_err(|e| e.to_string()))
        .as_deref()
        .map_err(|e| {
            AppError::Client(format!(
                "could not create the claude CLI working folder: {e}"
            ))
        })
}

/// Create a fresh, uniquely named, empty working folder under `parent`.
///
/// 060 review finding 1: `create_dir` fails on an existing path, so a folder
/// someone else prepared is never reused; on Unix it is private to the
/// operator.
///
/// # Errors
///
/// The I/O error when the folder cannot be created.
pub fn create_private_folder(parent: &Path) -> std::io::Result<PathBuf> {
    let folder = parent.join(format!(
        "parallax-claude-cli-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&folder)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(folder)
}

/// Build the backend's client for one routed `(model, effort)`: the shared
/// process runner and the process's working folder.
#[must_use]
pub fn build_cli_client(config: &Config, model: &str, effort: Option<Effort>) -> ClaudeCliClient {
    let mut client =
        ClaudeCliClient::with_runner(config, process_runner(config), Path::new(""), model, effort);
    client.folder = Folder::Process;
    client
}

#[async_trait::async_trait]
impl ModelClient for ClaudeCliClient {
    async fn complete(&self, prompt: &str, schema: &Value) -> Result<Completion, AppError> {
        let cwd: &Path = match &self.folder {
            Folder::Given(path) => path,
            Folder::Process => process_folder()?,
        };
        let request = invocation::build(&invocation::CallSpec {
            program: &self.program,
            model: &self.model,
            effort: self.effort,
            schema,
            prompt,
            cwd,
            timeout: self.timeout,
        });
        // 060 review finding 3: a call that ran but left no reply still counts
        // in the invocation's meter, as one without a figure.
        let unmetered = || {
            let _outside_an_invocation = crate::telemetry::CLI_COST.try_with(|m| m.record(None));
        };
        // A provider 5xx is retried under `MAX_RETRIES`, with a growing pause;
        // every other result, a usage limit included, is final (D8, D10).
        let mut attempts: u32 = 0;
        loop {
            attempts += 1;
            let output = match self.runner.run(&request).await {
                Ok(output) => output,
                Err(e) => {
                    unmetered();
                    return Err(e);
                }
            };
            if !classify::is_server_error(&output) {
                let result = classify::classify(&output);
                // 060 D11: the invocation's meter gets this call's figure; a
                // failed call reports none, which makes the total unknown.
                let figure = result.as_ref().ok().and_then(|c| c.reported_cost_micros);
                let _outside_an_invocation =
                    crate::telemetry::CLI_COST.try_with(|m| m.record(figure));
                return result;
            }
            if attempts > self.max_retries {
                let last = classify::classify(&output)
                    .err()
                    .map_or_else(String::new, |e| e.to_string());
                unmetered();
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
    // 060 review finding 7: a launcher script runs the CLI under a shell, and
    // a timeout would kill only the shell, leaving the real CLI running.
    let is_script = Path::new(&config.claude_cli_path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"));
    if is_script {
        return Err(AppError::Client(format!(
            "CLAUDE_CLI_PATH `{}` is a launcher script; point it at the native claude.exe \
             (a timeout cannot stop a CLI started through a script)",
            config.claude_cli_path
        )));
    }
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
        // A child that exits without reading all of its stdin (an argument
        // error, an early rejection) breaks the pipe; its own reply is the
        // useful answer then, so only other write errors fail the run (060
        // review finding 6).
        if let Err(e) = written {
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(AppError::Client(format!(
                    "writing stdin to `{}` failed: {e}",
                    request.program
                )));
            }
        }
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

    /// 060 T014, review finding 8: the clients the backend builds hold the
    /// process-wide runner itself, so the cap is server-wide, not per client.
    #[test]
    fn every_built_client_shares_the_process_runner() {
        let config = crate::config::test_config();
        let a = build_cli_client(&config, "claude-sonnet-5-5", None);
        let b = build_cli_client(&config, "claude-haiku-5-5", None);
        assert!(Arc::ptr_eq(a.runner_handle(), b.runner_handle()));
        assert!(Arc::ptr_eq(a.runner_handle(), &process_runner(&config)));
    }

    /// 060 review finding 1: the working folder is one Parallax creates
    /// itself, fresh and uniquely named; an existing folder is never reused.
    #[test]
    fn the_working_folder_is_fresh_and_never_reused() {
        let parent = tempfile::tempdir().unwrap();
        let a = create_private_folder(parent.path()).unwrap();
        let b = create_private_folder(parent.path()).unwrap();
        assert_ne!(a, b, "each folder is new");
        assert!(a.is_dir() && std::fs::read_dir(&a).unwrap().next().is_none());
        assert!(create_private_folder(&parent.path().join("missing")).is_err());
    }

    /// 060 review finding 3: a call that ran but produced no reply (a runner
    /// failure, or retries running out) still counts in the meter, so the
    /// invocation's CLI total becomes unknown rather than a partial sum.
    #[tokio::test]
    async fn a_call_without_a_reply_makes_the_meter_total_unknown() {
        let mut runner = crate::traits::runner::MockCliRunner::new();
        let mut seq = mockall::Sequence::new();
        runner
            .expect_run()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| Ok(ok_reply_with_cost()));
        runner
            .expect_run()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| {
                Err(AppError::Timeout {
                    what: "claude CLI",
                    ms: 1,
                })
            });
        let client = retrying_client(runner);
        let meter = Arc::new(crate::telemetry::CliCost::default());
        crate::telemetry::CLI_COST
            .scope(meter.clone(), async {
                client.complete("a", &serde_json::json!({})).await.unwrap();
                client
                    .complete("b", &serde_json::json!({}))
                    .await
                    .unwrap_err();
            })
            .await;
        assert_eq!(meter.total(), None);

        let mut exhausted = crate::traits::runner::MockCliRunner::new();
        exhausted
            .expect_run()
            .times(2)
            .returning(|_| Ok(status_reply(500)));
        let client = retrying_client(exhausted);
        let meter = Arc::new(crate::telemetry::CliCost::default());
        crate::telemetry::CLI_COST
            .scope(meter.clone(), async {
                client
                    .complete("c", &serde_json::json!({}))
                    .await
                    .unwrap_err();
            })
            .await;
        assert_eq!(meter.total(), None, "an exhausted call is counted too");
    }

    fn ok_reply_with_cost() -> crate::traits::runner::CliOutput {
        crate::traits::runner::CliOutput {
            exit_code: Some(0),
            stdout: serde_json::json!({
                "subtype": "success", "structured_output": {}, "total_cost_usd": 0.001
            })
            .to_string(),
            stderr: String::new(),
        }
    }

    /// 060 review finding 7 (FR-011): a `.cmd` or `.bat` launcher is refused
    /// at startup, because a timeout would kill only the shell and leave the
    /// real CLI running; nothing is started.
    #[tokio::test]
    async fn a_launcher_script_is_refused_at_startup() {
        for path in ["C:/npm/claude.cmd", "C:/tools/CLAUDE.BAT"] {
            let config = crate::config::Config {
                claude_cli_path: path.into(),
                ..crate::config::test_config()
            };
            let runner = crate::traits::runner::MockCliRunner::new();
            let err = check_cli(&config, &runner).await.unwrap_err().to_string();
            assert!(
                err.contains("CLAUDE_CLI_PATH") && err.contains("claude.exe"),
                "{path}: {err}"
            );
        }
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

    /// 060 T019 (D11): inside an invocation's cost meter, each completed call
    /// records the CLI's figure; outside one, nothing is recorded anywhere.
    #[tokio::test]
    async fn completed_calls_record_the_cli_figure_in_the_invocation_meter() {
        let mut runner = crate::traits::runner::MockCliRunner::new();
        runner.expect_run().times(3).returning(|_| {
            Ok(crate::traits::runner::CliOutput {
                exit_code: Some(0),
                stdout: serde_json::json!({
                    "subtype": "success", "structured_output": {}, "total_cost_usd": 0.002
                })
                .to_string(),
                stderr: String::new(),
            })
        });
        let client = cli_client(runner);
        let meter = Arc::new(crate::telemetry::CliCost::default());
        crate::telemetry::CLI_COST
            .scope(meter.clone(), async {
                client.complete("a", &serde_json::json!({})).await.unwrap();
                client.complete("b", &serde_json::json!({})).await.unwrap();
            })
            .await;
        let total = meter.total().unwrap();
        assert!((total - 0.004).abs() < 1e-12, "{total}");
        // Outside a meter the call still succeeds; there is just no meter.
        client.complete("c", &serde_json::json!({})).await.unwrap();
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

    /// 060 review finding 8 (FR-011): the timed-out process is dead, not just
    /// abandoned: a stand-in that would write a marker after 2 s never does.
    #[tokio::test]
    async fn a_timed_out_process_is_dead() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("still-alive");
        // The marker is written into the stand-in's own working folder, so
        // the command needs no quoted path (Windows argument escaping would
        // mangle one and the marker would never appear, alive or not).
        let mut req = if cfg!(windows) {
            standin(
                "cmd",
                &["/C", "ping -n 3 127.0.0.1 >NUL && type nul > still-alive"],
                Duration::from_millis(300),
            )
        } else {
            standin(
                "sh",
                &["-c", "sleep 2; touch still-alive"],
                Duration::from_millis(300),
            )
        };
        req.cwd = dir.path().to_path_buf();
        assert!(TokioCliRunner.run(&req).await.is_err());
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(!marker.exists(), "the process outlived its timeout");
    }

    /// 060 review finding 6: a program that exits without reading its stdin
    /// still returns its own output; the broken pipe does not replace it.
    #[tokio::test]
    async fn output_wins_over_a_broken_stdin_pipe() {
        let mut req = standin("rustc", &["--version"], Duration::from_secs(60));
        req.stdin = "x".repeat(4 * 1024 * 1024);
        let out = TokioCliRunner.run(&req).await.unwrap();
        assert_eq!(out.exit_code, Some(0), "{out:?}");
        assert!(out.stdout.starts_with("rustc "), "{out:?}");
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
