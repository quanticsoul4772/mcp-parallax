//! The rmcp server handler.
//!
//! The tool surface, the distinct error surface (FR-007), and the single exit
//! point where every invocation — success, failure, or abandonment — leaves
//! exactly one record (FR-010).

use crate::config::Config;
use crate::error::{AppError, Outcome};
use crate::modes::verify::{self, Verdict, VerifyParams, VERIFY_ID};
use crate::modes::ModeRegistry;
use crate::telemetry::InvocationRecord;
use crate::traits::client::ModelClient;
use crate::traits::clock::TimeProvider;
use crate::traits::storage::Storage;
use chrono::{DateTime, Utc};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorData, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use std::sync::Arc;

/// The Parallax MCP server: the seams it composes plus the mode registry.
#[derive(Clone)]
pub struct Parallax {
    // Read only inside the #[tool_handler]-generated impl; rustc's dead-code
    // pass doesn't see through the macro.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    client: Arc<dyn ModelClient>,
    storage: Arc<dyn Storage>,
    clock: Arc<dyn TimeProvider>,
    registry: Arc<ModeRegistry>,
    /// Per-process session UUID (one stdio connection per process).
    session_id: String,
    model: String,
    max_claim_chars: usize,
}

#[tool_router]
impl Parallax {
    /// Compose the server from its seams. Registers all modes — an illegal
    /// mode schema fails here, at boot, not on the first call.
    ///
    /// # Errors
    ///
    /// Propagates the registry's flat+closed schema assertion.
    pub fn new(
        client: Arc<dyn ModelClient>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn TimeProvider>,
        config: &Config,
    ) -> Result<Self, AppError> {
        let mut registry = ModeRegistry::new();
        verify::register(&mut registry, config.verify_ensemble_k)?;
        Ok(Self {
            tool_router: Self::tool_router(),
            client,
            storage,
            clock,
            registry: Arc::new(registry),
            session_id: uuid::Uuid::new_v4().to_string(),
            model: config.anthropic_model.clone(),
            max_claim_chars: config.verify_max_claim_chars,
        })
    }

    /// The `verify` tool: k stance-blind passes, aggregated verdict.
    #[tool(
        name = "verify",
        description = "Independently verify a claim. Runs multiple parallel verification passes \
        that see only the claim and optional context - never the requester's stance or \
        conversation. Returns a structured verdict: supported or refuted, specific concrete \
        findings (every refutation names the exact error), and a confidence score derived from \
        cross-pass agreement. Use when an assertion matters and being confidently wrong is costly."
    )]
    pub async fn verify(
        &self,
        Parameters(params): Parameters<VerifyParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<Verdict>, ErrorData> {
        // context.ct fires on an MCP cancellation notification and on service
        // shutdown (client disconnect ends the stdio service).
        self.verify_with_ct(params, context.ct).await
    }

    async fn verify_with_ct(
        &self,
        params: VerifyParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<Verdict>, ErrorData> {
        let mode = self.registry.get(VERIFY_ID).ok_or_else(|| {
            ErrorData::internal_error("verify mode not registered".to_string(), None)
        })?;

        // Single exit point: the guard guarantees exactly one record on every
        // path — cancellation via the select arm, abandonment (future dropped)
        // via the guard's Drop backstop.
        let guard = RecordGuard::new(
            Arc::clone(&self.storage),
            Arc::clone(&self.clock),
            self.session_id.clone(),
            VERIFY_ID.to_string(),
            self.model.clone(),
        );

        tokio::select! {
            () = ct.cancelled() => {
                guard.finish(0, 0, Outcome::Cancelled).await;
                Err(to_error_data(&AppError::Cancelled))
            }
            result = verify::run(self.client.as_ref(), mode, &params, self.max_claim_chars) => {
                match result {
                    Ok(run) => {
                        guard
                            .finish(run.input_tokens, run.output_tokens, Outcome::Success)
                            .await;
                        Ok(Json(run.verdict))
                    }
                    Err(error) => {
                        // Token usage on failed invocations is not attributable
                        // (failed passes carry no usage) — recorded as zero.
                        guard.finish(0, 0, error.outcome()).await;
                        Err(to_error_data(&error))
                    }
                }
            }
        }
    }
}

#[tool_handler]
impl ServerHandler for Parallax {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Parallax: independent correctives for the calling model's blind spots. \
             Call `verify` when an assertion matters and being confidently wrong is costly.",
        )
    }
}

