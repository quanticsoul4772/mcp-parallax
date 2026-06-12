//! The rmcp server handler.
//!
//! The tool surface, the distinct error surface (FR-007), and the single exit
//! point where every invocation — success, failure, or abandonment — leaves
//! exactly one record (FR-010).

use crate::client::{BraveClient, VoyageClient};
use crate::config::Config;
use crate::error::{AppError, Outcome};
use crate::memory::tools::{
    self as memory_tools, ForgetParams, ForgetResult, MemoryDeps, RecallParams, RecallResult,
    SaveParams, SaveResult,
};
use crate::modes::unstick::{self, NextStep, UnstickParams, UNSTICK_ID};
use crate::modes::verify::{self, Verdict, VerifyParams, VERIFY_ID};
use crate::modes::ModeRegistry;
use crate::research::contract::{ResearchParams, ResearchResult};
use crate::research::fetch::{FetchPolicy, HygieneFetcher, DOMAIN_SPACING_MS};
use crate::research::pipeline::{self, ResearchDeps};
use crate::traits::client::ModelClient;
use crate::traits::clock::TimeProvider;
use crate::traits::embedder::Embedder;
use crate::traits::search::SearchProvider;
use crate::traits::storage::Storage;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorData, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use std::sync::Arc;

mod record;

use record::{to_error_data, RecordGuard};

/// The Parallax MCP server: the seams it composes plus the mode registry.
#[derive(Clone)]
pub struct Parallax {
    tool_router: ToolRouter<Self>,
    client: Arc<dyn ModelClient>,
    storage: Arc<dyn Storage>,
    clock: Arc<dyn TimeProvider>,
    registry: Arc<ModeRegistry>,
    /// Memory tool dependencies — `None` when the capability is disabled
    /// (no `VOYAGE_API_KEY`), in which case the tools are absent from the
    /// catalog too (FR-007).
    memory: Option<Arc<MemoryDeps>>,
    /// Research dependencies — `None` when the capability is disabled
    /// (no `BRAVE_API_KEY`); same catalog honesty.
    research: Option<Arc<ResearchDeps>>,
    /// Per-source fetch timeout for research runs (`FETCH_TIMEOUT_MS`).
    fetch_timeout_ms: u64,
    /// SSRF guard override for research fetches (`FETCH_ALLOW_PRIVATE`).
    fetch_allow_private: bool,
    /// Per-process session UUID (one stdio connection per process).
    session_id: String,
    model: String,
    max_claim_chars: usize,
}

