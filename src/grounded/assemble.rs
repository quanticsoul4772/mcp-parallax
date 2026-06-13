//! The assembly stage (008 D4): resolve every locator all-or-nothing into the
//! verbatim evidence the ensemble judges, plus the audit manifest.
//!
//! Pure over a [`SourceReader`] — no filesystem here, so it tests through a
//! mock. A single failing locator aborts the whole call (008 FR-009); no model
//! pass ever runs on a partially-resolved evidence set.

use crate::error::AppError;
use crate::grounded::{AssemblyLimits, ManifestEntry, SourceLocator};
use crate::traits::source::SourceReader;

/// The verbatim evidence assembled from the resolved locators, plus its
/// manifest. `text` is the only context (besides the claim) the passes receive.
#[derive(Debug, Clone)]
pub struct AssembledEvidence {
    /// The verbatim evidence, each span framed with a deterministic
    /// server-generated provenance header.
    pub text: String,
    /// The audit manifest — one entry per resolved locator, in order.
    pub manifest: Vec<ManifestEntry>,
}

/// Resolve all locators all-or-nothing.
///
/// # Errors
///
/// [`AppError::InvalidInput`], naming the offending locator, for: an empty
/// locator set, more than `max_locators`, any locator the reader rejects
/// (missing/empty/out-of-range/non-text/out-of-root), or assembled bytes over
/// `max_bytes`. On any error nothing is returned — the call is all-or-nothing.
pub fn assemble(
    reader: &dyn SourceReader,
    locators: &[SourceLocator],
    limits: AssemblyLimits,
) -> Result<AssembledEvidence, AppError> {
    if locators.is_empty() {
        return Err(AppError::InvalidInput(
            "grounded_verify requires at least one locator".to_string(),
        ));
    }
    if locators.len() > limits.max_locators {
        return Err(AppError::InvalidInput(format!(
            "too many locators ({}, max {})",
            locators.len(),
            limits.max_locators
        )));
    }

    let mut manifest = Vec::with_capacity(locators.len());
    let mut sections = Vec::with_capacity(locators.len());
    let mut total: usize = 0;
    for loc in locators {
        let content = reader.read(&loc.path, loc.start_line, loc.end_line)?;
        total = total.saturating_add(usize::try_from(content.bytes).unwrap_or(usize::MAX));
        if total > limits.max_bytes {
            return Err(AppError::InvalidInput(format!(
                "assembled evidence exceeds {} bytes (at locator '{}')",
                limits.max_bytes, loc.path
            )));
        }
        sections.push(format!("===== {} =====\n{}", header(loc), content.text));
        manifest.push(ManifestEntry {
            path: loc.path.clone(),
            start_line: loc.start_line,
            end_line: loc.end_line,
            bytes: content.bytes,
        });
    }

    Ok(AssembledEvidence {
        text: sections.join("\n\n"),
        manifest,
    })
}

/// A deterministic provenance header for one span — server-generated framing,
/// not caller prose.
fn header(loc: &SourceLocator) -> String {
    match (loc.start_line, loc.end_line) {
        (Some(start), Some(end)) => format!("{} (lines {start}-{end})", loc.path),
        _ => loc.path.clone(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::source::{MockSourceReader, SourceContent};

    fn limits() -> AssemblyLimits {
        AssemblyLimits {
            max_bytes: 262_144,
            max_locators: 64,
        }
    }

    fn loc(path: &str) -> SourceLocator {
        SourceLocator {
            path: path.to_string(),
            start_line: None,
            end_line: None,
        }
    }

    fn ranged(path: &str, start: u32, end: u32) -> SourceLocator {
        SourceLocator {
            path: path.to_string(),
            start_line: Some(start),
            end_line: Some(end),
        }
    }

    /// A reader that serves `body:<path>` (10 bytes-ish) for every path except
    /// any in `fails`, which return a named not-found error.
    fn reader_failing(fails: &'static [&'static str]) -> MockSourceReader {
        let mut mock = MockSourceReader::new();
        mock.expect_read().returning(move |path, _, _| {
            if fails.contains(&path) {
                Err(AppError::InvalidInput(format!("source not found: {path}")))
            } else {
                let text = format!("body:{path}");
                let bytes = text.len() as u64;
                Ok(SourceContent { text, bytes })
            }
        });
        mock
    }

    #[test]
    fn assembles_verbatim_with_manifest_in_order() {
        let reader = reader_failing(&[]);
        let out = assemble(&reader, &[loc("a.rs"), ranged("b.rs", 2, 4)], limits()).unwrap();
        assert!(out.text.contains("body:a.rs"));
        assert!(out.text.contains("body:b.rs"));
        assert!(out.text.contains("b.rs (lines 2-4)"));
        assert_eq!(out.manifest.len(), 2);
        assert_eq!(out.manifest[0].path, "a.rs");
        assert_eq!(out.manifest[1].path, "b.rs");
        assert_eq!(out.manifest[1].start_line, Some(2));
        assert_eq!(out.manifest[1].end_line, Some(4));
        assert!(out.manifest[0].bytes > 0);
    }

    #[test]
    fn any_failing_locator_aborts_the_whole_call_named() {
        let reader = reader_failing(&["gone.rs"]);
        let err = assemble(&reader, &[loc("a.rs"), loc("gone.rs")], limits()).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
        assert!(err.to_string().contains("source not found: gone.rs"));
    }

    #[test]
    fn empty_locator_set_is_rejected() {
        let reader = reader_failing(&[]);
        let err = assemble(&reader, &[], limits()).unwrap_err();
        assert!(err.to_string().contains("at least one locator"));
    }

    #[test]
    fn too_many_locators_is_rejected() {
        let reader = reader_failing(&[]);
        let small = AssemblyLimits {
            max_bytes: 262_144,
            max_locators: 2,
        };
        let locs = vec![loc("a"), loc("b"), loc("c")];
        let err = assemble(&reader, &locs, small).unwrap_err();
        assert!(err.to_string().contains("too many locators"));
    }

    #[test]
    fn byte_ceiling_is_enforced_and_names_the_overflow() {
        let mut mock = MockSourceReader::new();
        mock.expect_read().returning(|_path, _, _| {
            Ok(SourceContent {
                text: "x".repeat(100),
                bytes: 100,
            })
        });
        let tight = AssemblyLimits {
            max_bytes: 150,
            max_locators: 64,
        };
        let err = assemble(&mock, &[loc("a.rs"), loc("b.rs")], tight).unwrap_err();
        assert!(err.to_string().contains("exceeds 150 bytes"));
        assert!(err.to_string().contains("b.rs"));
    }

    #[test]
    fn assembly_is_deterministic_for_the_same_inputs() {
        let reader = reader_failing(&[]);
        let locs = [loc("a.rs"), ranged("b.rs", 1, 2), loc("c.rs")];
        let first = assemble(&reader, &locs, limits()).unwrap();
        let reader2 = reader_failing(&[]);
        let second = assemble(&reader2, &locs, limits()).unwrap();
        assert_eq!(first.text, second.text);
        assert_eq!(first.manifest, second.manifest);
    }
}
