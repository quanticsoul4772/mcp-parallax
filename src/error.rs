//! Error types for the Parallax server, including the outcome taxonomy.
//!
//! The taxonomy is **one enum used twice** (see `specs/001-core-layer/data-model.md`
//! §6): every [`AppError`] maps to an [`Outcome`], which is both the basis of the
//! distinct error message surfaced to the MCP client and the `outcome` column on
//! the invocation record. A shared taxonomy makes the error surface and the
//! observability surface incapable of disagreeing.

use thiserror::Error;

/// Outcome classification for one tool invocation.
///
/// `ConfigError` is error-surface-only: it occurs at startup, before any
/// invocation exists, so it never appears on an invocation record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The invocation completed and returned a valid result.
    Success,
    /// The model provider refused the request (safety refusal).
    Refusal,
    /// The response was truncated before the schema completed (`max_tokens`).
    Truncation,
    /// The request exceeded the configured per-request timeout.
    Timeout,
    /// All retry attempts were exhausted without a usable response.
    RetriesExhausted,
    /// The tool input was rejected before any model call.
    InvalidInput,
    /// A response failed contract checks — local schema validation or an
    /// out-of-contract provider response (unexpected `stop_reason`, unparseable
    /// body).
    ValidationFailure,
    /// The search provider failed (research capability).
    SearchProvider,
    /// The embedding provider failed (memory capability).
    EmbeddingProvider,
    /// Startup configuration was missing or invalid (never recorded).
    ConfigError,
    /// The client abandoned the invocation before it completed.
    Cancelled,
}

impl Outcome {
    /// Parse the stable string form back into the taxonomy (storage read path).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Self::Success),
            "refusal" => Some(Self::Refusal),
            "truncation" => Some(Self::Truncation),
            "timeout" => Some(Self::Timeout),
            "retries_exhausted" => Some(Self::RetriesExhausted),
            "invalid_input" => Some(Self::InvalidInput),
            "validation_failure" => Some(Self::ValidationFailure),
            "search_provider" => Some(Self::SearchProvider),
            "embedding_provider" => Some(Self::EmbeddingProvider),
            "config_error" => Some(Self::ConfigError),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Stable string form — the `outcome` column value on invocation records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Refusal => "refusal",
            Self::Truncation => "truncation",
            Self::Timeout => "timeout",
            Self::RetriesExhausted => "retries_exhausted",
            Self::InvalidInput => "invalid_input",
            Self::ValidationFailure => "validation_failure",
            Self::SearchProvider => "search_provider",
            Self::EmbeddingProvider => "embedding_provider",
            Self::ConfigError => "config_error",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Top-level application error.
///
/// Every variant's message names its failure class distinctly (FR-007): an
/// operator must be able to identify the class from the error text alone.
#[derive(Debug, Error)]
pub enum AppError {
    /// A configuration problem.
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),

    /// A storage-layer problem (an operator/environment issue: bad database
    /// path, disk full).
    #[error("storage error: {0}")]
    Storage(String),

    /// A model-client failure outside the named classes: an out-of-contract
    /// provider response (the message says so at the construction site) or a
    /// transport-level error. Classifies as `validation_failure`.
    #[error("model client error: {0}")]
    Client(String),

    /// The model provider refused the request.
    #[error("model refused the request: {0}")]
    Refusal(String),

    /// The response was truncated before the schema completed.
    #[error("response truncated before the schema completed (max_tokens): {0}")]
    Truncation(String),

    /// A bounded operation exceeded its timeout (a provider request, or the
    /// in-process solver — the message names which).
    #[error("{what} timed out after {ms} ms")]
    Timeout {
        /// What timed out (e.g. "request", "solver (timeout or incompleteness)").
        what: &'static str,
        /// The configured timeout that elapsed.
        ms: u64,
    },

    /// All retries were exhausted.
    #[error("retries exhausted after {attempts} attempts; last error: {last}")]
    RetriesExhausted {
        /// Number of attempts made (initial try + retries).
        attempts: u32,
        /// The error from the final attempt.
        last: String,
    },

    /// The tool input was rejected before any model call.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// A result failed local validation: schema constraints the provider
    /// grammar cannot enforce, a semantic contract (e.g. unstick restating a
    /// tried item), or verification refuting a save.
    #[error("validation failure: {0}")]
    ValidationFailure(String),

    /// The client abandoned the invocation.
    #[error("invocation cancelled by the client")]
    Cancelled,

    /// The embedding provider failed (memory capability).
    #[error("embedding provider error: {0}")]
    EmbeddingProvider(String),

    /// The search provider failed (research capability).
    #[error("search provider error: {0}")]
    SearchProvider(String),
}

impl AppError {
    /// Classify this error into the outcome taxonomy.
    ///
    /// Total mapping: `Storage` classifies as `ConfigError` (an
    /// operator/environment problem — and a failed record write cannot be
    /// recorded anyway); `Client` classifies as `ValidationFailure` (the
    /// response was outside the contract).
    #[must_use]
    pub const fn outcome(&self) -> Outcome {
        match self {
            Self::Config(_) | Self::Storage(_) => Outcome::ConfigError,
            Self::Client(_) | Self::ValidationFailure(_) => Outcome::ValidationFailure,
            Self::Refusal(_) => Outcome::Refusal,
            Self::Truncation(_) => Outcome::Truncation,
            Self::Timeout { .. } => Outcome::Timeout,
            Self::RetriesExhausted { .. } => Outcome::RetriesExhausted,
            Self::InvalidInput(_) => Outcome::InvalidInput,
            Self::Cancelled => Outcome::Cancelled,
            Self::EmbeddingProvider(_) => Outcome::EmbeddingProvider,
            Self::SearchProvider(_) => Outcome::SearchProvider,
        }
    }
}

