//! The rmcp server handler.
//!
//! The tool surface, the distinct error surface (FR-007), and the single exit
//! point where every invocation — success, failure, or abandonment — leaves
//! exactly one record (FR-010).

use crate::checkpoint::contract::{
    CheckpointActionParams, CheckpointBatchParams, CheckpointResult, CheckpointTurnParams,
};
use crate::checkpoint::review as checkpoint_review;
use crate::checkpoint::run::{self as checkpoint_run, CheckpointDeps};
use crate::client::{BraveClient, VoyageClient};
use crate::config::Config;
use crate::deterministic::check::{self as check_tool, CheckDeps};
use crate::deterministic::contract::{CheckParams, CheckResult};
use crate::deterministic::translate as deterministic_translate;
use crate::error::{AppError, Outcome};
use crate::memory::consolidate as memory_consolidate;
use crate::memory::push::{self as memory_push, SurfaceParams, SurfaceResult};
use crate::memory::tools::{
    self as memory_tools, ForgetParams, ForgetResult, MemoryDeps, RecallParams, RecallResult,
    SaveParams, SaveResult,
};
use crate::modes::decide::{self, DecideParams, DecideResult, DECIDE_ID};
use crate::modes::diverge::{self, DivergeParams, DivergeResult, DIVERGE_ID};
use crate::modes::elicit::{self, ElicitParams, ElicitResult, ELICIT_ID};
use crate::modes::grounded_verify::{
    self, GroundedDeps, GroundedVerdict, GroundedVerifyParams, GROUNDED_VERIFY_ID,
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
use crate::traits::trajectory::FsTrajectoryReader;
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
    /// Grounded-verify dependencies — `None` when the capability is disabled
    /// (no `GROUNDED_VERIFY_ROOT`); the tool is then absent from the catalog
    /// and no file-read path exists (008 FR-001).
    grounded: Option<Arc<GroundedDeps>>,
    /// Deterministic-check dependencies — always present (pure in-process
    /// engines need no gate; FR-010).
    deterministic: Arc<CheckDeps>,
    /// Checkpoint-layer dependencies — always present; the layer is off by
    /// default because nothing invokes the tools until the sensor plane
    /// (hooks) is installed (006 FR-007).
    checkpoint: Arc<CheckpointDeps>,
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
    #[allow(clippy::too_many_lines)] // linear capability-wiring composition root
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
        diverge::register(&mut registry, config.verify_ensemble_k)?;
        decide::register(&mut registry)?;
        elicit::register(&mut registry)?;
        // Research-internal modes register unconditionally — their flat+closed
        // assertion belongs at boot whether or not the capability is on.
        pipeline::register(&mut registry)?;
        deterministic_translate::register(&mut registry)?;
        checkpoint_review::register(&mut registry)?;
        memory_consolidate::register(&mut registry)?;
        // Grounded-verify's mode registers unconditionally (its flat+closed
        // assertion belongs at boot); the tool is gated below on the root.
        grounded_verify::register(&mut registry, config.verify_ensemble_k)?;

        let verify_mode = registry
            .get(VERIFY_ID)
            .ok_or_else(|| AppError::Client("verify mode not registered at boot".to_string()))?
            .clone();
        let consolidation_mode = registry
            .get(memory_consolidate::CONSOLIDATION_MODE_ID)
            .ok_or_else(|| {
                AppError::Client("consolidation mode not registered at boot".to_string())
            })?
            .clone();
        let mode = |id: &str| -> Result<crate::modes::CorrectiveMode, AppError> {
            registry
                .get(id)
                .cloned()
                .ok_or_else(|| AppError::Client(format!("mode {id} not registered at boot")))
        };

        let checkpoint = Arc::new(CheckpointDeps {
            reader: Arc::new(FsTrajectoryReader),
            storage: Arc::clone(&storage),
            clock: Arc::clone(&clock),
            model_client: Arc::clone(&client),
            review_mode: mode(checkpoint_review::REVIEW_MODE_ID)?,
            model: config.anthropic_model.clone(),
            embedder: embedder.clone(),
            gate_extra_patterns: config.checkpoint_gate_patterns.clone(),
        });

        let memory = embedder.map(|embedder| {
            Arc::new(MemoryDeps {
                embedder,
                storage: Arc::clone(&storage),
                clock: Arc::clone(&clock),
                model_client: Arc::clone(&client),
                verify_mode: verify_mode.clone(),
                consolidation_mode: consolidation_mode.clone(),
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

        // Grounded-verify is enabled iff a source root is configured. The
        // reader canonicalizes the root once here — a missing/invalid root is a
        // loud startup error (008 FR-001/FR-004).
        let grounded = match &config.grounded_verify_root {
            Some(root) => Some(Arc::new(GroundedDeps {
                model_client: Arc::clone(&client),
                reader: Arc::new(crate::grounded::reader::SystemSourceReader::new(
                    root,
                    config.grounded_verify_max_bytes,
                )?),
                mode: mode(GROUNDED_VERIFY_ID)?,
                limits: crate::grounded::AssemblyLimits {
                    max_bytes: config.grounded_verify_max_bytes,
                    max_locators: config.grounded_verify_max_locators,
                },
                max_claim_chars: config.input_max_chars,
            })),
            None => None,
        };

        let deterministic = Arc::new(CheckDeps {
            model_client: Arc::clone(&client),
            translate_mode: mode(deterministic_translate::TRANSLATE_MODE_ID)?,
            input_max_chars: config.input_max_chars,
        });

        // Catalog honesty (FR-007): a disabled capability is absent from the
        // catalog, not present-but-erroring.
        let mut tool_router = Self::tool_router();
        if memory.is_none() {
            for name in ["save", "recall", "forget", "surface"] {
                tool_router.remove_route(name);
            }
        }
        if research.is_none() {
            tool_router.remove_route("research");
        }
        if grounded.is_none() {
            tool_router.remove_route(GROUNDED_VERIFY_ID);
        }

        Ok(Self {
            tool_router,
            client,
            storage,
            clock,
            registry: Arc::new(registry),
            memory,
            research,
            grounded,
            deterministic,
            checkpoint,
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

    /// The `diverge` tool: k stance-blind passes under distinct generative
    /// lenses, returning a deduplicated set of distinct framings.
    #[tool(
        name = "diverge",
        description = "Break out of a single framing of a problem. Runs parallel stance-blind \
        passes, each attacking the problem from a distinct angle (invert the goal, change whose \
        problem it is, shift the time horizon, deny the load-bearing assumption, reframe the \
        problem class), and returns a deduplicated set of genuinely different framings - each a \
        one-line reframing plus what it changes, labeled with the angle that produced it. Use \
        when you are anchored or tunnel-visioned and need real alternatives, not a more confident \
        version of the framing you already hold. To judge whether a claim is true use verify; to \
        commit to one next step use unstick."
    )]
    pub async fn diverge(
        &self,
        Parameters(params): Parameters<DivergeParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<DivergeResult>, ErrorData> {
        self.diverge_with_ct(params, context.ct).await
    }

    /// The `decide` tool: a single scored pass; the server picks the top option
    /// and calibrates confidence from the score margin.
    #[tool(
        name = "decide",
        description = "Choose among two or more options under tradeoffs, with the reasoning shown. \
        Applies an explicit decision methodology (weigh named criteria, trace what each option \
        causes, or reason under uncertainty), scores every option, and returns the recommended \
        option, the runner-up and why it lost, the deciding factors, the methodology used, and a \
        confidence calibrated to how close the call is. The choice is computed from the scores, \
        not asserted - never a menu handed back, never a hidden gut pick. To judge whether a \
        claim is true use verify; for one next step when you are looping use unstick; for a \
        computable comparison use check."
    )]
    pub async fn decide(
        &self,
        Parameters(params): Parameters<DecideParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<DecideResult>, ErrorData> {
        self.decide_with_ct(params, context.ct).await
    }

    /// The `elicit` tool: surface the assumed objective and governing
    /// preferences before committing — the wrong-objective corrective.
    #[tool(
        name = "elicit",
        description = "Surface the objective you're about to pursue and the preferences that \
        should govern it, before you commit - the corrective for solving the assumed problem \
        instead of the user's real one. Returns the objective a surface reading would assume, the \
        governing preferences/constraints (each traced to its signal; revealed/stored ones \
        outrank merely stated ones), and the divergence points where the assumed objective likely \
        departs from the user's actual one - the questions worth resolving first. Inference, not \
        interrogation: with little signal it says so rather than inventing preferences. When \
        memory is configured it also consults your stored verified preferences. It surfaces only \
        - it does not block or modify anything (that is the checkpoint layer)."
    )]
    pub async fn elicit(
        &self,
        Parameters(params): Parameters<ElicitParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<ElicitResult>, ErrorData> {
        self.elicit_with_ct(params, context.ct).await
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

    /// The `surface` tool: prompt-time memory push (016).
    #[tool(
        name = "surface",
        description = "Prompt-time memory push: deterministically rank the stored memories against the turn's starting prompt and surface the few relevant trusted ones (verbatim content + memory id + trust standing) as clearly-labeled advisory context - the model applies or ignores them; nothing is an instruction. Trusted memories only; at most a small capped number above a conservative relevance floor; a memory is surfaced at most once per session. No model passes - selection is pure ranking over stored data under a hard 500ms budget; on timeout or any failure the turn proceeds with nothing surfaced (fail-open). In the catalog only when the memory capability is configured. Intended to be invoked by the harness's UserPromptSubmit hook (integrations/claude-code); calling it directly behaves identically."
    )]
    pub async fn surface(
        &self,
        Parameters(params): Parameters<SurfaceParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<SurfaceResult>, ErrorData> {
        self.surface_with_ct(params, context.ct).await
    }

    async fn surface_with_ct(
        &self,
        params: SurfaceParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<SurfaceResult>, ErrorData> {
        let deps = Arc::clone(self.memory_deps()?);
        // Attribution: the embed lookup is the only metered call on this path.
        let model = deps.embedder.model_id().to_string();
        self.run_recorded("surface", model, ct, async {
            memory_push::run(&deps, &params).await
        })
        .await
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

    /// The `grounded_verify` tool: verify against machine-assembled verbatim
    /// source (008). Gated on `GROUNDED_VERIFY_ROOT`.
    #[tool(
        name = "grounded_verify",
        description = "Verify a claim against verbatim source you name. You give a claim and a set \
        of source locators (file paths or file/line ranges within the configured root); the server \
        reads that exact text and runs independent stance-blind passes over it - you cannot \
        paraphrase or bias the evidence. Returns a verdict (supported/refuted), findings citing \
        the source, a confidence from cross-pass agreement, an evidence manifest of exactly what \
        was read, and a completeness signal naming any evidence you did not provide. Use when a \
        claim must be checked against source you should not be trusted to summarize."
    )]
    pub async fn grounded_verify(
        &self,
        Parameters(params): Parameters<GroundedVerifyParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<GroundedVerdict>, ErrorData> {
        self.grounded_verify_with_ct(params, context.ct).await
    }

    /// The `check` tool: checkable claims settled by execution, not judgment.
    #[tool(
        name = "check",
        description = "Settle a checkable claim by execution, not judgment. The claim is translated into a small formal problem - an arithmetic comparison or a logic/constraint system - and a deterministic engine executes it: no judge to fool, no calibration, no sycophancy. Returns the verdict with the executed formal form and the engine's raw result so the check is auditable, plus a solver witness when one exists. Claims that cannot be formalized honestly (judgment, taste, open questions) return not_checkable with the reason - route those to verify instead."
    )]
    pub async fn check(
        &self,
        Parameters(params): Parameters<CheckParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<CheckResult>, ErrorData> {
        self.check_with_ct(params, context.ct).await
    }

    /// The `checkpoint_action` tool: the harness-triggered pre-action gate.
    #[tool(
        name = "checkpoint_action",
        description = "Pre-action checkpoint: deterministically evaluate one pending, risk-matched action against verified stored constraints before it runs. Returns hold (escalate to the user, quoting the conflicting stored memory) or silence. Decides within a hard time budget and fails open - an error or timeout never blocks the action. Never modifies the action. Intended to be invoked by the harness's pre-action hook; calling it directly behaves identically."
    )]
    pub async fn checkpoint_action(
        &self,
        Parameters(params): Parameters<CheckpointActionParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<CheckpointResult>, ErrorData> {
        self.checkpoint_action_with_ct(params, context.ct).await
    }

    /// The `checkpoint_batch` tool: harness-triggered post-batch screening.
    #[tool(
        name = "checkpoint_batch",
        description = "Post-batch checkpoint: after a completed group of tool calls, deterministically screen the recent trajectory for loops (the same normalized action repeated) and repeated failures (the same action failing consecutively). Returns a flag naming the specific repeated action and count, or silence. Pure and local - no model call, no network. A delivered flag is cooldown-suppressed at subsequent checkpoints until resolved. Fails open. Intended to be invoked by the harness's post-batch hook; calling it directly behaves identically."
    )]
    pub async fn checkpoint_batch(
        &self,
        Parameters(params): Parameters<CheckpointBatchParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<CheckpointResult>, ErrorData> {
        self.checkpoint_batch_with_ct(params, context.ct).await
    }

    /// The `checkpoint_turn` tool: harness-triggered end-of-turn review.
    #[tool(
        name = "checkpoint_turn",
        description = "End-of-turn checkpoint: deterministically mine the turn's final message and recent trajectory for candidate contradictions (against earlier committed statements and verified stored decisions) and, when memory is configured, candidate preference violations (the turn vs recalled trusted stored preferences); only when candidates exist, one independent blind review pass classifies them - a single hop judges both. A confirmed contradiction returns a flag citing both conflicting statements; a confirmed preference violation returns a flag quoting the stored preference verbatim with its provenance (memory id + trust standing) so the model can revise or explicitly contest it; both together return one combined flag. Delivered as forced continuation - the turn does not end until the model addresses it (at most once per turn). Otherwise silence. Preference enforcement never holds and never rewrites; with memory unconfigured, behavior is identical to the contradiction-only tool. With memory configured the same pass also judges capture-worthiness: a turn that produced a demonstrably working approach or a diagnosed failure may propose one candidate memory, stored quarantined (untrusted, never surfaced by push, capped per session) and promotable only through explicit re-admission or verification - capture never affects the verdict and fails open. Verdict and wording are server-assembled; the review pass never decides or phrases the verdict. Fails open. Intended to be invoked by the harness's stop hook; calling it directly behaves identically."
    )]
    pub async fn checkpoint_turn(
        &self,
        Parameters(params): Parameters<CheckpointTurnParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<CheckpointResult>, ErrorData> {
        self.checkpoint_turn_with_ct(params, context.ct).await
    }

    async fn checkpoint_action_with_ct(
        &self,
        params: CheckpointActionParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<CheckpointResult>, ErrorData> {
        let deps = Arc::clone(&self.checkpoint);
        // Attribution: the embed lookup is the only metered call on this path.
        let model = deps
            .embedder
            .as_ref()
            .map_or_else(|| self.model.clone(), |e| e.model_id().to_string());
        self.run_recorded("checkpoint_action", model, ct, async {
            checkpoint_run::run_action(&deps, &params).await
        })
        .await
    }

    async fn checkpoint_batch_with_ct(
        &self,
        params: CheckpointBatchParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<CheckpointResult>, ErrorData> {
        let deps = Arc::clone(&self.checkpoint);
        self.run_recorded("checkpoint_batch", self.model.clone(), ct, async {
            checkpoint_run::run_batch(&deps, &params).await
        })
        .await
    }

    async fn checkpoint_turn_with_ct(
        &self,
        params: CheckpointTurnParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<CheckpointResult>, ErrorData> {
        let deps = Arc::clone(&self.checkpoint);
        self.run_recorded("checkpoint_turn", self.model.clone(), ct, async {
            checkpoint_run::run_turn(&deps, &params).await
        })
        .await
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

    async fn decide_with_ct(
        &self,
        params: DecideParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<DecideResult>, ErrorData> {
        let mode = self.registry.get(DECIDE_ID).ok_or_else(|| {
            ErrorData::internal_error("decide mode not registered".to_string(), None)
        })?;
        self.run_recorded(DECIDE_ID, self.model.clone(), ct, async {
            decide::run(self.client.as_ref(), mode, &params, self.max_claim_chars)
                .await
                .map(|run| (run.result, run.input_tokens, run.output_tokens))
        })
        .await
    }

    async fn elicit_with_ct(
        &self,
        params: ElicitParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<ElicitResult>, ErrorData> {
        let mode = self.registry.get(ELICIT_ID).ok_or_else(|| {
            ErrorData::internal_error("elicit mode not registered".to_string(), None)
        })?;
        // Memory only enriches: pass it when configured, run without it otherwise.
        let memory = self.memory.as_deref();
        self.run_recorded(ELICIT_ID, self.model.clone(), ct, async {
            elicit::run(
                self.client.as_ref(),
                mode,
                memory,
                &params,
                self.max_claim_chars,
            )
            .await
            .map(|run| (run.result, run.input_tokens, run.output_tokens))
        })
        .await
    }

    async fn diverge_with_ct(
        &self,
        params: DivergeParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<DivergeResult>, ErrorData> {
        let mode = self.registry.get(DIVERGE_ID).ok_or_else(|| {
            ErrorData::internal_error("diverge mode not registered".to_string(), None)
        })?;
        self.run_recorded(DIVERGE_ID, self.model.clone(), ct, async {
            diverge::run(self.client.as_ref(), mode, &params, self.max_claim_chars)
                .await
                .map(|run| (run.result, run.input_tokens, run.output_tokens))
        })
        .await
    }

    /// The per-process session id (one stdio connection per process) —
    /// telemetry's `service.instance.id` (007 D6).
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
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

    async fn check_with_ct(
        &self,
        params: CheckParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<CheckResult>, ErrorData> {
        let deps = Arc::clone(&self.deterministic);
        // Translation is the only metered call — anthropic-model attribution.
        self.run_recorded("check", self.model.clone(), ct, async {
            check_tool::run(&deps, &params).await
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

    async fn grounded_verify_with_ct(
        &self,
        params: GroundedVerifyParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<Json<GroundedVerdict>, ErrorData> {
        let deps = Arc::clone(self.grounded.as_ref().ok_or_else(|| {
            ErrorData::internal_error(
                "grounded_verify capability is disabled (GROUNDED_VERIFY_ROOT is not configured)"
                    .to_string(),
                None,
            )
        })?);
        // The stance-blind passes dominate cost — attribute to the anthropic model.
        self.run_recorded(GROUNDED_VERIFY_ID, self.model.clone(), ct, async {
            deps.evaluate(&params)
                .await
                .map(|run| (run.verdict, run.input_tokens, run.output_tokens))
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
             Call `check` instead when the claim's truth is computable (arithmetic, \
             quantitative comparisons, logical/constraint consistency) - a deterministic \
             engine decides, not a judge. Call `unstick` when you are stuck or looping \
             and need to commit to one concrete next step.",
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
        if self.grounded.is_some() {
            instructions.push_str(
                " Call `grounded_verify` to verify a claim against verbatim source you name \
                 (file paths or line ranges) - the server reads the exact text, so you cannot \
                 paraphrase the evidence.",
            );
        }
        instructions.push_str(
            " The `checkpoint_*` tools are trajectory checkpoints triggered by the \
             harness's hooks when the checkpoint integration is installed - they are \
             not for routine self-invocation.",
        );
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
            checkpoint_gate_patterns: vec![],
            grounded_verify_root: None,
            grounded_verify_max_bytes: 262_144,
            grounded_verify_max_locators: 64,
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
        assert_eq!(
            catalog(&server),
            [
                "check",
                "checkpoint_action",
                "checkpoint_batch",
                "checkpoint_turn",
                "decide",
                "diverge",
                "elicit",
                "unstick",
                "verify"
            ]
        );
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
        assert_eq!(
            catalog(&server),
            [
                "check",
                "checkpoint_action",
                "checkpoint_batch",
                "checkpoint_turn",
                "decide",
                "diverge",
                "elicit",
                "research",
                "unstick",
                "verify"
            ]
        );
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
            [
                "check",
                "checkpoint_action",
                "checkpoint_batch",
                "checkpoint_turn",
                "decide",
                "diverge",
                "elicit",
                "forget",
                "recall",
                "save",
                "surface",
                "unstick",
                "verify"
            ]
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
            AppError::Timeout {
                what: "request",
                ms: 1,
            },
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
