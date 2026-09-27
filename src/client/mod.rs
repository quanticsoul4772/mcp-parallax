//! Provider clients implementing the [`crate::traits::client::ModelClient`] seam.

pub mod anthropic;
pub mod brave;
pub mod openai_compat;
pub mod pool;
pub mod voyage;

pub use anthropic::AnthropicClient;
pub use brave::BraveClient;
pub use openai_compat::OpenAiCompatClient;
pub use voyage::VoyageClient;

use crate::config::{Backend, Config};
use crate::routing::Effort;
use crate::traits::client::ModelClient;
use std::sync::Arc;

/// Build the model client for one routed `(model, effort)` pair on the
/// configured backend (BYOM design §3.1).
///
/// This is the single place the backend choice is expressed: every entry the
/// [`pool::ClientPool`] materialises — including the eager `(model, effort)`
/// cross product — comes through here, so a backend switch is a config change
/// and never a call-site edit.
#[must_use]
pub fn build_model_client(
    config: &Config,
    http: &reqwest::Client,
    model: &str,
    effort: Option<Effort>,
) -> Arc<dyn ModelClient> {
    match config.backend {
        Backend::Anthropic => Arc::new(AnthropicClient::with_http_client(
            config,
            http,
            &config.anthropic_api_base,
            model,
            effort,
        )),
        Backend::OpenAiCompat => Arc::new(OpenAiCompatClient::with_http_client(
            config,
            http,
            &config.openai_api_base,
            model,
            effort,
        )),
    }
}

/// The default-model client the server is constructed with (the injected
/// seam): the selected backend's client for [`Config::default_model`] at no
/// explicit effort.
#[must_use]
pub fn default_model_client(config: &Config) -> Arc<dyn ModelClient> {
    build_model_client(
        config,
        &reqwest::Client::new(),
        config.default_model(),
        None,
    )
}

#[cfg(test)]
mod call_site_tests;
#[cfg(test)]
mod parity_tests;