/// Map every failure class to a distinct, descriptive MCP error (FR-007 /
/// SC-005): the class is identifiable from the message alone, via the
/// outcome-taxonomy prefix plus the `AppError` Display text.
fn to_error_data(error: &AppError) -> ErrorData {
    let message = format!("[{}] {error}", error.outcome().as_str());
    match error {
        AppError::InvalidInput(_) => ErrorData::invalid_params(message, None),
        _ => ErrorData::internal_error(message, None),
    }
}

/// Drop-guard around one invocation: `finish()` writes the real record;
/// dropping unfinished (the request future was abandoned) records `cancelled`.
struct RecordGuard {
    storage: Arc<dyn Storage>,
    clock: Arc<dyn TimeProvider>,
    session_id: String,
    tool: String,
    model: String,
    started_at: DateTime<Utc>,
    done: bool,
}

impl RecordGuard {
    fn new(
        storage: Arc<dyn Storage>,
        clock: Arc<dyn TimeProvider>,
        session_id: String,
        tool: String,
        model: String,
    ) -> Self {
        let started_at = clock.now();
        Self {
            storage,
            clock,
            session_id,
            tool,
            model,
            started_at,
            done: false,
        }
    }

    async fn finish(mut self, input_tokens: u64, output_tokens: u64, outcome: Outcome) {
        self.done = true;
        let record = InvocationRecord::create(
            self.clock.as_ref(),
            &self.session_id,
            &self.tool,
            &self.model,
            input_tokens,
            output_tokens,
            outcome,
            self.started_at,
        );
        record.emit();
        if let Err(e) = self.storage.record_invocation(&record).await {
            // The record write itself failed — surface loudly on the
            // diagnostic stream; never on the protocol channel.
            tracing::error!(error = %e, "invocation record write failed");
        }
    }
}

