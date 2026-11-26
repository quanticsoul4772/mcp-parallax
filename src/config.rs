//! Runtime configuration, sourced entirely from the environment.

use crate::error::ConfigError;
use crate::routing::{DefaultVar, RoutingTable};

/// Default model when `ANTHROPIC_MODEL` is unset — the design corpus's stated
/// target (structured outputs GA).
pub const DEFAULT_MODEL: &str = "claude-opus-4-8";

/// Default OpenAI-compatible API base URL when `OPENAI_API_BASE` is unset.
///
/// `/v1`-suffixed by convention: OpenAI, Azure's OpenAI-compatible surface,
/// Ollama, vLLM and LM Studio all serve `/chat/completions` under it.
pub const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com/v1";

/// Default embedding model when `VOYAGE_MODEL` is unset. The voyage-4 family
/// shares one embedding space, so switching within the family needs no
/// re-index.
pub const DEFAULT_VOYAGE_MODEL: &str = "voyage-4";

/// Default tracing filter when `LOG_LEVEL` is unset.
///
/// Named rather than written twice: `main.rs` installs the subscriber before
/// this module's `Config` exists, so it needs the same default independently.
/// Two literals would drift with every test still green — the defect 040 was
/// written to remove, between two code sites rather than code and a document.
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// Server-side ceiling on recall result counts.
pub const MEMORY_RECALL_LIMIT_MAX: u8 = 20;

/// Default `EXA_MAX_AGE_HOURS`: one week (059 research.md D3).
pub const EXA_MAX_AGE_HOURS_DEFAULT: i32 = 168;

/// Server-side ceiling on research concurrency.
pub const RESEARCH_CONCURRENCY_MAX: u8 = 32;

/// Default Claude Code CLI to run on the `claude_cli` backend (060):
/// the command name, resolved on the operator's path. `CLAUDE_CLI_PATH`.
pub const DEFAULT_CLAUDE_CLI_PATH: &str = "claude";

/// Default model on the `claude_cli` backend (060 research.md D4).
/// `CLAUDE_CLI_MODEL`.
pub const DEFAULT_CLAUDE_CLI_MODEL: &str = "claude-sonnet-5-5";

/// Default cap on live Claude Code CLI processes, server-wide (060 D5).
/// `CLAUDE_CLI_CONCURRENCY`.
pub const DEFAULT_CLAUDE_CLI_CONCURRENCY: u8 = 4;

/// Upper bound for `CLAUDE_CLI_CONCURRENCY`.
pub const CLAUDE_CLI_CONCURRENCY_MAX: u8 = 32;

/// Default total assembled-evidence ceiling for grounded-verify, in bytes
/// (256 KiB). `GROUNDED_VERIFY_MAX_BYTES`.
pub const DEFAULT_GROUNDED_VERIFY_MAX_BYTES: usize = 262_144;

/// Default maximum locators accepted in one grounded-verify call.
/// `GROUNDED_VERIFY_MAX_LOCATORS`.
pub const DEFAULT_GROUNDED_VERIFY_MAX_LOCATORS: usize = 64;

/// The pinned Jev model (064): a versioned id, never a moving alias, so
/// recorded answers and later thresholds stay comparable.
/// `PARALLAX_JEV_MODEL`.
pub const DEFAULT_JEV_MODEL: &str = "jev-1.14.0";

/// The reserved Jev mode namespace (064): `PARALLAX_JEV_MODE_<SITE>`.
const JEV_MODE_PREFIX: &str = "PARALLAX_JEV_MODE_";

/// Jev settings (064). Every site is off unless the operator sets its mode,
/// and a mode needs `TYPESAFE_API_KEY`; the key alone enables nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JevSettings {
    /// `TYPESAFE_API_KEY`; a blank value counts as unset. Never logged.
    pub api_key: Option<String>,
    /// `PARALLAX_JEV_MODEL`, default [`DEFAULT_JEV_MODEL`]. A
    /// `JEV_MODEL` variable is never read.
    pub model: String,
    /// `PARALLAX_JEV_MODE_TURN_REVIEW`, default off.
    pub turn_review: crate::traits::classifier::JevMode,
    /// `PARALLAX_JEV_MODE_RESEARCH_CLAIMS`, default off.
    pub research_claims: crate::traits::classifier::JevMode,
    /// `PARALLAX_JEV_MODE_RESEARCH_INJECTION`, default off; the only site
    /// that accepts `on`, and only with a pinned threshold (065).
    pub research_injection: crate::traits::classifier::JevMode,
}

impl JevSettings {
    /// Read the Jev settings from the process environment.
    ///
    /// # Errors
    ///
    /// As [`Self::resolve`].
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::resolve(std::env::vars())
    }

    /// Resolve from an arbitrary variable set — the testable form. Pure: no
    /// process environment, no I/O.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Jev`] naming the variable when a `PARALLAX_JEV_MODE_*`
    /// suffix is unknown, its value isn't `off`, `shadow` or `on`, `on` is
    /// set for a site other than `RESEARCH_INJECTION` or for a model with no
    /// pinned injection threshold, or a site is not `off` without a non-blank
    /// `TYPESAFE_API_KEY`.
    pub fn resolve<I>(vars: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        use crate::traits::classifier::{JevMode, JevSite};

        let mut vars: Vec<(String, String)> = vars.into_iter().collect();
        // Deterministic error ordering, independent of environment order.
        vars.sort_by(|(a, _), (b, _)| a.cmp(b));
        let lookup = |name: &str| {
            vars.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };

        let api_key = present_key(lookup("TYPESAFE_API_KEY"));
        // A blank value counts as unset, like the key (064 review F).
        let model = present_key(lookup("PARALLAX_JEV_MODEL"))
            .map_or_else(|| DEFAULT_JEV_MODEL.to_string(), |m| m.trim().to_string());

        let mut settings = Self {
            api_key,
            model,
            turn_review: JevMode::Off,
            research_claims: JevMode::Off,
            research_injection: JevMode::Off,
        };
        for (name, value) in vars
            .iter()
            .filter(|(name, _)| name.starts_with(JEV_MODE_PREFIX))
        {
            let suffix = &name[JEV_MODE_PREFIX.len()..];
            let site = JevSite::ALL
                .into_iter()
                .find(|site| site.suffix() == suffix)
                .ok_or_else(|| {
                    ConfigError::Jev(format!(
                        "unknown variable `{name}` in the reserved `{JEV_MODE_PREFIX}*` \
                         namespace — the sites are TURN_REVIEW, RESEARCH_CLAIMS and RESEARCH_INJECTION"
                    ))
                })?;
            let mode = JevMode::parse(value).ok_or_else(|| {
                ConfigError::Jev(format!(
                    "`{name}` must be `off`, `shadow` or `on` (got `{}`)",
                    value.trim()
                ))
            })?;
            if mode == JevMode::On && site != JevSite::ResearchInjection {
                return Err(ConfigError::Jev(format!(
                    "`{name}` is `on`, but only RESEARCH_INJECTION has a measured threshold; \
                     use `off` or `shadow`"
                )));
            }
            if mode != JevMode::Off && settings.api_key.is_none() {
                return Err(ConfigError::Jev(format!(
                    "`{name}` is `{}` but TYPESAFE_API_KEY is not set",
                    mode.as_str()
                )));
            }
            match site {
                JevSite::TurnReview => settings.turn_review = mode,
                JevSite::ResearchClaims => settings.research_claims = mode,
                JevSite::ResearchInjection => settings.research_injection = mode,
            }
        }
        if settings.research_injection == JevMode::On
            && crate::jev::policy::injection_threshold(&settings.model).is_none()
        {
            return Err(ConfigError::Jev(format!(
                "`PARALLAX_JEV_MODE_RESEARCH_INJECTION` is `on`, but `PARALLAX_JEV_MODEL` \
                 (`{}`) has no pinned injection threshold; use {DEFAULT_JEV_MODEL} or \
                 `shadow`",
                settings.model
            )));
        }
        // 064 review D: a site in shadow is costed per request, so its model
        // must have a price row; the fallback rate would overstate it ~120x.
        let shadowed = settings.turn_review != JevMode::Off
            || settings.research_claims != JevMode::Off
            || settings.research_injection != JevMode::Off;
        if shadowed && !crate::telemetry::pricing_known(&settings.model) {
            return Err(ConfigError::Jev(format!(
                "`PARALLAX_JEV_MODEL` is `{}`, which has no price row; Jev costs would be \
                 recorded at the fallback rate. Use {DEFAULT_JEV_MODEL} or add the row.",
                settings.model
            )));
        }
        Ok(settings)
    }
}

