//! Source-grounded verification (008): mechanically-assembled verbatim evidence.
//!
//! The caller names source locators within a single configured root; the
//! [`reader::SystemSourceReader`] reads the verbatim text (confined,
//! text-only), and [`assemble::assemble`] resolves every locator all-or-nothing
//! into the evidence the stance-blind ensemble judges. The model never authors
//! the evidence — it is pulled from disk — so the caller cannot paraphrase or
//! smuggle a conclusion into it.

pub mod assemble;
pub mod glob;
pub mod reader;

use crate::error::AppError;
use serde::{Deserialize, Serialize};

/// A caller-supplied reference to evidence.
///
/// Interpreted within the configured source root: a locator is *either* an
/// exact `path` (optionally with a line range, 008) *or* a `glob` pattern (009)
/// — never both.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SourceLocator {
    /// Exact relative path within the configured source root. Mutually
    /// exclusive with `glob`.
    pub path: Option<String>,
    /// Extended-glob pattern (009), expanded within the root. Mutually
    /// exclusive with `path`; may not carry a line range.
    pub glob: Option<String>,
    /// 1-based inclusive start line (only with `path`). With `end_line`,
    /// restricts to a range; both omitted reads the whole file.
    pub start_line: Option<u32>,
    /// 1-based inclusive end line (only with `path`). Must be paired with
    /// `start_line`.
    pub end_line: Option<u32>,
}

/// The validated shape of a locator: exactly one of an exact path or a glob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocatorKind<'a> {
    /// An exact path, optionally restricted to a line range.
    Path {
        /// The relative path.
        path: &'a str,
        /// 1-based inclusive start line, if a range was given.
        start_line: Option<u32>,
        /// 1-based inclusive end line, if a range was given.
        end_line: Option<u32>,
    },
    /// A glob pattern to expand within the root.
    Glob {
        /// The extended-glob pattern.
        pattern: &'a str,
    },
}

impl SourceLocator {
    /// Validate and classify the locator (009 FR-007, data-model v2).
    ///
    /// # Errors
    ///
    /// [`AppError::InvalidInput`] when neither or both of `path`/`glob` are
    /// given, or a `glob` carries a line range.
    pub fn classify(&self) -> Result<LocatorKind<'_>, AppError> {
        match (self.path.as_deref(), self.glob.as_deref()) {
            (Some(path), None) => Ok(LocatorKind::Path {
                path,
                start_line: self.start_line,
                end_line: self.end_line,
            }),
            (None, Some(pattern)) => {
                if self.start_line.is_some() || self.end_line.is_some() {
                    return Err(AppError::InvalidInput(format!(
                        "a line range is not allowed with a glob: {pattern}"
                    )));
                }
                Ok(LocatorKind::Glob { pattern })
            }
            (Some(_), Some(_)) => Err(AppError::InvalidInput(
                "locator cannot give both a path and a glob".to_string(),
            )),
            (None, None) => Err(AppError::InvalidInput(
                "locator must give a path or a glob".to_string(),
            )),
        }
    }
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