#[tool_router]
impl Parallax {
    /// Compose the server from its seams. Registers all modes — an illegal
    /// mode schema fails here, at boot, not on the first call. Builds the
    /// Voyage embedder iff `VOYAGE_API_KEY` is configured (FR-007);
    /// construction never touches the network.
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
        let embedder: Option<Arc<dyn Embedder>> = match config.voyage_api_key {
            Some(_) => Some(Arc::new(VoyageClient::new(config)?)),
            None => None,
        };
        let search: Option<Arc<dyn SearchProvider>> = match config.brave_api_key {
            Some(_) => Some(Arc::new(BraveClient::new(config)?)),
            None => None,
        };
        Self::with_capabilities(client, storage, clock, config, embedder, search)
    }

    /// [`Parallax::new`] with the embedder injected and research off (003-era
    /// test constructor, kept for the existing suites).
    ///
    /// # Errors
    ///
    /// Propagates the registry's flat+closed schema assertion.
    pub fn with_embedder(
        client: Arc<dyn ModelClient>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn TimeProvider>,
        config: &Config,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> Result<Self, AppError> {
        Self::with_capabilities(client, storage, clock, config, embedder, None)
    }

    /// [`Parallax::new`] with every gated capability injected (tests pass
    /// mocks): memory is on iff `embedder` is `Some`, research iff `search`
    /// is `Some`.
    ///
    /// # Errors
    ///
    /// Propagates the registry's flat+closed schema assertion.
    pub fn with_capabilities(
        client: Arc<dyn ModelClient>,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn TimeProvider>,
        config: &Config,
        embedder: Option<Arc<dyn Embedder>>,
        search: Option<Arc<dyn SearchProvider>>,
    ) -> Result<Self, AppError> {
        let mut registry = ModeRegistry::new();
        verify::register(&mut registry, config.verify_ensemble_k)?;
        unstick::register(&mut registry)?;
        // Research-internal modes register unconditionally — their flat+closed
        // assertion belongs at boot whether or not the capability is on.
        pipeline::register(&mut registry)?;

        let verify_mode = registry
            .get(VERIFY_ID)
            .ok_or_else(|| AppError::Client("verify mode not registered at boot".to_string()))?
            .clone();
        let mode = |id: &str| -> Result<crate::modes::CorrectiveMode, AppError> {
            registry
                .get(id)
                .cloned()
                .ok_or_else(|| AppError::Client(format!("mode {id} not registered at boot")))
        };

        let memory = embedder.map(|embedder| {
            Arc::new(MemoryDeps {
                embedder,
                storage: Arc::clone(&storage),
                clock: Arc::clone(&clock),
                model_client: Arc::clone(&client),
                verify_mode: verify_mode.clone(),
                input_max_chars: config.input_max_chars,
                default_recall_limit: config.memory_recall_limit,
            })
        });

        let research = match search {
            Some(search) => Some(Arc::new(ResearchDeps {
                model_client: Arc::clone(&client),
                search,
                clock: Arc::clone(&clock),
                scope_mode: mode(pipeline::SCOPE_MODE_ID)?,
                extract_mode: mode(pipeline::EXTRACT_MODE_ID)?,
                synth_mode: mode(pipeline::SYNTH_MODE_ID)?,
                verify_mode: pipeline::research_verify_mode(&verify_mode),
                input_max_chars: config.input_max_chars,
                concurrency: usize::from(config.research_concurrency),
            })),
            None => None,
        };

        // Catalog honesty (FR-007): a disabled capability is absent from the
        // catalog, not present-but-erroring.
        let mut tool_router = Self::tool_router();
        if memory.is_none() {
            for name in ["save", "recall", "forget"] {
                tool_router.remove_route(name);
            }
        }
        if research.is_none() {
            tool_router.remove_route("research");
        }

        Ok(Self {
            tool_router,
            client,
            storage,
            clock,
            registry: Arc::new(registry),
            memory,
            research,
            fetch_timeout_ms: config.fetch_timeout_ms,
            fetch_allow_private: config.fetch_allow_private,
            session_id: uuid::Uuid::new_v4().to_string(),
            model: config.anthropic_model.clone(),
            max_claim_chars: config.input_max_chars,
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

    /// The `unstick` tool: one committed next step for a stuck caller.
    #[tool(
        name = "unstick",
        description = "Break a stuck loop by committing to one concrete next step. Call when you \
        have a goal, you have tried things, and you are producing plausible motion that goes \
        nowhere. Provide the goal, where you are blocked, and what you already tried; you get \
        back exactly one immediately actionable step with a rationale - never a menu of options, \
        never a plan. An external frame breaks the loop you cannot see from inside."
    )]
    pub async fn unstick(
        &self,
        Parameters(params): Parameters<UnstickParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<NextStep>, ErrorData> {
        self.unstick_with_ct(params, context.ct).await
    }

    /// The `save` tool: store one memory with derived trust.
    #[tool(
        name = "save",
        description = "Save a memory for future sessions: a skill (reusable approach that \
        worked), a lesson (what failed and why), or a fact (durable knowledge). Provide \
        provenance: where this came from and whether it is first-hand or from external content. \
        External memories are stored untrusted unless you request verification; verified or \
        first-hand memories rank higher at recall."
    )]
    pub async fn save(
        &self,
        Parameters(params): Parameters<SaveParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<SaveResult>, ErrorData> {
        self.save_with_ct(params, context.ct).await
    }

    /// The `recall` tool: semantically relevant memories for a query.
    #[tool(
        name = "recall",
        description = "Recall saved memories relevant to what you are working on. Describe what \
        you need in natural language; returns the most relevant skills, lessons, and facts from \
        prior sessions, ranked, each labeled with its provenance and trust standing. Call before \
        re-deriving something that may already be solved."
    )]
    pub async fn recall(
        &self,
        Parameters(params): Parameters<RecallParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<RecallResult>, ErrorData> {
        self.recall_with_ct(params, context.ct).await
    }

    /// The `forget` tool: permanently delete one memory by id.
    #[tool(
        name = "forget",
        description = "Permanently delete a saved memory by id. Use when a memory is wrong, \
        stale, or should not be retained. Deletion is irreversible; the memory will never \
        appear in recall again."
    )]
    pub async fn forget(
        &self,
        Parameters(params): Parameters<ForgetParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<ForgetResult>, ErrorData> {
        self.forget_with_ct(params, context.ct).await
    }

    /// The `research` tool: offloaded, cited, adversarially-verified answers.
    #[tool(
        name = "research",
        description = "Offload a research question; get back a short, cited, \
        adversarially-verified answer - not a pile of links. Runs scoped parallel searches, \
        fetches and extracts sources, verifies every claim with independent refute-biased \
        passes, and synthesizes a compact answer with inline citations, surfaced \
        disagreements, and honest gaps. Depth scales rigor; budget and deadline ceilings \
        synthesize early and say so. Every citation resolves to a source fetched during the \
        run."
    )]
    pub async fn research(
        &self,
        Parameters(params): Parameters<ResearchParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<ResearchResult>, ErrorData> {
        self.research_with_ct(params, context.ct).await
    }

    async fn verify_with_ct(
        &self,
        params: VerifyParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<Verdict>, ErrorData> {
        let mode = self.registry.get(VERIFY_ID).ok_or_else(|| {
            ErrorData::internal_error("verify mode not registered".to_string(), None)
        })?;
        self.run_recorded(VERIFY_ID, self.model.clone(), ct, async {
            verify::run(self.client.as_ref(), mode, &params, self.max_claim_chars)
                .await
                .map(|run| (run.verdict, run.input_tokens, run.output_tokens))
        })
        .await
    }

    async fn unstick_with_ct(
        &self,
        params: UnstickParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<NextStep>, ErrorData> {
        let mode = self.registry.get(UNSTICK_ID).ok_or_else(|| {
            ErrorData::internal_error("unstick mode not registered".to_string(), None)
        })?;
        self.run_recorded(UNSTICK_ID, self.model.clone(), ct, async {
            unstick::run(self.client.as_ref(), mode, &params, self.max_claim_chars)
                .await
                .map(|run| (run.step, run.input_tokens, run.output_tokens))
        })
        .await
    }

    /// The memory deps, or the internal error for a tool that should not have
    /// been reachable — the catalog omits memory tools when disabled (FR-007),
    /// so a call without deps is a client ignoring the catalog.
    fn memory_deps(&self) -> Result<&Arc<MemoryDeps>, ErrorData> {
        self.memory.as_ref().ok_or_else(|| {
            ErrorData::internal_error(
                "memory capability is disabled (VOYAGE_API_KEY is not configured)".to_string(),
                None,
            )
        })
    }

    async fn save_with_ct(
        &self,
        params: SaveParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<SaveResult>, ErrorData> {
        let deps = Arc::clone(self.memory_deps()?);
        // Model attribution is known up front: a verifying save is dominated
        // by the verify ensemble's model; otherwise only the embedder runs.
        let model = if memory_tools::save_runs_verification(&params) {
            self.model.clone()
        } else {
            deps.embedder.model_id().to_string()
        };
        self.run_recorded("save", model, ct, async {
            memory_tools::save(&deps, &params).await
        })
        .await
    }

    async fn recall_with_ct(
        &self,
        params: RecallParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<RecallResult>, ErrorData> {
        let deps = Arc::clone(self.memory_deps()?);
        let model = deps.embedder.model_id().to_string();
        self.run_recorded("recall", model, ct, async {
            memory_tools::recall(&deps, &params).await
        })
        .await
    }

    async fn research_with_ct(
        &self,
        params: ResearchParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<ResearchResult>, ErrorData> {
        let deps = Arc::clone(self.research.as_ref().ok_or_else(|| {
            ErrorData::internal_error(
                "research capability is disabled (BRAVE_API_KEY is not configured)".to_string(),
                None,
            )
        })?);
        // The fetcher is per-run: the robots cache is run-scoped and the
        // allow/deny lists come from the call's constraints (research.md D5).
        let constraints = params.constraints.clone().unwrap_or_default();
        let policy = FetchPolicy {
            timeout_ms: self.fetch_timeout_ms,
            domains_allow: constraints.domains_allow.unwrap_or_default(),
            domains_deny: constraints.domains_deny.unwrap_or_default(),
            domain_spacing_ms: DOMAIN_SPACING_MS,
            allow_private: self.fetch_allow_private,
        };
        // LLM calls dominate research cost; Brave bills per-request, not
        // per-token — attribute the record to the anthropic model (plan.md).
        self.run_recorded("research", self.model.clone(), ct, async {
            let fetcher = HygieneFetcher::new(policy)?;
            pipeline::run(&deps, &fetcher, &params).await
        })
        .await
    }

    async fn forget_with_ct(
        &self,
        params: ForgetParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<ForgetResult>, ErrorData> {
        let deps = Arc::clone(self.memory_deps()?);
        // No provider call is involved; attribute to the embedder model the
        // capability is keyed on.
        let model = deps.embedder.model_id().to_string();
        self.run_recorded("forget", model, ct, async {
            memory_tools::forget(&deps, &params).await
        })
        .await
    }

    /// The shared per-invocation wrapper: single exit point where every path —
    /// success, each failure class, cancellation (ct select arm), abandonment
    /// (guard Drop backstop) — leaves exactly one record (FR-010).
    async fn run_recorded<T, Fut>(
        &self,
        tool_id: &'static str,
        model: String,
        ct: tokio_util::sync::CancellationToken,
        work: Fut,
    ) -> Result<Json<T>, ErrorData>
    where
        Fut: std::future::Future<Output = Result<(T, u64, u64), AppError>>,
    {
        let guard = RecordGuard::new(
            Arc::clone(&self.storage),
            Arc::clone(&self.clock),
            self.session_id.clone(),
            tool_id.to_string(),
            model,
        );

        tokio::select! {
            () = ct.cancelled() => {
                guard.finish(0, 0, Outcome::Cancelled).await;
                Err(to_error_data(&AppError::Cancelled))
            }
            result = work => {
                match result {
                    Ok((value, input_tokens, output_tokens)) => {
                        guard.finish(input_tokens, output_tokens, Outcome::Success).await;
                        Ok(Json(value))
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

// The router expression must be the instance field — the macro default
// (`Self::tool_router()`) would rebuild the full, ungated router per call and
// silently undo the capability gating done at construction.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for Parallax {
    fn get_info(&self) -> ServerInfo {
        let mut instructions = String::from(
            "Parallax: independent correctives for the calling model's blind spots. \
             Call `verify` when an assertion matters and being confidently wrong is costly. \
             Call `unstick` when you are stuck or looping and need to commit to one \
             concrete next step.",
        );
        if self.memory.is_some() {
            instructions.push_str(
                " Call `recall` before re-deriving prior work, `save` to keep a skill, \
                 lesson, or fact for future sessions, and `forget` to delete a stored \
                 memory by id.",
            );
        }
        if self.research.is_some() {
            instructions.push_str(
                " Call `research` to offload a web research question to a separate \
                 budget and get back a short, cited, adversarially-verified answer.",
            );
        }
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(instructions)
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
            input_max_chars: 50_000,
            voyage_api_key: None,
            voyage_model: "voyage-4".into(),
            memory_recall_limit: 5,
            brave_api_key: None,
            fetch_timeout_ms: 10_000,
            research_concurrency: 8,
            fetch_allow_private: false,
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

    fn catalog(server: &Parallax) -> Vec<String> {
        let mut names: Vec<String> = server
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn without_capability_keys_the_catalog_is_exactly_the_correctives() {
        let (server, _) = server_with(MockModelClient::new()).await;
        assert_eq!(catalog(&server), ["unstick", "verify"]);
        assert!(server.memory.is_none());
        assert!(server.research.is_none());
        // The instructions don't advertise tools that aren't there.
        let info = server.get_info();
        let instructions = info.instructions.unwrap();
        assert!(!instructions.contains("recall"));
        assert!(!instructions.contains("research"));
    }

    #[tokio::test]
    async fn with_a_search_provider_the_research_tool_joins_the_catalog() {
        let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
        let search = crate::traits::search::MockSearchProvider::new();
        let server = Parallax::with_capabilities(
            Arc::new(MockModelClient::new()),
            storage,
            Arc::new(SystemClock),
            &test_config(),
            None,
            Some(Arc::new(search)),
        )
        .unwrap();
        assert_eq!(catalog(&server), ["research", "unstick", "verify"]);
        let info = server.get_info();
        assert!(info.instructions.unwrap().contains("research"));
    }

    #[tokio::test]
    async fn research_records_one_invocation_attributed_to_the_anthropic_model() {
        // Scope fails with a refusal — the cheapest full path through
        // run_recorded; the record carries the class and the model.
        let mut client = MockModelClient::new();
        client
            .expect_complete()
            .returning(|_, _| Err(AppError::Refusal("declined".into())));
        let mut search = crate::traits::search::MockSearchProvider::new();
        search.expect_search().times(0);
        let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
        let server = Parallax::with_capabilities(
            Arc::new(client),
            storage.clone(),
            Arc::new(SystemClock),
            &test_config(),
            None,
            Some(Arc::new(search)),
        )
        .unwrap();

        let Err(err) = server
            .research_with_ct(
                ResearchParams {
                    question: "q?".into(),
                    depth: None,
                    focus: None,
                    constraints: None,
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .await
        else {
            panic!("expected the refusal to surface")
        };
        assert!(err.message.starts_with("[refusal]"), "{}", err.message);

        let records = storage.list_invocations().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tool, "research");
        assert_eq!(records[0].model, "claude-opus-4-8");
        assert_eq!(records[0].outcome, Outcome::Refusal);
    }

    #[tokio::test]
    async fn with_an_embedder_the_memory_tools_join_the_catalog() {
        let mut embedder = crate::traits::embedder::MockEmbedder::new();
        embedder
            .expect_model_id()
            .return_const("voyage-4".to_string());
        let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
        let server = Parallax::with_embedder(
            Arc::new(MockModelClient::new()),
            storage,
            Arc::new(SystemClock),
            &test_config(),
            Some(Arc::new(embedder)),
        )
        .unwrap();
        assert_eq!(
            catalog(&server),
            ["forget", "recall", "save", "unstick", "verify"]
        );
        let info = server.get_info();
        assert!(info.instructions.unwrap().contains("recall"));
    }

    #[tokio::test]
    async fn memory_tools_record_with_the_attributed_model() {
        let mut embedder = crate::traits::embedder::MockEmbedder::new();
        embedder
            .expect_model_id()
            .return_const("voyage-4".to_string());
        embedder.expect_embed_document().returning(|_| {
            Ok(crate::traits::embedder::Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 9,
            })
        });
        embedder.expect_embed_query().returning(|_| {
            Ok(crate::traits::embedder::Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 4,
            })
        });
        // The verifying save runs the ensemble (k = 3 in test_config).
        let mut client = MockModelClient::new();
        client.expect_complete().times(3).returning(|_, _| {
            Ok(Completion {
                value: json!({ "verdict": "supported", "findings": [] }),
                input_tokens: 100,
                output_tokens: 10,
            })
        });
        let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
        let server = Parallax::with_embedder(
            Arc::new(client),
            storage.clone(),
            Arc::new(SystemClock),
            &test_config(),
            Some(Arc::new(embedder)),
        )
        .unwrap();
        let ct = tokio_util::sync::CancellationToken::new;

        // Plain save: only the embedder ran — attributed to the voyage model.
        let saved = server
            .save_with_ct(
                SaveParams {
                    content: "first-hand lesson".into(),
                    kind: crate::memory::Kind::Lesson,
                    origin: "this session".into(),
                    external: false,
                    tags: None,
                    verify: None,
                },
                ct(),
            )
            .await
            .unwrap();

        // Verifying save: the ensemble dominates — attributed to the
        // anthropic model.
        server
            .save_with_ct(
                SaveParams {
                    content: "external claim".into(),
                    kind: crate::memory::Kind::Fact,
                    origin: "the web".into(),
                    external: true,
                    tags: None,
                    verify: Some(true),
                },
                ct(),
            )
            .await
            .unwrap();

        let recalled = server
            .recall_with_ct(
                RecallParams {
                    query: "lesson".into(),
                    kind: None,
                    limit: None,
                },
                ct(),
            )
            .await
            .unwrap();
        assert_eq!(recalled.0.memories.len(), 2);

        server
            .forget_with_ct(
                ForgetParams {
                    id: saved.0.id.clone(),
                },
                ct(),
            )
            .await
            .unwrap();

        let records = storage.list_invocations().await.unwrap();
        assert_eq!(records.len(), 4);
        let model_for = |tool: &str| -> Vec<&str> {
            records
                .iter()
                .filter(|r| r.tool == tool)
                .map(|r| r.model.as_str())
                .collect()
        };
        // Both saves recorded; the verifying one carries the anthropic model.
        let mut save_models = model_for("save");
        save_models.sort_unstable();
        assert_eq!(save_models, ["claude-opus-4-8", "voyage-4"]);
        assert_eq!(model_for("recall"), ["voyage-4"]);
        assert_eq!(model_for("forget"), ["voyage-4"]);
        // Every record succeeded.
        assert!(records.iter().all(|r| r.outcome == Outcome::Success));
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