/// Configuration loading / validation errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A required environment variable was missing or empty.
    #[error("missing required environment variable: {0}")]
    MissingRequired(&'static str),

    /// An environment variable held a value that failed to parse.
    #[error("invalid value for environment variable: {0}")]
    Invalid(&'static str),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// FR-007 / SC-005: every failure class is identifiable from the error
    /// message alone — each message must contain a phrase unique to its class.
    #[test]
    fn messages_name_their_class_distinctly() {
        let cases: Vec<(AppError, &str)> = vec![
            (AppError::Refusal("policy".into()), "refused"),
            (AppError::Truncation("partial".into()), "truncated"),
            (
                AppError::Timeout {
                    what: "request",
                    ms: 30_000,
                },
                "timed out",
            ),
            (
                AppError::RetriesExhausted {
                    attempts: 3,
                    last: "503".into(),
                },
                "retries exhausted",
            ),
            (
                AppError::InvalidInput("empty claim".into()),
                "invalid input",
            ),
            (
                AppError::ValidationFailure("confidence out of range".into()),
                "validation failure",
            ),
            (AppError::Cancelled, "cancelled"),
            (
                AppError::EmbeddingProvider("voyage 503".into()),
                "embedding provider",
            ),
            (
                AppError::SearchProvider("brave 503".into()),
                "search provider",
            ),
            (
                AppError::Config(ConfigError::MissingRequired("ANTHROPIC_API_KEY")),
                "configuration error",
            ),
        ];

        let messages: Vec<String> = cases.iter().map(|(e, _)| e.to_string()).collect();
        for (i, ((_, marker), msg)) in cases.iter().zip(&messages).enumerate() {
            assert!(
                msg.contains(marker),
                "message {msg:?} must contain its class marker {marker:?}"
            );
            // The marker must be unique to this class across all messages.
            for (j, other) in messages.iter().enumerate() {
                if i != j {
                    assert!(
                        !other.contains(marker),
                        "marker {marker:?} is not unique: also appears in {other:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn outcome_mapping_is_total_and_stable() {
        assert_eq!(AppError::Refusal(String::new()).outcome(), Outcome::Refusal);
        assert_eq!(
            AppError::Truncation(String::new()).outcome(),
            Outcome::Truncation
        );
        assert_eq!(
            AppError::Timeout {
                what: "request",
                ms: 1
            }
            .outcome(),
            Outcome::Timeout
        );
        assert_eq!(
            AppError::RetriesExhausted {
                attempts: 1,
                last: String::new()
            }
            .outcome(),
            Outcome::RetriesExhausted
        );
        assert_eq!(
            AppError::InvalidInput(String::new()).outcome(),
            Outcome::InvalidInput
        );
        assert_eq!(
            AppError::ValidationFailure(String::new()).outcome(),
            Outcome::ValidationFailure
        );
        assert_eq!(
            AppError::Client(String::new()).outcome(),
            Outcome::ValidationFailure
        );
        assert_eq!(AppError::Cancelled.outcome(), Outcome::Cancelled);
        assert_eq!(
            AppError::EmbeddingProvider(String::new()).outcome(),
            Outcome::EmbeddingProvider
        );
        assert_eq!(
            AppError::SearchProvider(String::new()).outcome(),
            Outcome::SearchProvider
        );
        assert_eq!(
            AppError::Storage(String::new()).outcome(),
            Outcome::ConfigError
        );
    }

    #[test]
    fn outcome_strings_match_the_record_contract() {
        // Must stay in sync with specs/001-core-layer/contracts/invocation-record.schema.json
        assert_eq!(Outcome::Success.as_str(), "success");
        assert_eq!(Outcome::Refusal.as_str(), "refusal");
        assert_eq!(Outcome::Truncation.as_str(), "truncation");
        assert_eq!(Outcome::Timeout.as_str(), "timeout");
        assert_eq!(Outcome::RetriesExhausted.as_str(), "retries_exhausted");
        assert_eq!(Outcome::InvalidInput.as_str(), "invalid_input");
        assert_eq!(Outcome::ValidationFailure.as_str(), "validation_failure");
        assert_eq!(Outcome::ConfigError.as_str(), "config_error");
        assert_eq!(Outcome::Cancelled.as_str(), "cancelled");
        assert_eq!(Outcome::EmbeddingProvider.as_str(), "embedding_provider");
        assert_eq!(
            Outcome::parse("embedding_provider"),
            Some(Outcome::EmbeddingProvider)
        );
        assert_eq!(Outcome::SearchProvider.as_str(), "search_provider");
        assert_eq!(
            Outcome::parse("search_provider"),
            Some(Outcome::SearchProvider)
        );
    }
}