/// The model-backend family a [`Config`] selects (`PARALLAX_BACKEND`).
///
/// BYOM (design §3.4): the backend is chosen once at startup; per-call-site
/// model *routing* stays orthogonal to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The native Anthropic Messages wire format — the default, and the only
    /// backend before BYOM.
    Anthropic,
    /// OpenAI Chat Completions: OpenAI, Azure's OpenAI-compatible surface,
    /// Ollama, vLLM, LM Studio.
    OpenAiCompat,
    /// The operator's own signed-in Claude Code CLI, one isolated `claude -p`
    /// process per model call (060); no API key.
    ClaudeCli,
}

impl Backend {
    /// Every backend, for validation and reporting.
    pub const ALL: [Self; 3] = [Self::Anthropic, Self::OpenAiCompat, Self::ClaudeCli];

    /// The operator-facing spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAiCompat => "openai_compat",
            Self::ClaudeCli => "claude_cli",
        }
    }

    /// Parse an operator-supplied value, case-insensitively.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_lowercase();
        Self::ALL.into_iter().find(|b| b.as_str() == value)
    }
}

/// The structured-output strategy for the `openai_compat` backend — the design
/// §3.2 ladder, pin-able per deployment (`OPENAI_STRUCTURED_OUTPUT`).
///
/// `auto` walks the ladder, degrading one rung when the endpoint rejects the
/// strategy parameter itself; a pin selects exactly one rung, which is the
/// remedy when a server silently ignores a parameter instead of rejecting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredOutput {
    /// The full ladder: `json_schema` → `json_object` → `tool_shim` →
    /// `prompt_only`, degrading on a capability rejection.
    Auto,
    /// `response_format: {type: "json_schema", ...}` (structured outputs).
    JsonSchema,
    /// `response_format: {type: "json_object"}` with the schema in the prompt.
    JsonObject,
    /// A forced single-function tool call; `function.arguments` carries the
    /// JSON.
    ToolShim,
    /// The schema in the prompt only. Strict parse; failures are loud.
    PromptOnly,
}

impl StructuredOutput {
    /// Parse an operator-supplied value, case-insensitively.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "json_schema" => Some(Self::JsonSchema),
            "json_object" => Some(Self::JsonObject),
            "tool_shim" => Some(Self::ToolShim),
            "prompt_only" => Some(Self::PromptOnly),
            _ => None,
        }
    }

    /// The operator-facing spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::JsonSchema => "json_schema",
            Self::JsonObject => "json_object",
            Self::ToolShim => "tool_shim",
            Self::PromptOnly => "prompt_only",
        }
    }
}

