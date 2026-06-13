//! Runtime configuration, sourced entirely from the environment.

use crate::error::ConfigError;

/// Default model when `ANTHROPIC_MODEL` is unset — the design corpus's stated
/// target (structured outputs GA).
pub const DEFAULT_MODEL: &str = "claude-opus-4-8";

/// Default embedding model when `VOYAGE_MODEL` is unset. The voyage-4 family
/// shares one embedding space, so switching within the family needs no
/// re-index.
pub const DEFAULT_VOYAGE_MODEL: &str = "voyage-4";

/// Server-side ceiling on recall result counts.
pub const MEMORY_RECALL_LIMIT_MAX: u8 = 20;

/// Server-side ceiling on research concurrency.
pub const RESEARCH_CONCURRENCY_MAX: u8 = 32;

/// Server configuration. Every field is sourced from an environment variable so
/// the binary is configured the same way in every host (Claude Code / Desktop).
#[derive(Debug, Clone)]
pub struct Config {
    /// Anthropic API key (required). `ANTHROPIC_API_KEY`.
    pub anthropic_api_key: String,
    /// Model id for verification passes. `ANTHROPIC_MODEL`, default
    /// [`DEFAULT_MODEL`].
    pub anthropic_model: String,
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
    /// Per-request timeout in milliseconds. `REQUEST_TIMEOUT_MS`, default `30000`.
    pub request_timeout_ms: u64,
    /// Maximum API retry attempts. `MAX_RETRIES`, default `3`.
    pub max_retries: u32,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingRequired`] if `ANTHROPIC_API_KEY` is unset
    /// or empty, and [`ConfigError::Invalid`] if a numeric variable is present
    /// but fails to parse or violates its bounds (`VERIFY_ENSEMBLE_K` ≥ 1,
    /// `MEMORY_RECALL_LIMIT` in 1..=20). A present-but-invalid value is an
    /// error, never a silent default.
    pub fn from_env() -> Result<Self, ConfigError> {
        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ConfigError::MissingRequired("ANTHROPIC_API_KEY"))?;
        if anthropic_api_key.trim().is_empty() {
            return Err(ConfigError::MissingRequired("ANTHROPIC_API_KEY"));
        }

        let anthropic_model =
            std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
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
        let grounded_verify_max_bytes = parse_env("GROUNDED_VERIFY_MAX_BYTES", 262_144_usize)?;
        let grounded_verify_max_locators = parse_env("GROUNDED_VERIFY_MAX_LOCATORS", 64_usize)?;
        let database_path =
            std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/parallax.db".to_string());
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        let request_timeout_ms = parse_env("REQUEST_TIMEOUT_MS", 30_000)?;
        let max_retries = parse_env("MAX_RETRIES", 3)?;

        Ok(Self {
            anthropic_api_key,
            anthropic_model,
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
}
