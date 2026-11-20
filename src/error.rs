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
    /// The model provider refused the request with an HTTP 4xx other than
    /// 429 (billing, authentication, access, unknown model, rejected request)
    /// - 057.
    ProviderRejected,
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
    /// Every outcome, in declaration order. The contract-binding test compares
    /// this list (minus `ConfigError`, which is never recorded) with the
    /// published lists, so a new value cannot ship unlisted (057).
    pub const ALL: [Self; 12] = [
        Self::Success,
        Self::Refusal,
        Self::Truncation,
        Self::Timeout,
        Self::RetriesExhausted,
        Self::InvalidInput,
        Self::ValidationFailure,
        Self::ProviderRejected,
        Self::SearchProvider,
        Self::EmbeddingProvider,
        Self::ConfigError,
        Self::Cancelled,
    ];

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
            "provider_rejected" => Some(Self::ProviderRejected),
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
            Self::ProviderRejected => "provider_rejected",
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

    /// The model provider refused the request with an HTTP 4xx other than
    /// 429; terminal, not retried, not billed (057).
    #[error("provider rejected the request: HTTP {status}: {detail}")]
    ProviderRejected {
        /// The provider's HTTP status.
        status: u16,
        /// The provider's response body, plus any remedy hint.
        detail: String,
    },

    /// The client abandoned the invocation.
    #[error("invocation cancelled by the client")]
    Cancelled,

    /// The embedding provider failed (memory capability).
    #[error("embedding provider error: {0}")]
    EmbeddingProvider(String),

    /// The search provider failed (research capability).
    #[error("search provider error: {0}")]
    SearchProvider(String),

    /// A failure that nonetheless consumed billable tokens (020).
    ///
    /// A truncation or a refusal is an HTTP 200 response: the provider ran the
    /// model, billed for it, and returned a `stop_reason` the contract cannot
    /// use. Discarding that usage under-reports spend, and — because the
    /// truncated calls are exactly the ones that would say how much output
    /// headroom is needed — makes the output ceiling unsizable from data.
    ///
    /// The same applies above the single call: when an ensemble fails to reach
    /// quorum, the passes that *did* complete were still billed, as were the
    /// sibling failures that lost the dominant-class vote.
    ///
    /// This wraps rather than extending each variant, so the outcome taxonomy
    /// is unchanged — [`AppError::outcome`] and [`std::fmt::Display`] both
    /// delegate to `source`, and [`AppError::root`] recovers the wrapped error
    /// for matching. Tokens are raw rather than per-model because the
    /// producers (a client, an aggregator) know the count and the recorder
    /// knows the model; attributing them here would mean guessing.
    #[error("{source}")]
    Metered {
        /// The failure that actually occurred.
        source: Box<Self>,
        /// Input tokens billed before the failure.
        input_tokens: u64,
        /// Output tokens billed before the failure.
        output_tokens: u64,
    },
}

impl AppError {
    /// Attach billed tokens to this failure, merging if some are already
    /// attached. Zero tokens are not wrapped — an error that cost nothing
    /// stays exactly the error it was.
    #[must_use]
    pub fn metered(self, input_tokens: u64, output_tokens: u64) -> Self {
        if input_tokens == 0 && output_tokens == 0 {
            return self;
        }
        match self {
            Self::Metered {
                source,
                input_tokens: have_in,
                output_tokens: have_out,
            } => Self::Metered {
                source,
                input_tokens: have_in.saturating_add(input_tokens),
                output_tokens: have_out.saturating_add(output_tokens),
            },
            other => Self::Metered {
                source: Box::new(other),
                input_tokens,
                output_tokens,
            },
        }
    }

    /// Tokens billed before this failure, `(0, 0)` when none are attached.
    #[must_use]
    pub const fn billed(&self) -> (u64, u64) {
        match self {
            Self::Metered {
                input_tokens,
                output_tokens,
                ..
            } => (*input_tokens, *output_tokens),
            _ => (0, 0),
        }
    }

    /// The underlying failure, unwrapping any attached usage. Match on this
    /// rather than on the error directly — metering is orthogonal to class.
    #[must_use]
    pub fn root(&self) -> &Self {
        match self {
            Self::Metered { source, .. } => source.root(),
            other => other,
        }
    }

