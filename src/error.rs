//! Error types for the Parallax server.

use thiserror::Error;

/// Top-level application error.
#[derive(Debug, Error)]
pub enum AppError {
    /// A configuration problem.
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),

    /// A storage-layer problem.
    #[error("storage error: {0}")]
    Storage(String),

    /// An upstream model-client problem.
    #[error("model client error: {0}")]
    Client(String),
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