/// Server configuration. Every field is sourced from an environment variable so
/// the binary is configured the same way in every host (Claude Code / Desktop).
#[derive(Debug, Clone)]
pub struct Config {
    /// Model backend family. `PARALLAX_BACKEND`, default `anthropic`.
    pub backend: Backend,
    /// Anthropic API key — required iff `backend` is [`Backend::Anthropic`],
    /// empty otherwise. `ANTHROPIC_API_KEY`.
    pub anthropic_api_key: String,
    /// Model id for verification passes. `ANTHROPIC_MODEL`, default
    /// [`DEFAULT_MODEL`]. The fall-through for every call site that no
    /// `PARALLAX_MODEL_*` setting routes elsewhere.
    pub anthropic_model: String,
    /// Anthropic API base URL. `ANTHROPIC_API_BASE`, default
    /// `https://api.anthropic.com`.
    ///
    /// Exists so the whole client pool — including the per-effort variants 028
    /// builds — can be pointed at a test double. Before this, only the single
    /// injected client could be redirected, so any call carrying an effort
    /// left the `ModelClient` seam and reached the live endpoint from inside
    /// the test suite (Principle IV).
    pub anthropic_api_base: String,
    /// OpenAI-compatible API key — required iff `backend` is
    /// [`Backend::OpenAiCompat`] and `openai_api_base` is the default OpenAI
    /// endpoint. A custom (keyless) endpoint leaves it empty and the client
    /// sends no `Authorization` header. `OPENAI_API_KEY`.
    pub openai_api_key: String,
    /// OpenAI-compatible API base URL (`/v1`-suffixed by convention).
    /// `OPENAI_API_BASE`, default [`DEFAULT_OPENAI_API_BASE`].
    pub openai_api_base: String,
    /// Model id for the `openai_compat` backend — required iff that backend is
    /// selected, empty otherwise. Model names are provider-specific, so there
    /// is no default worth guessing. `OPENAI_MODEL`.
    pub openai_model: String,
    /// Structured-output strategy for the `openai_compat` backend.
    /// `OPENAI_STRUCTURED_OUTPUT`, default `auto`.
    pub openai_structured_output: StructuredOutput,
    /// Per-call-site model routing (018). Resolved from the reserved
    /// `PARALLAX_MODEL_*` namespace over [`Self::anthropic_model`]; with
    /// nothing set every call site resolves to that default.
    pub routing: RoutingTable,
    /// Parallel verification passes per Verify invocation. `VERIFY_ENSEMBLE_K`,
    /// default `3`; must be ≥ 1.
    pub verify_ensemble_k: u8,
    /// Generic per-tool input bound in characters. `INPUT_MAX_CHARS`
    /// (default `50000`); the legacy `VERIFY_MAX_CLAIM_CHARS` is honored as a
    /// fallback alias when the new variable is unset.
    pub input_max_chars: usize,
    /// Voyage API key. **Optional — its presence enables the memory
    /// capability** (`save`/`recall`/`forget`); absent, no memory tools exist
    /// and no Voyage connection is ever made. `VOYAGE_API_KEY`.
    pub voyage_api_key: Option<String>,
    /// Embedding model. `VOYAGE_MODEL`, default [`DEFAULT_VOYAGE_MODEL`].
    pub voyage_model: String,
    /// Default recall result count. `MEMORY_RECALL_LIMIT`, default `5`;
    /// must be in `1..=20`.
    pub memory_recall_limit: u8,
    /// Exa API key. **Optional — its presence enables the research
    /// capability** (059); absent, no research egress is made. `EXA_API_KEY`.
    pub exa_api_key: Option<String>,
    /// Freshness limit for research page text, in hours. `EXA_MAX_AGE_HOURS`,
    /// default `168`; must be in `-1..=720` (`0` always live, `-1` cache only).
    pub exa_max_age_hours: i32,
    /// Concurrent extract/verify cap for research runs.
    /// `RESEARCH_CONCURRENCY`, default `8`; must be in `1..=32`.
    pub research_concurrency: u8,
    /// The Claude Code CLI the `claude_cli` backend runs (060).
    /// `CLAUDE_CLI_PATH`, default [`DEFAULT_CLAUDE_CLI_PATH`].
    pub claude_cli_path: String,
    /// The `claude_cli` backend's default model (060 D4). `CLAUDE_CLI_MODEL`,
    /// default [`DEFAULT_CLAUDE_CLI_MODEL`].
    pub claude_cli_model: String,
    /// Live Claude Code CLI processes allowed at once, server-wide (060 D5).
    /// `CLAUDE_CLI_CONCURRENCY`, default [`DEFAULT_CLAUDE_CLI_CONCURRENCY`];
    /// must be in `1..=32`.
    pub claude_cli_concurrency: u8,
    /// Extra pre-action gate risk patterns (checkpoint layer, FR-013) —
    /// comma-separated substrings extending the built-in set.
    /// `CHECKPOINT_GATE_PATTERNS`, default empty. A present value with an
    /// empty entry (`"a,,b"`) is an error, never silently skipped.
    pub checkpoint_gate_patterns: Vec<String>,
    /// The single source root for the `grounded_verify` tool (008). **Optional
    /// — its presence enables the tool**; absent, the tool does not exist and
    /// no file-read path is ever taken. `GROUNDED_VERIFY_ROOT`.
    pub grounded_verify_root: Option<String>,
    /// Total assembled-evidence byte ceiling for one `grounded_verify` call.
    /// `GROUNDED_VERIFY_MAX_BYTES`, default `262144` (256 KiB).
    pub grounded_verify_max_bytes: usize,
    /// Maximum locators accepted in one `grounded_verify` call.
    /// `GROUNDED_VERIFY_MAX_LOCATORS`, default `64`.
    pub grounded_verify_max_locators: usize,
    /// Path to the SQLite database file. `DATABASE_PATH`, default `./data/parallax.db`.
    pub database_path: String,
    /// Log-level filter. `LOG_LEVEL`, default `info`.
    pub log_level: String,
    /// Per-request timeout in milliseconds. `REQUEST_TIMEOUT_MS`, default
    /// `120000` — raised from 30 s by 018 D7 in step with the output budget,
    /// since a model that reasons before answering can exceed 30 s on a large
    /// ceiling, converting a truncation into a timeout rather than fixing it.
    pub request_timeout_ms: u64,
    /// Maximum API retry attempts. `MAX_RETRIES`, default `3`.
    pub max_retries: u32,
    /// Jev settings (064); every site off by default.
    pub jev: JevSettings,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingRequired`] if a key the selected backend
    /// requires is unset or empty (`ANTHROPIC_API_KEY` for `anthropic`;
    /// `OPENAI_MODEL`, plus `OPENAI_API_KEY` when the calls target the default
    /// OpenAI endpoint (a custom endpoint may run keyless), for
    /// `openai_compat`), and
    /// [`ConfigError::Invalid`] if a variable is present but fails to parse or
    /// violates its bounds (`VERIFY_ENSEMBLE_K` ≥ 1, `MEMORY_RECALL_LIMIT` in
    /// 1..=20). A present-but-invalid value is an error, never a silent
    /// default.
    ///
    /// # Adding a variable
    ///
    /// Every default written here is read back out of this file's source and
    /// compared against `--help` and the README table (040). Two consequences
    /// worth knowing before the build tells you:
    ///
    /// - **Both documents must state the new default**, or the suite fails
    ///   naming the variable.
    /// - **A named constant used as a default must live in a file
    ///   `config_facts::SOURCES` reads.** A `const` declared anywhere else
    ///   fails as `CONSTANT_NOT_FOUND` until that file is added there. That
    ///   list is deliberately not copied here — it is the authority, and the
    ///   failure prints its current contents.
    ///
    /// That list is enumerated rather than a crate-wide search on purpose: one
    /// constant name may be declared in several modules, and a search would
    /// silently compare a document against whichever declaration it reached
    /// first. Prefer declaring the constant in this file over widening the
    /// list; qualify the path (`crate::client::anthropic::NAME`) when it must
    /// live elsewhere, which is what selects the file to read.
    #[allow(clippy::too_many_lines)] // the environment-reading composition root reads best unbroken
    pub fn from_env() -> Result<Self, ConfigError> {
        // BYOM (design §3.4): one backend for the whole server, selected at
        // startup. An unknown spelling is a startup error naming the variable,
        // never a silent fall-through to the default backend.
        let backend_raw =
            std::env::var("PARALLAX_BACKEND").unwrap_or_else(|_| "anthropic".to_string());
        let backend =
            Backend::parse(&backend_raw).ok_or(ConfigError::Invalid("PARALLAX_BACKEND"))?;

        // Credentials are required per selected backend — never both:
        // `ANTHROPIC_API_KEY` iff anthropic, `OPENAI_MODEL` (plus
        // `OPENAI_API_KEY` on the default OpenAI endpoint) iff openai_compat.
        // The model id is always required because model names are
        // provider-specific; guessing one would fail later and less clearly.
        // A custom endpoint may run keyless — see `resolve_openai_api_key`.
        // Read before the credential check: `OPENAI_API_KEY`'s contract
        // depends on which endpoint the calls will hit.
        let openai_api_base = std::env::var("OPENAI_API_BASE")
            .unwrap_or_else(|_| DEFAULT_OPENAI_API_BASE.to_string());
        let (anthropic_api_key, openai_api_key, openai_model) = backend_credentials(
            backend,
            &openai_api_base,
            std::env::var("ANTHROPIC_API_KEY").ok(),
            std::env::var("OPENAI_API_KEY").ok(),
            std::env::var("OPENAI_MODEL").ok(),
        )?;

        let anthropic_api_base = std::env::var("ANTHROPIC_API_BASE")
            .unwrap_or_else(|_| crate::client::anthropic::ANTHROPIC_API_BASE.to_string());
        let anthropic_model =
            std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let structured_raw =
            std::env::var("OPENAI_STRUCTURED_OUTPUT").unwrap_or_else(|_| "auto".to_string());
        let openai_structured_output = StructuredOutput::parse(&structured_raw)
            .ok_or(ConfigError::Invalid("OPENAI_STRUCTURED_OUTPUT"))?;
        // Read in the form the config-facts scan recognizes, so the documented
        // defaults are checked against these constants (040). The model is
        // read before routing, which falls through to it on claude_cli.
        let claude_cli_path = reject_blank(
            "CLAUDE_CLI_PATH",
            std::env::var("CLAUDE_CLI_PATH")
                .unwrap_or_else(|_| DEFAULT_CLAUDE_CLI_PATH.to_string()),
        )?;
        let claude_cli_model = reject_blank(
            "CLAUDE_CLI_MODEL",
            std::env::var("CLAUDE_CLI_MODEL")
                .unwrap_or_else(|_| DEFAULT_CLAUDE_CLI_MODEL.to_string()),
        )?;
        // Routing resolves over the selected backend's default model, and
        // records which variable that default came from so the startup table
        // names the real supplier. A bad `PARALLAX_MODEL_*` variable stops
        // startup here, before any client is built (SC-005).
        let (default_model, default_var) =
            backend_default(backend, &anthropic_model, &openai_model, &claude_cli_model);
        let routing = RoutingTable::from_env(default_model, default_var)?;
        let verify_ensemble_k = validate_ensemble_k(parse_env("VERIFY_ENSEMBLE_K", 3)?)?;
        // INPUT_MAX_CHARS is canonical; VERIFY_MAX_CLAIM_CHARS is the 002-era
        // alias, honored only when the canonical variable is unset.
        let input_max_chars = if std::env::var("INPUT_MAX_CHARS").is_ok() {
            parse_env("INPUT_MAX_CHARS", 50_000)?
        } else {
            parse_env("VERIFY_MAX_CLAIM_CHARS", 50_000)?
        };
        let voyage_api_key = std::env::var("VOYAGE_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty());
        let voyage_model =
            std::env::var("VOYAGE_MODEL").unwrap_or_else(|_| DEFAULT_VOYAGE_MODEL.to_string());
        let memory_recall_limit = validate_recall_limit(parse_env("MEMORY_RECALL_LIMIT", 5)?)?;
        let exa_api_key = present_key(std::env::var("EXA_API_KEY").ok());
        check_removed(&|key| present_key(std::env::var(key).ok()).is_some())?;
        let exa_max_age_hours =
            validate_exa_max_age_hours(parse_env("EXA_MAX_AGE_HOURS", EXA_MAX_AGE_HOURS_DEFAULT)?)?;
        let research_concurrency =
            validate_research_concurrency(parse_env("RESEARCH_CONCURRENCY", 8)?)?;
        let claude_cli_concurrency = validate_claude_cli_concurrency(parse_env(
            "CLAUDE_CLI_CONCURRENCY",
            DEFAULT_CLAUDE_CLI_CONCURRENCY,
        )?)?;
        let checkpoint_gate_patterns =
            parse_gate_patterns(std::env::var("CHECKPOINT_GATE_PATTERNS").ok().as_deref())?;
        let grounded_verify_root = std::env::var("GROUNDED_VERIFY_ROOT")
            .ok()
            .filter(|root| !root.trim().is_empty());
        let grounded_verify_max_bytes = parse_env(
            "GROUNDED_VERIFY_MAX_BYTES",
            DEFAULT_GROUNDED_VERIFY_MAX_BYTES,
        )?;
        let grounded_verify_max_locators = parse_env(
            "GROUNDED_VERIFY_MAX_LOCATORS",
            DEFAULT_GROUNDED_VERIFY_MAX_LOCATORS,
        )?;
        let database_path =
            std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/parallax.db".to_string());
        let log_level =
            std::env::var("LOG_LEVEL").unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_string());
        // 018 D7 step 3: raised with the output budget. A ceiling four times
        // larger on a family that reasons before answering can outrun 30 s,
        // which would convert a truncation into a timeout rather than fixing
        // it. Provisional until the T050 family sweep confirms zero timeouts.
        let request_timeout_ms = parse_env("REQUEST_TIMEOUT_MS", 120_000)?;
        let max_retries = parse_env("MAX_RETRIES", 3)?;
        let jev = JevSettings::from_env()?;

        Ok(Self {
            backend,
            anthropic_api_key,
            anthropic_model,
            anthropic_api_base,
            openai_api_key,
            openai_api_base,
            openai_model,
            openai_structured_output,
            routing,
            verify_ensemble_k,
            input_max_chars,
            voyage_api_key,
            voyage_model,
            memory_recall_limit,
            exa_api_key,
            exa_max_age_hours,
            research_concurrency,
            claude_cli_path,
            claude_cli_model,
            claude_cli_concurrency,
            checkpoint_gate_patterns,
            grounded_verify_root,
            grounded_verify_max_bytes,
            grounded_verify_max_locators,
            database_path,
            log_level,
            request_timeout_ms,
            max_retries,
            jev,
        })
    }

    /// The default model id every unrouted call site resolves to: the selected
    /// backend's model (`ANTHROPIC_MODEL`, `OPENAI_MODEL` or `CLAUDE_CLI_MODEL`).
    #[must_use]
    pub fn default_model(&self) -> &str {
        backend_default(
            self.backend,
            &self.anthropic_model,
            &self.openai_model,
            &self.claude_cli_model,
        )
        .0
    }
}

/// The selected backend's default model and the variable that supplied it,
/// so the routing table names the real supplier.
fn backend_default<'a>(
    backend: Backend,
    anthropic_model: &'a str,
    openai_model: &'a str,
    claude_cli_model: &'a str,
) -> (&'a str, DefaultVar) {
    match backend {
        Backend::Anthropic => (anthropic_model, DefaultVar::AnthropicModel),
        Backend::OpenAiCompat => (openai_model, DefaultVar::OpenAiModel),
        Backend::ClaudeCli => (claude_cli_model, DefaultVar::ClaudeCliModel),
    }
}

/// The credentials the selected backend requires, as
/// `(anthropic_api_key, openai_api_key, openai_model)` — never more than one
/// backend's: `ANTHROPIC_API_KEY` iff anthropic, `OPENAI_MODEL` (plus
/// `OPENAI_API_KEY` on the default OpenAI endpoint) iff openai_compat, and
/// nothing on claude_cli, which signs in through the operator's Claude Code
/// (060 FR-002). The raw values are read in `from_env`, where the
/// config-facts scan sees each variable.
fn backend_credentials(
    backend: Backend,
    openai_api_base: &str,
    anthropic_api_key: Option<String>,
    openai_api_key: Option<String>,
    openai_model: Option<String>,
) -> Result<(String, String, String), ConfigError> {
    match backend {
        Backend::Anthropic => {
            let key = anthropic_api_key.ok_or(ConfigError::MissingRequired("ANTHROPIC_API_KEY"))?;
            if key.trim().is_empty() {
                return Err(ConfigError::MissingRequired("ANTHROPIC_API_KEY"));
            }
            Ok((key, String::new(), String::new()))
        }
        Backend::OpenAiCompat => {
            let key = resolve_openai_api_key(openai_api_base, openai_api_key)?;
            let model = openai_model.ok_or(ConfigError::MissingRequired("OPENAI_MODEL"))?;
            if model.trim().is_empty() {
                return Err(ConfigError::MissingRequired("OPENAI_MODEL"));
            }
            Ok((String::new(), key, model))
        }
        Backend::ClaudeCli => Ok((String::new(), String::new(), String::new())),
    }
}

/// The `OPENAI_API_KEY` contract for the `openai_compat` backend (BYOM §3.4):
/// the default OpenAI endpoint always authenticates, while a custom endpoint —
/// Ollama, LM Studio, vLLM, a proxy — may run keyless, in which case the
/// adapter sends no `Authorization` header at all.
///
/// A present-but-empty or whitespace-only value is normalized to empty so
/// "keyless" has exactly one spelling and a blank never becomes a bearer
/// token. Only the default endpoint's missing key fails startup; anywhere
/// else, an endpoint that does need a key answers 401 loudly on the first
/// call.
fn resolve_openai_api_key(
    openai_api_base: &str,
    raw: Option<String>,
) -> Result<String, ConfigError> {
    let key = raw.unwrap_or_default();
    let key = if key.trim().is_empty() {
        String::new()
    } else {
        key
    };
    let hits_default_endpoint = openai_api_base.trim_end_matches('/') == DEFAULT_OPENAI_API_BASE;
    if hits_default_endpoint && key.is_empty() {
        return Err(ConfigError::MissingRequired("OPENAI_API_KEY"));
    }
    Ok(key)
}

/// `VERIFY_ENSEMBLE_K` must be at least 1 — zero passes cannot produce a
/// verdict, so it is a configuration error, not a degenerate success.
fn validate_ensemble_k(k: u8) -> Result<u8, ConfigError> {
    if k >= 1 {
        Ok(k)
    } else {
        Err(ConfigError::Invalid("VERIFY_ENSEMBLE_K"))
    }
}

/// `MEMORY_RECALL_LIMIT` must be in `1..=MEMORY_RECALL_LIMIT_MAX`.
fn validate_recall_limit(limit: u8) -> Result<u8, ConfigError> {
    if (1..=MEMORY_RECALL_LIMIT_MAX).contains(&limit) {
        Ok(limit)
    } else {
        Err(ConfigError::Invalid("MEMORY_RECALL_LIMIT"))
    }
}

/// `RESEARCH_CONCURRENCY` must be in `1..=RESEARCH_CONCURRENCY_MAX`.
fn validate_research_concurrency(n: u8) -> Result<u8, ConfigError> {
    if (1..=RESEARCH_CONCURRENCY_MAX).contains(&n) {
        Ok(n)
    } else {
        Err(ConfigError::Invalid("RESEARCH_CONCURRENCY"))
    }
}

/// `CLAUDE_CLI_CONCURRENCY` must be in `1..=CLAUDE_CLI_CONCURRENCY_MAX`.
fn validate_claude_cli_concurrency(n: u8) -> Result<u8, ConfigError> {
    if (1..=CLAUDE_CLI_CONCURRENCY_MAX).contains(&n) {
        Ok(n)
    } else {
        Err(ConfigError::Invalid("CLAUDE_CLI_CONCURRENCY"))
    }
}

/// A string setting may be unset (its default applies) but never blank: a
/// blank value is a startup error naming `name`, not a silent default.
fn reject_blank(name: &'static str, value: String) -> Result<String, ConfigError> {
    if value.trim().is_empty() {
        Err(ConfigError::Invalid(name))
    } else {
        Ok(value)
    }
}

/// Variables removed by 059, checked before anything starts (FR-002,
/// FR-003). `present` reports a variable set to a non-blank value. A Brave
/// key is an error only when no Exa key would enable research instead —
/// other tools share `BRAVE_API_KEY`, so with Exa set it is ignored. The two
/// fetch settings are errors whenever set: nothing reads them any more.
fn check_removed(present: &dyn Fn(&str) -> bool) -> Result<(), ConfigError> {
    if present("BRAVE_API_KEY") && !present("EXA_API_KEY") {
        return Err(ConfigError::Removed {
            var: "BRAVE_API_KEY",
            remedy: "research now uses Exa; set EXA_API_KEY (BRAVE_API_KEY is ignored once \
                     EXA_API_KEY is set)",
        });
    }
    for var in ["FETCH_TIMEOUT_MS", "FETCH_ALLOW_PRIVATE"] {
        if present(var) {
            return Err(ConfigError::Removed {
                var,
                remedy: "Parallax no longer fetches pages (research text comes from Exa); \
                         remove it",
            });
        }
    }
    Ok(())
}

/// An API key is present only when non-blank.
fn present_key(raw: Option<String>) -> Option<String> {
    raw.filter(|key| !key.trim().is_empty())
}

/// `EXA_MAX_AGE_HOURS` must be in `-1..=720` (Exa's range). Unparseable
/// values already fail in [`parse_env`], naming the variable.
fn validate_exa_max_age_hours(hours: i32) -> Result<i32, ConfigError> {
    if (-1..=720).contains(&hours) {
        Ok(hours)
    } else {
        Err(ConfigError::Invalid("EXA_MAX_AGE_HOURS"))
    }
}

/// Parse `CHECKPOINT_GATE_PATTERNS`: comma-separated, trimmed, all entries
/// non-empty. Unset → empty (built-ins only). A present-but-malformed value
/// (an empty entry) is an error, never a silent skip.
fn parse_gate_patterns(raw: Option<&str>) -> Result<Vec<String>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let patterns: Vec<String> = raw.split(',').map(|p| p.trim().to_string()).collect();
    if patterns.iter().any(String::is_empty) {
        return Err(ConfigError::Invalid("CHECKPOINT_GATE_PATTERNS"));
    }
    Ok(patterns)
}

/// Read an environment variable and parse it, falling back to `default` when the
/// variable is unset. A present-but-unparseable value is an error, not a silent
/// fallback.
fn parse_env<T>(key: &'static str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
{
    std::env::var(key).map_or_else(
        |_| Ok(default),
        |value| value.parse::<T>().map_err(|_| ConfigError::Invalid(key)),
    )
}

/// A `Config` for unit tests: no network reachable, no capability gated on.
///
/// Four modules hand-rolled this same 21-field literal and it had already
/// drifted — `server.rs` used `max_retries: 1` where the three client modules
/// used `2`, for no recorded reason. Each copy was one more place a new field
/// gets a value nobody chose. Override what a test needs with struct update
/// syntax, so the difference is the only thing written down:
///
/// ```ignore
/// Config { brave_api_key: Some("k".into()), ..crate::config::test_config() }
/// ```
///
/// **`tests/integration.rs` cannot use this**, and keeps its own copy. An
/// integration test is a separate crate linking the library compiled *without*
/// `cfg(test)`, so this item does not exist for it — the same linkage that
/// forced `config_facts` into the binary crate (040). Making it visible would
/// mean shipping test scaffolding in the public API, which is a worse trade
/// than one duplicated fixture.
///
/// The endpoint is `127.0.0.1:1` so a test that escapes its mock fails by
/// connection refusal rather than reaching the live API on a fixture key —
/// the failure 028's review found the suite had been doing.
#[cfg(test)]
#[must_use]
pub(crate) fn test_config() -> Config {
    Config {
        anthropic_api_key: "test-key".into(),
        anthropic_model: DEFAULT_MODEL.into(),
        anthropic_api_base: "http://127.0.0.1:1".into(),
        routing: crate::routing::RoutingTable::single(DEFAULT_MODEL, DefaultVar::AnthropicModel),
        verify_ensemble_k: 3,
        input_max_chars: 50_000,
        voyage_api_key: None,
        voyage_model: DEFAULT_VOYAGE_MODEL.into(),
        memory_recall_limit: 5,
        exa_api_key: None,
        exa_max_age_hours: EXA_MAX_AGE_HOURS_DEFAULT,
        research_concurrency: 8,
        claude_cli_path: DEFAULT_CLAUDE_CLI_PATH.into(),
        claude_cli_model: DEFAULT_CLAUDE_CLI_MODEL.into(),
        claude_cli_concurrency: DEFAULT_CLAUDE_CLI_CONCURRENCY,
        checkpoint_gate_patterns: vec![],
        grounded_verify_root: None,
        grounded_verify_max_bytes: DEFAULT_GROUNDED_VERIFY_MAX_BYTES,
        grounded_verify_max_locators: DEFAULT_GROUNDED_VERIFY_MAX_LOCATORS,
        database_path: ":memory:".into(),
        log_level: DEFAULT_LOG_LEVEL.into(),
        request_timeout_ms: 2_000,
        max_retries: 2,
        backend: Backend::Anthropic,
        openai_api_key: String::new(),
        openai_api_base: "http://127.0.0.1:1".into(),
        openai_model: String::new(),
        openai_structured_output: StructuredOutput::Auto,
        jev: JevSettings {
            model: DEFAULT_JEV_MODEL.into(),
            ..JevSettings::default()
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_returns_default_when_unset() {
        // A key guaranteed not to be set in the test environment.
        let got: u64 = parse_env("PARALLAX_TEST_DEFINITELY_UNSET_KEY", 42).unwrap();
        assert_eq!(got, 42);
    }

    // 018 T008 / SC-005. `from_env` reads the real process environment, which
    // tests must not mutate (parallel threads share it), so the startup-refusal
    // path is asserted at the pure resolution layer `from_env` delegates to.
    // The wiring itself — that `from_env` propagates this error with `?` before
    // any client is built — is a one-line call verified by review.
    #[test]
    fn a_bad_routing_variable_is_a_config_error_naming_it() {
        let err = RoutingTable::resolve(
            vec![(
                "PARALLAX_MODEL_NOT_A_CALL_SITE".to_string(),
                "claude-haiku-4-5".to_string(),
            )],
            DEFAULT_MODEL,
            DefaultVar::AnthropicModel,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Routing(_)));
        assert!(err.to_string().contains("PARALLAX_MODEL_NOT_A_CALL_SITE"));
    }

    // FR-002: the identity `from_env` relies on — nothing set means every call
    // site on `ANTHROPIC_MODEL`, which is exactly the pre-018 shape.
    #[test]
    fn unrouted_resolution_equals_the_single_model_table() {
        let resolved =
            RoutingTable::resolve(Vec::new(), DEFAULT_MODEL, DefaultVar::AnthropicModel).unwrap();
        assert_eq!(
            resolved,
            RoutingTable::single(DEFAULT_MODEL, DefaultVar::AnthropicModel)
        );
        assert_eq!(resolved.distinct_models(), vec![DEFAULT_MODEL.to_string()]);
    }

    // The startup table must name the variable that actually supplied the
    // default model — `OPENAI_MODEL` on openai_compat, not a hardcoded
    // `ANTHROPIC_MODEL`.
    #[test]
    fn the_default_route_names_the_backend_default_variable() {
        let openai =
            RoutingTable::resolve(Vec::new(), "qwen2.5:7b", DefaultVar::OpenAiModel).unwrap();
        assert_eq!(
            openai.report()[0].3,
            "OPENAI_MODEL",
            "openai_compat fall-through must be reported as OPENAI_MODEL"
        );
        let anthropic =
            RoutingTable::resolve(Vec::new(), DEFAULT_MODEL, DefaultVar::AnthropicModel).unwrap();
        assert_eq!(anthropic.report()[0].3, "ANTHROPIC_MODEL");
    }

    #[test]
    fn ensemble_k_zero_is_a_config_error_naming_the_variable() {
        let err = validate_ensemble_k(0).unwrap_err();
        assert!(err.to_string().contains("VERIFY_ENSEMBLE_K"));
    }

    #[test]
    fn ensemble_k_accepts_one_and_above() {
        assert_eq!(validate_ensemble_k(1).unwrap(), 1);
        assert_eq!(validate_ensemble_k(3).unwrap(), 3);
        assert_eq!(validate_ensemble_k(u8::MAX).unwrap(), u8::MAX);
    }

    #[test]
    fn recall_limit_bounds_name_the_variable() {
        assert!(validate_recall_limit(0)
            .unwrap_err()
            .to_string()
            .contains("MEMORY_RECALL_LIMIT"));
        assert!(validate_recall_limit(21).is_err());
        assert_eq!(validate_recall_limit(1).unwrap(), 1);
        assert_eq!(validate_recall_limit(20).unwrap(), 20);
    }

    /// 059 T005 (research.md D3): -1 (cache only) through 720 load;
    /// anything outside is a startup error naming the variable. The 168
    /// default is checked against README and --help by the config-facts scan.
    #[test]
    fn exa_max_age_hours_bounds_name_the_variable() {
        for hours in [-1, 0, 24, 720] {
            assert_eq!(validate_exa_max_age_hours(hours).unwrap(), hours);
        }
        for hours in [-2, 721] {
            let err = validate_exa_max_age_hours(hours).unwrap_err();
            assert!(
                err.to_string().contains("EXA_MAX_AGE_HOURS"),
                "{hours}: {err}"
            );
        }
    }

    /// 059 T011 (FR-002, FR-003, research.md D7 as amended): a Brave key is
    /// an error only when no Exa key would enable research instead; the two
    /// fetch settings are errors whenever set. Each error names its variable.
    #[test]
    fn removed_variables_are_startup_errors_naming_them() {
        let set = |names: &'static [&'static str]| move |key: &str| names.contains(&key);

        assert!(check_removed(&set(&[])).is_ok());
        assert!(check_removed(&set(&["EXA_API_KEY"])).is_ok());

        let msg = check_removed(&set(&["BRAVE_API_KEY"]))
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("BRAVE_API_KEY") && msg.contains("EXA_API_KEY"),
            "{msg}"
        );
        assert!(
            check_removed(&set(&["BRAVE_API_KEY", "EXA_API_KEY"])).is_ok(),
            "Brave is ignored with Exa set"
        );

        for name in ["FETCH_TIMEOUT_MS", "FETCH_ALLOW_PRIVATE"] {
            let names: &'static [&'static str] = if name == "FETCH_TIMEOUT_MS" {
                &["FETCH_TIMEOUT_MS", "EXA_API_KEY"]
            } else {
                &["FETCH_ALLOW_PRIVATE", "EXA_API_KEY"]
            };
            let msg = check_removed(&set(names)).unwrap_err().to_string();
            assert!(
                msg.contains(name) && msg.contains("no longer used"),
                "{msg}"
            );
        }
    }

    /// 059 T005: the Exa key is present only when non-blank.
    #[test]
    fn exa_api_key_is_present_only_when_non_blank() {
        assert_eq!(present_key(Some("k".into())), Some("k".to_string()));
        assert_eq!(present_key(Some("  ".into())), None);
        assert_eq!(present_key(None), None);
    }

    #[test]
    fn research_concurrency_bounds_name_the_variable() {
        assert!(validate_research_concurrency(0)
            .unwrap_err()
            .to_string()
            .contains("RESEARCH_CONCURRENCY"));
        assert!(validate_research_concurrency(33).is_err());
        assert_eq!(validate_research_concurrency(1).unwrap(), 1);
        assert_eq!(validate_research_concurrency(32).unwrap(), 32);
    }

    #[test]
    fn gate_patterns_parse_trim_and_reject_empty_entries() {
        assert!(parse_gate_patterns(None).unwrap().is_empty());
        assert_eq!(
            parse_gate_patterns(Some(" systemctl , docker compose down ")).unwrap(),
            vec!["systemctl".to_string(), "docker compose down".to_string()]
        );
        let err = parse_gate_patterns(Some("a,,b")).unwrap_err();
        assert!(err.to_string().contains("CHECKPOINT_GATE_PATTERNS"));
    }

    #[test]
    fn default_models_are_the_corpus_targets() {
        assert_eq!(DEFAULT_MODEL, "claude-opus-4-8");
        assert_eq!(DEFAULT_VOYAGE_MODEL, "voyage-4");
    }

    #[test]
    fn the_openai_key_is_required_only_on_the_default_endpoint() {
        assert!(matches!(
            resolve_openai_api_key(DEFAULT_OPENAI_API_BASE, None),
            Err(ConfigError::MissingRequired("OPENAI_API_KEY"))
        ));
        assert!(matches!(
            resolve_openai_api_key(DEFAULT_OPENAI_API_BASE, Some("   ".to_string())),
            Err(ConfigError::MissingRequired("OPENAI_API_KEY"))
        ));
        assert_eq!(
            resolve_openai_api_key(DEFAULT_OPENAI_API_BASE, Some("sk-test".to_string())).ok(),
            Some("sk-test".to_string())
        );
    }

    #[test]
    fn a_custom_endpoint_may_run_keyless() {
        assert_eq!(
            resolve_openai_api_key("http://localhost:11434/v1", None).ok(),
            Some(String::new())
        );
        assert_eq!(
            resolve_openai_api_key("http://localhost:11434/v1", Some("  ".to_string())).ok(),
            Some(String::new())
        );
        assert_eq!(
            resolve_openai_api_key("http://localhost:11434/v1", Some("key".to_string())).ok(),
            Some("key".to_string())
        );
        // A trailing slash on the default endpoint does not dodge the contract.
        assert!(matches!(
            resolve_openai_api_key("https://api.openai.com/v1/", None),
            Err(ConfigError::MissingRequired("OPENAI_API_KEY"))
        ));
    }

    /// 060 T011: `claude_cli` parses, and no API key is required on it; the
    /// anthropic backend still requires its key.
    #[test]
    fn claude_cli_parses_and_needs_no_api_key() {
        assert_eq!(Backend::parse("claude_cli"), Some(Backend::ClaudeCli));
        assert_eq!(Backend::parse(" CLAUDE_CLI "), Some(Backend::ClaudeCli));
        assert_eq!(Backend::ClaudeCli.as_str(), "claude_cli");
        assert!(Backend::ALL.contains(&Backend::ClaudeCli));
        assert_eq!(
            backend_credentials(
                Backend::ClaudeCli,
                DEFAULT_OPENAI_API_BASE,
                None,
                None,
                None
            )
            .unwrap(),
            (String::new(), String::new(), String::new())
        );
        assert!(matches!(
            backend_credentials(
                Backend::Anthropic,
                DEFAULT_OPENAI_API_BASE,
                None,
                None,
                None
            ),
            Err(ConfigError::MissingRequired("ANTHROPIC_API_KEY"))
        ));
    }

    /// 060 T011: on `claude_cli` the default model is `CLAUDE_CLI_MODEL`, and
    /// the routing table reports it as that variable.
    #[test]
    fn claude_cli_default_model_is_claude_cli_model() {
        let cfg = Config {
            backend: Backend::ClaudeCli,
            claude_cli_model: "claude-sonnet-5-5".into(),
            ..test_config()
        };
        assert_eq!(cfg.default_model(), "claude-sonnet-5-5");
        assert_eq!(
            backend_default(Backend::ClaudeCli, "a", "o", "c"),
            ("c", DefaultVar::ClaudeCliModel)
        );
        assert_eq!(
            backend_default(Backend::Anthropic, "a", "o", "c"),
            ("a", DefaultVar::AnthropicModel)
        );
        assert_eq!(
            backend_default(Backend::OpenAiCompat, "a", "o", "c"),
            ("o", DefaultVar::OpenAiModel)
        );
    }

    /// 060 T005: `CLAUDE_CLI_CONCURRENCY` loads in 1..=32; anything outside is
    /// a startup error naming the variable.
    #[test]
    fn claude_cli_concurrency_bounds_name_the_variable() {
        for n in [1, 4, 32] {
            assert_eq!(validate_claude_cli_concurrency(n).unwrap(), n);
        }
        for n in [0, 33] {
            let err = validate_claude_cli_concurrency(n).unwrap_err();
            assert!(
                err.to_string().contains("CLAUDE_CLI_CONCURRENCY"),
                "{n}: {err}"
            );
        }
    }

    /// 060 T005 (research.md D4, D5): the documented defaults.
    #[test]
    fn claude_cli_defaults_are_the_documented_values() {
        assert_eq!(DEFAULT_CLAUDE_CLI_PATH, "claude");
        assert_eq!(DEFAULT_CLAUDE_CLI_MODEL, "claude-sonnet-5-5");
        assert_eq!(DEFAULT_CLAUDE_CLI_CONCURRENCY, 4);
        let cfg = test_config();
        assert_eq!(cfg.claude_cli_path, DEFAULT_CLAUDE_CLI_PATH);
        assert_eq!(cfg.claude_cli_model, DEFAULT_CLAUDE_CLI_MODEL);
        assert_eq!(cfg.claude_cli_concurrency, DEFAULT_CLAUDE_CLI_CONCURRENCY);
    }

    /// 060 T005: a blank `CLAUDE_CLI_PATH` or `CLAUDE_CLI_MODEL` is a startup
    /// error naming it, never a silent fall-back to the default.
    #[test]
    fn a_blank_claude_cli_setting_is_an_error_naming_it() {
        for name in ["CLAUDE_CLI_PATH", "CLAUDE_CLI_MODEL"] {
            let err = reject_blank(name, "  ".into()).unwrap_err();
            assert!(err.to_string().contains(name), "{err}");
        }
        assert_eq!(
            reject_blank("CLAUDE_CLI_PATH", "C:/tools/claude.exe".into()).unwrap(),
            "C:/tools/claude.exe"
        );
    }

    #[test]
    fn grounded_verify_defaults_match_the_documented_values() {
        // The README/CLAUDE config tables and the spec cite these — keep them
        // in lockstep with the named constants `from_env` uses.
        assert_eq!(DEFAULT_GROUNDED_VERIFY_MAX_BYTES, 262_144);
        assert_eq!(DEFAULT_GROUNDED_VERIFY_MAX_LOCATORS, 64);
    }

    // 064 T003: Jev settings, resolved from a variable set like routing so
    // tests never touch the process environment.
    mod jev {
        use super::super::JevSettings;
        use crate::traits::classifier::JevMode;

        fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect()
        }

        const KEY: (&str, &str) = ("TYPESAFE_API_KEY", "test-key");

        #[test]
        fn with_no_jev_variables_both_sites_are_off() {
            for set in [vars(&[]), vars(&[KEY])] {
                let s = JevSettings::resolve(set).unwrap();
                assert_eq!(s.turn_review, JevMode::Off);
                assert_eq!(s.research_claims, JevMode::Off);
            }
        }

        #[test]
        fn turn_review_shadow_with_a_key_is_shadow() {
            let s = JevSettings::resolve(vars(&[KEY, ("PARALLAX_JEV_MODE_TURN_REVIEW", "shadow")]))
                .unwrap();
            assert_eq!(s.turn_review, JevMode::Shadow);
            assert_eq!(s.research_claims, JevMode::Off);
        }

        #[test]
        fn research_claims_shadow_with_a_key_is_shadow() {
            let s = JevSettings::resolve(vars(&[
                KEY,
                ("PARALLAX_JEV_MODE_RESEARCH_CLAIMS", "shadow"),
            ]))
            .unwrap();
            assert_eq!(s.research_claims, JevMode::Shadow);
            assert_eq!(s.turn_review, JevMode::Off);
        }

        #[test]
        fn an_unknown_mode_suffix_is_an_error_naming_it() {
            let err = JevSettings::resolve(vars(&[KEY, ("PARALLAX_JEV_MODE_FOO", "shadow")]))
                .unwrap_err();
            assert!(err.to_string().contains("PARALLAX_JEV_MODE_FOO"), "{err}");
        }

        #[test]
        fn a_bad_mode_value_is_an_error_naming_the_variable() {
            let err =
                JevSettings::resolve(vars(&[KEY, ("PARALLAX_JEV_MODE_TURN_REVIEW", "maybe")]))
                    .unwrap_err();
            assert!(
                err.to_string().contains("PARALLAX_JEV_MODE_TURN_REVIEW"),
                "{err}"
            );
        }

        #[test]
        fn no_jev_config_error_spans_lines() {
            let cases: [&[(&str, &str)]; 5] = [
                &[KEY, ("PARALLAX_JEV_MODE_NOPE", "shadow")],
                &[KEY, ("PARALLAX_JEV_MODE_TURN_REVIEW", "maybe")],
                &[KEY, ("PARALLAX_JEV_MODE_TURN_REVIEW", "on")],
                &[("PARALLAX_JEV_MODE_RESEARCH_INJECTION", "shadow")],
                &[
                    KEY,
                    ("PARALLAX_JEV_MODE_RESEARCH_INJECTION", "on"),
                    ("PARALLAX_JEV_MODEL", "jev-1.13.0"),
                ],
            ];
            for case in cases {
                let err = JevSettings::resolve(vars(case)).unwrap_err().to_string();
                assert!(!err.contains('\n'), "{err:?}");
                assert!(!err.contains("  "), "{err:?}");
            }
        }

        #[test]
        fn the_injection_site_takes_off_shadow_and_on() {
            let off = JevSettings::resolve(vars(&[KEY])).unwrap();
            assert_eq!(off.research_injection, JevMode::Off);
            for (value, mode) in [("shadow", JevMode::Shadow), ("on", JevMode::On)] {
                let s = JevSettings::resolve(vars(&[
                    KEY,
                    ("PARALLAX_JEV_MODE_RESEARCH_INJECTION", value),
                ]))
                .unwrap();
                assert_eq!(s.research_injection, mode);
            }
        }

        #[test]
        fn injection_on_without_a_pinned_threshold_names_both_variables() {
            let err = JevSettings::resolve(vars(&[
                KEY,
                ("PARALLAX_JEV_MODE_RESEARCH_INJECTION", "on"),
                ("PARALLAX_JEV_MODEL", "jev-1.13.0"),
            ]))
            .unwrap_err()
            .to_string();
            assert!(
                err.contains("PARALLAX_JEV_MODE_RESEARCH_INJECTION"),
                "{err}"
            );
            assert!(err.contains("PARALLAX_JEV_MODEL"), "{err}");
        }

        #[test]
        fn injection_without_a_key_is_an_error_naming_the_variable() {
            for value in ["shadow", "on"] {
                let err =
                    JevSettings::resolve(vars(&[("PARALLAX_JEV_MODE_RESEARCH_INJECTION", value)]))
                        .unwrap_err();
                assert!(
                    err.to_string()
                        .contains("PARALLAX_JEV_MODE_RESEARCH_INJECTION"),
                    "{err}"
                );
            }
        }

        #[test]
        fn on_stays_an_error_for_the_turn_review_site() {
            let err = JevSettings::resolve(vars(&[KEY, ("PARALLAX_JEV_MODE_TURN_REVIEW", "on")]))
                .unwrap_err();
            assert!(
                err.to_string().contains("PARALLAX_JEV_MODE_TURN_REVIEW"),
                "{err}"
            );
        }

        #[test]
        fn on_is_not_accepted_in_064() {
            let err =
                JevSettings::resolve(vars(&[KEY, ("PARALLAX_JEV_MODE_RESEARCH_CLAIMS", "on")]))
                    .unwrap_err();
            assert!(
                err.to_string()
                    .contains("PARALLAX_JEV_MODE_RESEARCH_CLAIMS"),
                "{err}"
            );
        }

        #[test]
        fn a_mode_without_a_key_is_an_error_naming_the_mode_variable() {
            let err = JevSettings::resolve(vars(&[("PARALLAX_JEV_MODE_TURN_REVIEW", "shadow")]))
                .unwrap_err();
            assert!(
                err.to_string().contains("PARALLAX_JEV_MODE_TURN_REVIEW"),
                "{err}"
            );
        }

        #[test]
        fn a_blank_key_counts_as_no_key() {
            let s = JevSettings::resolve(vars(&[("TYPESAFE_API_KEY", "  ")])).unwrap();
            assert!(s.api_key.is_none());
            let err = JevSettings::resolve(vars(&[
                ("TYPESAFE_API_KEY", "  "),
                ("PARALLAX_JEV_MODE_TURN_REVIEW", "shadow"),
            ]))
            .unwrap_err();
            assert!(
                err.to_string().contains("PARALLAX_JEV_MODE_TURN_REVIEW"),
                "{err}"
            );
        }

        #[test]
        fn the_model_defaults_to_the_pinned_version_and_is_not_a_mode_suffix() {
            let default = JevSettings::resolve(vars(&[KEY])).unwrap();
            assert_eq!(default.model, "jev-1.14.0");
            let set =
                JevSettings::resolve(vars(&[KEY, ("PARALLAX_JEV_MODEL", "jev-1.13.0")])).unwrap();
            assert_eq!(set.model, "jev-1.13.0");
        }

        /// Review finding D: with a site in shadow, a model without a price
        /// row stops startup instead of being costed at the fallback rate.
        #[test]
        fn an_unpriced_model_with_a_site_on_stops_startup() {
            let err = JevSettings::resolve(vars(&[
                KEY,
                ("PARALLAX_JEV_MODEL", "jev-1.13.0"),
                ("PARALLAX_JEV_MODE_TURN_REVIEW", "shadow"),
            ]))
            .unwrap_err()
            .to_string();
            assert!(err.contains("PARALLAX_JEV_MODEL"), "{err}");
            assert!(err.contains("jev-1.13.0"), "{err}");
        }

        /// Review finding F: a blank model counts as unset.
        #[test]
        fn a_blank_model_counts_as_unset() {
            let s = JevSettings::resolve(vars(&[KEY, ("PARALLAX_JEV_MODEL", "  ")])).unwrap();
            assert_eq!(s.model, "jev-1.14.0");
        }

        #[test]
        fn exa_mcp_servers_jev_model_is_ignored() {
            let s = JevSettings::resolve(vars(&[KEY, ("JEV_MODEL", "jev-latest")])).unwrap();
            assert_eq!(s.model, "jev-1.14.0");
        }
    }
}
