//! The assembly stage (008 D4, extended by 009): resolve every locator
//! all-or-nothing into the verbatim evidence the ensemble judges, plus the
//! audit manifest.
//!
//! Pure over a [`SourceReader`] — no filesystem here, so it tests through a
//! mock. A single failing locator aborts the whole call (008 FR-009); no model
//! pass ever runs on a partially-resolved evidence set. A glob locator (009) is
//! expanded into its concrete files before the read loop, subject to the same
//! locator-count and byte ceilings.

use crate::error::AppError;
use crate::grounded::glob::expand::expand;
use crate::grounded::{AssemblyLimits, LocatorKind, ManifestEntry, SourceLocator};
use crate::traits::source::SourceReader;

/// The verbatim evidence assembled from the resolved locators, plus its
/// manifest. `text` is the only context (besides the claim) the passes receive.
#[derive(Debug, Clone)]
pub struct AssembledEvidence {
    /// The verbatim evidence, each span framed with a deterministic
    /// server-generated provenance header.
    pub text: String,
    /// The audit manifest — one entry per resolved file, in order.
    pub manifest: Vec<ManifestEntry>,
    /// The raw per-read-unit content, in order — the verbatim source as the
    /// reader returned it, **without** the provenance headers framing `text`.
    /// The compute-settle path (011) counts over this, never over `text`; a
    /// single unit (`units.len() == 1`) is the single-source gate.
    pub units: Vec<RawUnit>,
}

/// The verbatim content of one resolved read unit (011) — what a count runs over.
#[derive(Debug, Clone)]
pub struct RawUnit {
    /// The raw source text as read (no provenance header).
    pub text: String,
    /// The reader's byte length for this unit (mirrors the manifest entry).
    pub bytes: u64,
}

/// One concrete file to read: a path with an optional line range.
type ReadUnit = (String, Option<u32>, Option<u32>);

/// Resolve all locators all-or-nothing, expanding globs into concrete files.
///
/// # Errors
///
/// [`AppError::InvalidInput`], naming the offending locator, for: an empty
/// locator set, a glob/range conflict, a glob that matches nothing, more than
/// `max_locators` after expansion, any locator the reader rejects, or assembled
/// bytes over `max_bytes`. On any error nothing is returned.
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

    // Phase 1: classify + expand into concrete read units, enforcing the
    // locator-count ceiling against the post-expansion total (009 FR-006).
    let mut units: Vec<ReadUnit> = Vec::new();
    for loc in locators {
        match loc.classify()? {
            LocatorKind::Path {
                path,
                start_line,
                end_line,
            } => units.push((path.to_string(), start_line, end_line)),
            LocatorKind::Glob { pattern } => {
                for rel in expand(reader, pattern)? {
                    units.push((rel, None, None));
                }
            }
        }
        if units.len() > limits.max_locators {
            return Err(AppError::InvalidInput(format!(
                "too many locators ({}, max {}) after glob expansion",
                units.len(),
                limits.max_locators
            )));
        }
    }

    // Phase 2: read every unit all-or-nothing (008's read/manifest/byte loop).
    let mut manifest = Vec::with_capacity(units.len());
    let mut sections = Vec::with_capacity(units.len());
    let mut raw_units = Vec::with_capacity(units.len());
    let mut total: usize = 0;
    for (path, start, end) in &units {
        let content = reader.read(path, *start, *end)?;
        total = total.saturating_add(usize::try_from(content.bytes).unwrap_or(usize::MAX));
        if total > limits.max_bytes {
            return Err(AppError::InvalidInput(format!(
                "assembled evidence exceeds {} bytes (at locator '{}')",
                limits.max_bytes, path
            )));
        }
        sections.push(format!(
            "===== {} =====\n{}",
            header(path, *start, *end),
            content.text
        ));
        manifest.push(ManifestEntry {
            path: path.clone(),
            start_line: *start,
            end_line: *end,
            bytes: content.bytes,
        });
        // The verbatim source, kept separate from the framed `text` so the
        // compute-settle count (011) never includes the provenance headers.
        raw_units.push(RawUnit {
            text: content.text,
            bytes: content.bytes,
        });
    }

    Ok(AssembledEvidence {
        text: sections.join("\n\n"),
        manifest,
        units: raw_units,
    })
}

