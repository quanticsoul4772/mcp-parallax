//! Runtime configuration, sourced entirely from the environment.

use crate::error::ConfigError;

/// Server configuration. Every field is sourced from an environment variable so
/// the binary is configured the same way in every host (Claude Code / Desktop).
#[derive(Debug, Clone)]
pub struct Config {
    /// Anthropic API key (required). `ANTHROPIC_API_KEY`.
    pub anthropic_api_key: String,
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
    /// but fails to parse.
    pub fn from_env() -> Result<Self, ConfigError> {
        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ConfigError::MissingRequired("ANTHROPIC_API_KEY"))?;
        if anthropic_api_key.trim().is_empty() {
            return Err(ConfigError::MissingRequired("ANTHROPIC_API_KEY"));
        }

        let database_path =
            std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/parallax.db".to_string());
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        let request_timeout_ms = parse_env("REQUEST_TIMEOUT_MS", 30_000)?;
        let max_retries = parse_env("MAX_RETRIES", 3)?;

        Ok(Self {
            anthropic_api_key,
            database_path,
            log_level,
            request_timeout_ms,
            max_retries,
        })
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
}
