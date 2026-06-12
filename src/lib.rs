//! # Parallax
//!
//! An LLM-augmentation MCP server. When Claude calls a reasoning tool, Claude
//! is calling Claude — so the value is not reasoning *harder*. The value is an
//! external, independent pass that catches the ways the model reliably goes
//! wrong and **cannot see from inside its own context**: a catalog of
//! correctives for the calling model's predictable failure modes —
//! *metacognition the model can't run on itself.*
//!
//! The north-star architecture (four layers — cognitive correctives, watchdog,
//! memory/experience, deterministic/symbolic) is documented in
//! `docs/design/NEW_SERVER_DESIGN.md`.
//!
//! ## Status
//!
//! Scaffold. This crate currently provides the foundation only: configuration,
//! error types, and the mockable trait boundaries the rest is composed from.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────┐     stdin      ┌─────────────────┐──────▶ Anthropic API
//! │ Claude Code │───────────────▶│  Parallax (Rust)│
//! │ or Desktop  │◀───────────────│   MCP server    │──────▶ SQLite
//! └─────────────┘     stdout     └─────────────────┘
//! ```

#![forbid(unsafe_code)]
// Production code must not panic via unwrap/expect; test modules opt out with a
// local `#[allow(...)]`. This makes the guarantee compiler-enforced, not custom.
#![deny(clippy::unwrap_used, clippy::expect_used)]
// stdout is the MCP JSON-RPC channel — a stray print corrupts the protocol.
#![deny(clippy::print_stdout, clippy::dbg_macro)]
#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
// Allowed pedantic/nursery lints for practical reasons.
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::doc_markdown)] // Backticks in docs not required for all identifiers

pub mod client;
pub mod config;
pub mod error;
pub mod memory;
pub mod modes;
pub mod research;
pub mod schema;
pub mod server;
pub mod storage;
pub mod telemetry;
pub mod traits;
