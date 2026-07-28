//! The `Config` every acceptance example starts from.
//!
//! Seven examples hand-rolled the same 20-field literal. Fifteen of those
//! fields were identical in all seven; the five that differed each did so for a
//! reason — a live key rather than a placeholder, a longer timeout for the
//! examples that reach the network, the one confinement root, the two that need
//! embeddings.
//!
//! **Bendability is not lost, which was the objection to consolidating these.**
//! Struct update syntax leaves every field overridable, and an example that
//! bends one now says so in one line instead of restating nineteen it did not
//! mean to choose:
//!
//! ```ignore
//! mod common;
//! let config = Config { max_retries: 2, ..common::config() };
//! ```
//!
//! The placeholder key was written three different ways across the seven
//! (`test-key`, `dummy-acceptance`, `unused`) for a field none of those
//! examples reads — variance with no intent behind it, which is what a shared
//! base is for.
//!
//! Cargo does not treat this as an example target: it discovers
//! `examples/*.rs` and `examples/*/main.rs`, and this directory has neither.
//!
//! # Not covered by CI
//!
//! These examples are acceptance scripts run by hand against a mock or the live
//! API. `cargo test` never executes them, so `cargo build --examples` and the
//! lint gate are the only automated checks they get.

// Each example compiles this module into itself and uses only the parts it
// needs, so anything one example ignores would otherwise be dead code there.
#![allow(dead_code)]
// An acceptance script with no API key cannot do anything useful, and panicking
// at the point the key is read names the variable. The examples that call
// `live_config` already allow this at their own crate root; the allow has to
// sit here too because a module does not inherit it from the file that declares
// it when that allow is written per-function rather than crate-wide.
#![allow(clippy::expect_used)]

use mcp_parallax::config::{Config, DEFAULT_LOG_LEVEL, DEFAULT_MODEL, DEFAULT_VOYAGE_MODEL};

/// A `Config` reaching nothing: placeholder key, unroutable endpoint, in-memory
/// database, every capability gate off.
///
/// The endpoint is `127.0.0.1:1` so an example that escapes its mock fails by
/// connection refusal rather than reaching the live API — the failure 028's
/// review found the suite had been doing on a fixture key.
///
/// Model and log level come from the constants rather than repeating their
/// values, so a default that moves takes these with it. 041 guarantees
/// `DEFAULT_MODEL` has a price row, so a costed example stays costed.
pub fn config() -> Config {
    Config {
        anthropic_api_key: "test-key".into(),
        anthropic_model: DEFAULT_MODEL.into(),
        anthropic_api_base: "http://127.0.0.1:1".into(),
        routing: mcp_parallax::routing::RoutingTable::single(DEFAULT_MODEL),
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
        grounded_verify_max_bytes: 262_144,
        grounded_verify_max_locators: 64,
        database_path: ":memory:".into(),
        log_level: DEFAULT_LOG_LEVEL.into(),
        request_timeout_ms: 5_000,
        max_retries: 1,
    }
}

/// [`config`] for the examples that talk to the real API: keys read from the
/// environment, and a timeout that a reasoning model can actually finish in.
///
/// # Panics
///
/// Panics naming the variable when a required key is unset — these examples
/// cannot do anything useful without one, and a placeholder would fail later
/// with a less obvious message.
pub fn live_config() -> Config {
    Config {
        anthropic_api_key: std::env::var("ANTHROPIC_API_KEY")
            .expect("ANTHROPIC_API_KEY must be set to run this example"),
        request_timeout_ms: 30_000,
        max_retries: 2,
        ..config()
    }
}