    /// Classify this error into the outcome taxonomy.
    ///
    /// Total mapping: `Storage` classifies as `ConfigError` (an
    /// operator/environment problem — and a failed record write cannot be
    /// recorded anyway); `Client` classifies as `ValidationFailure` (the
    /// response was outside the contract); `Metered` delegates to the failure
    /// it wraps, so attaching usage never changes an error's class.
    #[must_use]
    pub fn outcome(&self) -> Outcome {
        match self {
            Self::Metered { source, .. } => source.outcome(),
            Self::Config(_) | Self::Storage(_) => Outcome::ConfigError,
            Self::Client(_) | Self::ValidationFailure(_) => Outcome::ValidationFailure,
            Self::ProviderRejected { .. } => Outcome::ProviderRejected,
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

    /// A `PARALLAX_MODEL_*` variable was unrecognised or unusable (018 FR-006,
    /// FR-006a). Carries an owned message because the offending variable name
    /// is discovered at runtime — a misspelled suffix is not a `&'static str`.
    #[error("invalid routing configuration: {0}")]
    Routing(String),

    /// `GROUNDED_VERIFY_ROOT` is, or is inside, a folder whose files are all
    /// refused as likely secrets (055 FR-003). Carries the folder (or the
    /// pair of folders) as written and the root as a caller would type it.
    #[error(
        "GROUNDED_VERIFY_ROOT is, or is inside, {folder}, whose files are refused as likely \
         secrets: {root}"
    )]
    SecretRoot {
        /// The refused folder, or the two folders of a pair joined by `/`.
        folder: String,
        /// The root, as a caller would type it.
        root: String,
    },

    /// `GROUNDED_VERIFY_ROOT` resolves to a folder too wide to read from: a
    /// drive root or the user's home folder. With the root set to `.`, that
    /// happens when a session is started there.
    #[error(
        "GROUNDED_VERIFY_ROOT resolves to {what} ({root}): grounded_verify would read every \
         file under it - start the session inside a project folder"
    )]
    WideRoot {
        /// "a drive root" or "the home folder".
        what: &'static str,
        /// The root, as a caller would type it.
        root: String,
    },
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

    /// 057: a provider HTTP rejection is its own outcome, and its message
    /// keeps the status and the provider's text.
    #[test]
    fn a_provider_rejection_is_its_own_outcome() {
        let err = AppError::ProviderRejected {
            status: 400,
            detail: "credit balance too low".into(),
        };
        assert_eq!(err.outcome(), Outcome::ProviderRejected);
        assert_eq!(Outcome::ProviderRejected.as_str(), "provider_rejected");
        let text = err.to_string();
        assert!(
            text.contains("HTTP 400") && text.contains("credit balance too low"),
            "{text}"
        );
    }

    /// The outcomes that can appear on a record: all of them except
    /// `config_error`, which occurs at startup before any invocation exists.
    fn recorded_outcomes() -> std::collections::BTreeSet<String> {
        Outcome::ALL
            .iter()
            .filter(|o| **o != Outcome::ConfigError)
            .map(|o| o.as_str().to_string())
            .collect()
    }

    /// 057: the published outcome lists are read from the files and compared
    /// with the code. The test this replaces compared `as_str()` with
    /// literals and only claimed, in a comment, to stay in sync with the
    /// record schema — it never opened it, so the schema could drift unseen.
    #[test]
    fn the_published_outcome_lists_equal_the_recorded_outcomes() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../specs/001-core-layer/contracts/invocation-record.schema.json"
        ))
        .unwrap();
        let record: std::collections::BTreeSet<String> = schema["properties"]["outcome"]["enum"]
            .as_array()
            .expect("the record schema has an outcome enum")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(
            !record.is_empty(),
            "the record schema's outcome enum is empty"
        );
        assert_eq!(record, recorded_outcomes(), "invocation-record.schema.json");

        let telemetry = include_str!("../specs/007-observability-layer/contracts/telemetry.md");
        let rows: Vec<&str> = telemetry
            .lines()
            .filter(|l| l.trim_start().starts_with("| `parallax.outcome`"))
            .collect();
        assert_eq!(rows.len(), 1, "telemetry.md has one parallax.outcome row");
        let exported: std::collections::BTreeSet<String> = rows[0]
            .split('`')
            .skip(1)
            .step_by(2)
            .skip(1) // the attribute name itself
            .map(str::to_string)
            .collect();
        assert!(
            !exported.is_empty(),
            "the parallax.outcome row lists no values"
        );
        assert_eq!(
            exported,
            recorded_outcomes(),
            "telemetry.md parallax.outcome"
        );
    }

    /// Every outcome round-trips through its stable string, `ALL` holds each
    /// once, and the exhaustive match below stops a new variant compiling
    /// until it is given a place in `ALL`.
    #[test]
    fn every_outcome_round_trips_and_is_listed_once() {
        const fn position(o: Outcome) -> usize {
            match o {
                Outcome::Success => 0,
                Outcome::Refusal => 1,
                Outcome::Truncation => 2,
                Outcome::Timeout => 3,
                Outcome::RetriesExhausted => 4,
                Outcome::InvalidInput => 5,
                Outcome::ValidationFailure => 6,
                Outcome::ProviderRejected => 7,
                Outcome::SearchProvider => 8,
                Outcome::EmbeddingProvider => 9,
                Outcome::ConfigError => 10,
                Outcome::Cancelled => 11,
            }
        }
        for (i, o) in Outcome::ALL.iter().enumerate() {
            assert_eq!(position(*o), i, "{o:?} is out of place in ALL");
            assert_eq!(Outcome::parse(o.as_str()), Some(*o), "{o:?}");
        }
    }
}
