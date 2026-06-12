//! Runtime configuration, sourced entirely from the environment.

use crate::error::ConfigError;

/// Default model when `ANTHROPIC_MODEL` is unset — the design corpus's stated
/// target (structured outputs GA).
pub const DEFAULT_MODEL: &str = "claude-opus-4-8";

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
    /// Upper bound on claim length in characters. `VERIFY_MAX_CLAIM_CHARS`,
    /// default `50000`.
    pub verify_max_claim_chars: usize,
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
    /// but fails to parse or violates its bounds (`VERIFY_ENSEMBLE_K` must be
    /// ≥ 1). A present-but-invalid value is an error, never a silent default.
    pub fn from_env() -> Result<Self, ConfigError> {
        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ConfigError::MissingRequired("ANTHROPIC_API_KEY"))?;
        if anthropic_api_key.trim().is_empty() {
            return Err(ConfigError::MissingRequired("ANTHROPIC_API_KEY"));
        }

        let anthropic_model =
            std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let verify_ensemble_k = validate_ensemble_k(parse_env("VERIFY_ENSEMBLE_K", 3)?)?;
        let verify_max_claim_chars = parse_env("VERIFY_MAX_CLAIM_CHARS", 50_000)?;
        let database_path =
            std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/parallax.db".to_string());
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        let request_timeout_ms = parse_env("REQUEST_TIMEOUT_MS", 30_000)?;
        let max_retries = parse_env("MAX_RETRIES", 3)?;

        Ok(Self {
            anthropic_api_key,
            anthropic_model,
            verify_ensemble_k,
            verify_max_claim_chars,
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
    fn default_model_is_the_corpus_target() {
        assert_eq!(DEFAULT_MODEL, "claude-opus-4-8");
    }
}
