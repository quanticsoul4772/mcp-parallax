//! Source-grounded verification (008): mechanically-assembled verbatim evidence.
//!
//! The caller names source locators within a single configured root; the
//! [`reader::SystemSourceReader`] reads the verbatim text (confined,
//! text-only), and [`assemble::assemble`] resolves every locator all-or-nothing
//! into the evidence the stance-blind ensemble judges. The model never authors
//! the evidence — it is pulled from disk — so the caller cannot paraphrase or
//! smuggle a conclusion into it.

pub mod assemble;
pub mod reader;

use serde::{Deserialize, Serialize};

/// A caller-supplied reference to evidence, interpreted within the configured
/// source root. Globs are deferred (008 clarification) — `path` is a literal.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SourceLocator {
    /// Relative path within the configured source root.
    pub path: String,
    /// 1-based inclusive start line. With `end_line`, restricts to a range;
    /// both omitted reads the whole file. Must be paired with `end_line`.
    pub start_line: Option<u32>,
    /// 1-based inclusive end line. Must be paired with `start_line`.
    pub end_line: Option<u32>,
}

/// One entry of the evidence manifest — the auditable record of exactly what
/// was read for one resolved locator (008 FR-008). Server-assembled, never
/// model-authored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ManifestEntry {
    /// The locator's path, as given.
    pub path: String,
    /// The inclusive start line read (absent for a whole-file read).
    pub start_line: Option<u32>,
    /// The inclusive end line read (absent for a whole-file read).
    pub end_line: Option<u32>,
    /// Byte length of the text read for this locator.
    pub bytes: u64,
}

/// Per-call bounds from configuration (008 D6).
#[derive(Debug, Clone, Copy)]
pub struct AssemblyLimits {
    /// `GROUNDED_VERIFY_MAX_BYTES` — total assembled-evidence ceiling.
    pub max_bytes: usize,
    /// `GROUNDED_VERIFY_MAX_LOCATORS` — locators accepted per call.
    pub max_locators: usize,
}