impl Drop for RecordGuard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Abandoned mid-flight: the edge case "client disconnects
        // mid-invocation" — record `cancelled` (spec edge case 4).
        let record = InvocationRecord::create(
            self.clock.as_ref(),
            &self.session_id,
            &self.tool,
            &self.model,
            0,
            0,
            Outcome::Cancelled,
            self.started_at,
        );
        record.emit();
        let storage = Arc::clone(&self.storage);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(e) = storage.record_invocation(&record).await {
                    tracing::error!(error = %e, "cancelled-invocation record write failed");
                }
            });
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::storage::SqliteStorage;
    use crate::traits::client::{Completion, MockModelClient};
    use crate::traits::clock::SystemClock;
    use serde_json::json;
    use std::time::Duration;

    fn test_config() -> Config {
        Config {
            anthropic_api_key: "test-key".into(),
            anthropic_model: "claude-opus-4-8".into(),
            verify_ensemble_k: 3,
            verify_max_claim_chars: 50_000,
            database_path: ":memory:".into(),
            log_level: "info".into(),
            request_timeout_ms: 2_000,
            max_retries: 1,
        }
    }

    async fn server_with(client: MockModelClient) -> (Parallax, Arc<SqliteStorage>) {
        let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
        let server = Parallax::new(
            Arc::new(client),
            storage.clone(),
            Arc::new(SystemClock),
            &test_config(),
        )
        .unwrap();
        (server, storage)
    }

    #[tokio::test]
    async fn every_error_class_yields_a_distinct_message() {
        // The full class → message table (T023): each rendered error names its
        // class via the outcome prefix, and prefixes are pairwise distinct.
        let errors = [
            AppError::Refusal("x".into()),
            AppError::Truncation("x".into()),
            AppError::Timeout { ms: 1 },
            AppError::RetriesExhausted {
                attempts: 2,
                last: "x".into(),
            },
            AppError::InvalidInput("x".into()),
            AppError::ValidationFailure("x".into()),
            AppError::Cancelled,
        ];
        let messages: Vec<String> = errors
            .iter()
            .map(|e| to_error_data(e).message.to_string())
            .collect();
        for (error, message) in errors.iter().zip(&messages) {
            assert!(
                message.starts_with(&format!("[{}]", error.outcome().as_str())),
                "{message}"
            );
        }
        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(unique.len(), messages.len());
    }

    #[tokio::test]
    async fn success_leaves_exactly_one_success_record_with_usage() {
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(3).returning(|_, _| {
            Ok(Completion {
                value: json!({ "verdict": "supported", "findings": [] }),
                input_tokens: 100,
                output_tokens: 10,
            })
        });
        let (server, storage) = server_with(mock).await;

        let out = server
            .verify_with_ct(
                VerifyParams {
                    claim: "c".into(),
                    context: None,
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(out.0.passes, 3);

        let records = storage.list_invocations().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].outcome, Outcome::Success);
        assert_eq!(records[0].input_tokens, 300);
        assert_eq!(records[0].output_tokens, 30);
        assert!(records[0].cost_usd > 0.0);
    }

    #[tokio::test]
    async fn failure_leaves_exactly_one_record_with_its_class() {
        let mut mock = MockModelClient::new();
        mock.expect_complete()
            .returning(|_, _| Err(AppError::Refusal("declined".into())));
        let (server, storage) = server_with(mock).await;

        let Err(err) = server
            .verify_with_ct(
                VerifyParams {
                    claim: "c".into(),
                    context: None,
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .await
        else {
            panic!("expected a refusal error")
        };
        assert!(err.message.starts_with("[refusal]"), "{}", err.message);

        let records = storage.list_invocations().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].outcome, Outcome::Refusal);
    }

    #[tokio::test]
    async fn invalid_input_records_without_any_model_call() {
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);
        let (server, storage) = server_with(mock).await;

        let Err(err) = server
            .verify_with_ct(
                VerifyParams {
                    claim: "   ".into(),
                    context: None,
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .await
        else {
            panic!("expected an invalid_input error")
        };
        assert!(err.message.starts_with("[invalid_input]"));

        let records = storage.list_invocations().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].outcome, Outcome::InvalidInput);
    }

    /// First 3 passes hang for a minute (the invocation the client abandons);
    /// every later pass succeeds immediately.
    struct HangThenSucceed {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::traits::client::ModelClient for HangThenSucceed {
        async fn complete(
            &self,
            _prompt: &str,
            _schema: &serde_json::Value,
        ) -> Result<Completion, AppError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 3 {
                tokio::time::sleep(Duration::from_mins(1)).await;
                return Err(AppError::Cancelled);
            }
            Ok(Completion {
                value: json!({ "verdict": "supported", "findings": [] }),
                input_tokens: 1,
                output_tokens: 1,
            })
        }
    }

    #[tokio::test]
    async fn abandoned_invocation_records_cancelled_and_server_stays_healthy() {
        let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
        let server = Parallax::new(
            Arc::new(HangThenSucceed {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            storage.clone(),
            Arc::new(SystemClock),
            &test_config(),
        )
        .unwrap();

        // Cancel the request mid-flight (MCP cancellation / client disconnect,
        // edge case 4) — the select arm records `cancelled` deterministically.
        let ct = tokio_util::sync::CancellationToken::new();
        let cancel = ct.clone();
        let (cancelled, ()) = tokio::join!(
            server.verify_with_ct(
                VerifyParams {
                    claim: "c".into(),
                    context: None
                },
                ct
            ),
            async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                cancel.cancel();
            }
        );
        let Err(err) = cancelled else {
            panic!("expected the cancelled error")
        };
        assert!(err.message.starts_with("[cancelled]"), "{}", err.message);

        let records = storage.list_invocations().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].outcome, Outcome::Cancelled);

        // The server remains healthy for the next invocation.
        let out = server
            .verify_with_ct(
                VerifyParams {
                    claim: "again".into(),
                    context: None,
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(out.0.verdict, crate::modes::verify::VerdictKind::Supported);
        let records = storage.list_invocations().await.unwrap();
        assert_eq!(records.len(), 2);
    }
}