/// A deterministic provenance header for one span — server-generated framing,
/// not caller prose.
fn header(path: &str, start: Option<u32>, end: Option<u32>) -> String {
    match (start, end) {
        (Some(start), Some(end)) => format!("{path} (lines {start}-{end})"),
        _ => path.to_string(),
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

    fn path_loc(path: &str) -> SourceLocator {
        SourceLocator {
            path: Some(path.to_string()),
            glob: None,
            start_line: None,
            end_line: None,
        }
    }

    fn ranged(path: &str, start: u32, end: u32) -> SourceLocator {
        SourceLocator {
            path: Some(path.to_string()),
            glob: None,
            start_line: Some(start),
            end_line: Some(end),
        }
    }

    fn glob_loc(pattern: &str) -> SourceLocator {
        SourceLocator {
            path: None,
            glob: Some(pattern.to_string()),
            start_line: None,
            end_line: None,
        }
    }

    /// A reader that serves `body:<path>` for every read except any in `fails`,
    /// and (for glob tests) lists `list`.
    fn reader_failing(
        fails: &'static [&'static str],
        list: &'static [&'static str],
    ) -> MockSourceReader {
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
        mock.expect_list_files()
            .returning(move || Ok(list.iter().map(|s| (*s).to_string()).collect()));
        mock
    }

    #[test]
    fn assembles_verbatim_with_manifest_in_order() {
        let reader = reader_failing(&[], &[]);
        let out = assemble(&reader, &[path_loc("a.rs"), ranged("b.rs", 2, 4)], limits()).unwrap();
        assert!(out.text.contains("body:a.rs"));
        assert!(out.text.contains("b.rs (lines 2-4)"));
        assert_eq!(out.manifest.len(), 2);
        assert_eq!(out.manifest[1].start_line, Some(2));
    }

    #[test]
    fn a_glob_expands_into_concrete_manifest_entries() {
        let reader = reader_failing(&[], &["src/a.rs", "src/b.rs", "src/x.txt"]);
        let out = assemble(&reader, &[glob_loc("src/*.rs")], limits()).unwrap();
        assert_eq!(out.manifest.len(), 2);
        assert_eq!(out.manifest[0].path, "src/a.rs");
        assert_eq!(out.manifest[1].path, "src/b.rs");
        assert!(out.manifest[0].start_line.is_none());
    }

    #[test]
    fn a_glob_and_an_exact_path_mix_in_one_call() {
        let reader = reader_failing(&[], &["src/a.rs"]);
        let out = assemble(
            &reader,
            &[glob_loc("src/*.rs"), path_loc("README.md")],
            limits(),
        )
        .unwrap();
        let paths: Vec<&str> = out.manifest.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, vec!["src/a.rs", "README.md"]);
    }

    #[test]
    fn a_glob_with_a_range_is_rejected() {
        let reader = reader_failing(&[], &["src/a.rs"]);
        let loc = SourceLocator {
            path: None,
            glob: Some("src/*.rs".to_string()),
            start_line: Some(1),
            end_line: Some(2),
        };
        let err = assemble(&reader, &[loc], limits()).unwrap_err();
        assert!(err
            .to_string()
            .contains("a line range is not allowed with a glob"));
    }

    #[test]
    fn a_zero_match_glob_aborts_named() {
        let reader = reader_failing(&[], &["src/a.rs"]);
        let err = assemble(&reader, &[glob_loc("nope/*.rs")], limits()).unwrap_err();
        assert!(err.to_string().contains("matched no files"));
    }

    #[test]
    fn expansion_past_the_locator_ceiling_is_rejected() {
        let reader = reader_failing(&[], &["a.rs", "b.rs", "c.rs"]);
        let tight = AssemblyLimits {
            max_bytes: 262_144,
            max_locators: 2,
        };
        let err = assemble(&reader, &[glob_loc("*.rs")], tight).unwrap_err();
        assert!(err.to_string().contains("too many locators"));
    }

    #[test]
    fn any_failing_unit_aborts_the_whole_call_named() {
        let reader = reader_failing(&["gone.rs"], &[]);
        let err =
            assemble(&reader, &[path_loc("a.rs"), path_loc("gone.rs")], limits()).unwrap_err();
        assert!(err.to_string().contains("source not found: gone.rs"));
    }

    #[test]
    fn empty_locator_set_is_rejected() {
        let reader = reader_failing(&[], &[]);
        let err = assemble(&reader, &[], limits()).unwrap_err();
        assert!(err.to_string().contains("at least one locator"));
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
        let err = assemble(&mock, &[path_loc("a.rs"), path_loc("b.rs")], tight).unwrap_err();
        assert!(err.to_string().contains("exceeds 150 bytes"));
    }

    #[test]
    fn assembly_is_deterministic_for_the_same_inputs() {
        let reader = reader_failing(&[], &[]);
        let locs = [path_loc("a.rs"), ranged("b.rs", 1, 2)];
        let first = assemble(&reader, &locs, limits()).unwrap();
        let reader2 = reader_failing(&[], &[]);
        let second = assemble(&reader2, &locs, limits()).unwrap();
        assert_eq!(first.text, second.text);
        assert_eq!(first.manifest, second.manifest);
    }
}
