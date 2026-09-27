//! Runtime configuration, sourced entirely from the environment.

use crate::error::ConfigError;
use crate::routing::RoutingTable;

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

/// Server-side ceiling on research concurrency.
pub const RESEARCH_CONCURRENCY_MAX: u8 = 32;

/// Default total assembled-evidence ceiling for grounded-verify, in bytes
/// (256 KiB). `GROUNDED_VERIFY_MAX_BYTES`.
pub const DEFAULT_GROUNDED_VERIFY_MAX_BYTES: usize = 262_144;

/// Default maximum locators accepted in one grounded-verify call.
/// `GROUNDED_VERIFY_MAX_LOCATORS`.
pub const DEFAULT_GROUNDED_VERIFY_MAX_LOCATORS: usize = 64;

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
}

impl Backend {
    /// Every backend, for validation and reporting.
    pub const ALL: [Self; 2] = [Self::Anthropic, Self::OpenAiCompat];

    /// The operator-facing spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAiCompat => "openai_compat",
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
    /// [`Backend::OpenAiCompat`], empty otherwise. `OPENAI_API_KEY`.
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
    /// Brave Search API key. **Optional — its presence enables the research
    /// capability** (`research`); absent, the tool does not exist and no
    /// research egress is ever made. `BRAVE_API_KEY`.
    pub brave_api_key: Option<String>,
    /// Per-source fetch timeout in milliseconds. `FETCH_TIMEOUT_MS`,
    /// default `10000`.
    pub fetch_timeout_ms: u64,
    /// Concurrent fetch/extract/verify cap for research runs.
    /// `RESEARCH_CONCURRENCY`, default `8`; must be in `1..=32`.
    pub research_concurrency: u8,
    /// Permit research fetches to loopback/private/link-local targets.
    /// `FETCH_ALLOW_PRIVATE`, default `false` — an SSRF guard; enable only
    /// for local testing.
    pub fetch_allow_private: bool,
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
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingRequired`] if a key the selected backend
    /// requires is unset or empty (`ANTHROPIC_API_KEY` for `anthropic`;
    /// `OPENAI_API_KEY` and `OPENAI_MODEL` for `openai_compat`), and
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
        // `ANTHROPIC_API_KEY` iff anthropic, `OPENAI_API_KEY` and `OPENAI_MODEL`
        // iff openai_compat. The model id joins the key in being required
        // because model names are provider-specific; guessing one would fail
        // later and less clearly.
        let (anthropic_api_key, openai_api_key, openai_model) = match backend {
            Backend::Anthropic => {
                let key = std::env::var("ANTHROPIC_API_KEY")
                    .map_err(|_| ConfigError::MissingRequired("ANTHROPIC_API_KEY"))?;
                if key.trim().is_empty() {
                    return Err(ConfigError::MissingRequired("ANTHROPIC_API_KEY"));
                }
                (key, String::new(), String::new())
            }
            Backend::OpenAiCompat => {
                let key = std::env::var("OPENAI_API_KEY")
                    .map_err(|_| ConfigError::MissingRequired("OPENAI_API_KEY"))?;
                if key.trim().is_empty() {
                    return Err(ConfigError::MissingRequired("OPENAI_API_KEY"));
                }
                let model = std::env::var("OPENAI_MODEL")
                    .map_err(|_| ConfigError::MissingRequired("OPENAI_MODEL"))?;
                if model.trim().is_empty() {
                    return Err(ConfigError::MissingRequired("OPENAI_MODEL"));
                }
                (String::new(), key, model)
            }
        };

        let anthropic_api_base = std::env::var("ANTHROPIC_API_BASE")
            .unwrap_or_else(|_| crate::client::anthropic::ANTHROPIC_API_BASE.to_string());
        let openai_api_base = std::env::var("OPENAI_API_BASE")
            .unwrap_or_else(|_| DEFAULT_OPENAI_API_BASE.to_string());
        let anthropic_model =
            std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let structured_raw =
            std::env::var("OPENAI_STRUCTURED_OUTPUT").unwrap_or_else(|_| "auto".to_string());
        let openai_structured_output = StructuredOutput::parse(&structured_raw)
            .ok_or(ConfigError::Invalid("OPENAI_STRUCTURED_OUTPUT"))?;
        // Routing resolves over the selected backend's default model. A bad
        // `PARALLAX_MODEL_*` variable stops startup here, before any client is
        // built (SC-005).
        let routing = RoutingTable::from_env(match backend {
            Backend::Anthropic => &anthropic_model,
            Backend::OpenAiCompat => &openai_model,
        })?;
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
        let brave_api_key = std::env::var("BRAVE_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty());
        let fetch_timeout_ms = parse_env("FETCH_TIMEOUT_MS", 10_000)?;
        let research_concurrency =
            validate_research_concurrency(parse_env("RESEARCH_CONCURRENCY", 8)?)?;
        let fetch_allow_private = parse_env("FETCH_ALLOW_PRIVATE", false)?;
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
            brave_api_key,
            fetch_timeout_ms,
            research_concurrency,
            fetch_allow_private,
            checkpoint_gate_patterns,
            grounded_verify_root,
            grounded_verify_max_bytes,
            grounded_verify_max_locators,
            database_path,
            log_level,
            request_timeout_ms,
            max_retries,
        })
    }

    /// The default model id every unrouted call site resolves to: the selected
    /// backend's model (`ANTHROPIC_MODEL` or `OPENAI_MODEL`).
    #[must_use]
    pub fn default_model(&self) -> &str {
        match self.backend {
            Backend::Anthropic => &self.anthropic_model,
            Backend::OpenAiCompat => &self.openai_model,
        }
    }
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
        routing: crate::routing::RoutingTable::single(DEFAULT_MODEL),
        verify_ensemble_k: 3,
        input_max_chars: 50_000,
        voyage_api_key: None,
        voyage_model: DEFAULT_VOYAGE_MODEL.into(),
        memory_recall_limit: 5,
        brave_api_key: None,
        fetch_timeout_ms: 10_000,
        research_concurrency: 8,
        fetch_allow_private: false,
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
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Routing(_)));
        assert!(err.to_string().contains("PARALLAX_MODEL_NOT_A_CALL_SITE"));
    }

    // FR-002: the identity `from_env` relies on — nothing set means every call
    // site on `ANTHROPIC_MODEL`, which is exactly the pre-018 shape.
    #[test]
    fn unrouted_resolution_equals_the_single_model_table() {
        let resolved = RoutingTable::resolve(Vec::new(), DEFAULT_MODEL).unwrap();
        assert_eq!(resolved, RoutingTable::single(DEFAULT_MODEL));
        assert_eq!(resolved.distinct_models(), vec![DEFAULT_MODEL.to_string()]);
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
    fn grounded_verify_defaults_match_the_documented_values() {
        // The README/CLAUDE config tables and the spec cite these — keep them
        // in lockstep with the named constants `from_env` uses.
        assert_eq!(DEFAULT_GROUNDED_VERIFY_MAX_BYTES, 262_144);
        assert_eq!(DEFAULT_GROUNDED_VERIFY_MAX_LOCATORS, 64);
    }
}
