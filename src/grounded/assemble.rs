//! The assembly stage (008 D4, extended by 009): resolve every locator
//! all-or-nothing into the verbatim evidence the ensemble judges, plus the
//! audit manifest.
//!
//! Pure over a [`SourceReader`] — no filesystem here, so it tests through a
//! mock. A single failing locator aborts the whole call (008 FR-009); no model
//! pass ever runs on a partially-resolved evidence set. A glob locator (009) is
//! expanded into its concrete files before the read loop, subject to the same
//! locator-count and byte ceilings. Since 055 a file whose name marks it as a
//! likely secret is refused, and a glob's refused matches are the one exception
//! to all-or-nothing: they are left out and listed in `refused`.

use crate::error::AppError;
use crate::grounded::glob::expand::{expand, glob_text_check};
use crate::grounded::secrets::{refusal, simplify, Rule};
use crate::grounded::{AssemblyLimits, LocatorKind, ManifestEntry, SourceLocator};
use crate::traits::source::{CheckFailure, Checked, SourceReader};
use std::collections::BTreeSet;

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
    /// Files a glob matched that the name rules refuse (055): sorted, no
    /// duplicates, never read.
    pub refused: Vec<String>,
}

/// The verbatim content of one resolved read unit (011) — what a count runs over.
#[derive(Debug, Clone)]
pub struct RawUnit {
    /// The raw source text as read (no provenance header).
    pub text: String,
    /// The reader's byte length for this unit (mirrors the manifest entry).
    pub bytes: u64,
}

/// One concrete file to read, in locator order, before any exact path is
/// checked on disk.
enum Unit {
    /// An exact path, with its optional line range.
    Exact {
        path: String,
        start: Option<u32>,
        end: Option<u32>,
    },
    /// A file a glob matched; `glob` is the locator's index.
    Match { path: String, glob: usize },
}

/// A unit ready to read: an exact path has passed its check.
enum Ready {
    Exact(Checked, Option<u32>, Option<u32>),
    Match(String, usize),
}

/// What one glob locator has left to read, and how many of its matches were
/// refused, so the call can fail if every match turns out refused.
struct GlobTally<'a> {
    pattern: &'a str,
    left: usize,
    refused: usize,
}

/// The units of a call after phase two, with the refused files so far and,
/// per locator, its glob tally (`None` for an exact path).
struct Listed<'a> {
    units: Vec<Unit>,
    refused: BTreeSet<String>,
    tallies: Vec<Option<GlobTally<'a>>>,
}

/// Resolve all locators into the verbatim evidence, in 055 FR-002's order.
///
/// The order is: the locators' text first, then glob listings under the
/// running locator count, then every exact path checked, then the reads.
///
/// All-or-nothing, with one exception: a glob's matches that the name rules
/// refuse are left out and listed in `refused` (055 FR-006), never read.
///
/// # Errors
///
/// [`AppError::InvalidInput`], naming the offending locator, for: an empty
/// locator set, a malformed locator or line range, a refused exact path, a
/// glob that is refused, matches nothing or matches only refused files, more
/// than `max_locators` after expansion, any locator the reader rejects, or
/// assembled bytes over `max_bytes`. On any error nothing is returned.
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
    let kinds = check_text(locators, limits)?;
    let Listed {
        units,
        refused,
        tallies,
    } = list_globs(reader, &kinds, limits)?;
    // Phase three: every exact path is checked before any file is read.
    let ready: Vec<Ready> = units
        .into_iter()
        .map(|unit| match unit {
            Unit::Exact { path, start, end } => reader
                .check(&path)
                .map(|checked| Ready::Exact(checked, start, end))
                .map_err(|failure| from_failure(&path, failure)),
            Unit::Match { path, glob } => Ok(Ready::Match(path, glob)),
        })
        .collect::<Result<_, _>>()?;
    read_all(reader, ready, refused, tallies, limits)
}

/// Phase one: everything decidable from the locators' text, nothing touched.
fn check_text(
    locators: &[SourceLocator],
    limits: AssemblyLimits,
) -> Result<Vec<LocatorKind<'_>>, AppError> {
    let mut kinds = Vec::with_capacity(locators.len());
    for loc in locators {
        let kind = loc.classify()?;
        match kind {
            LocatorKind::Path {
                path,
                start_line,
                end_line,
            } => {
                check_range(path, start_line, end_line)?;
                if let Some(rule) = refusal(&simplify(path, limits.platform), limits.platform) {
                    return Err(refused(path, rule));
                }
            }
            LocatorKind::Glob { pattern } => glob_text_check(pattern)?,
        }
        kinds.push(kind);
    }
    Ok(kinds)
}

/// Phase two: glob listings, in locator order, with the locator-count
/// ceiling applied to the running total exactly as before (009 FR-006). No
/// exact path is resolved here. Refused matches do not count.
fn list_globs<'a>(
    reader: &dyn SourceReader,
    kinds: &[LocatorKind<'a>],
    limits: AssemblyLimits,
) -> Result<Listed<'a>, AppError> {
    let mut listed = Listed {
        units: Vec::new(),
        refused: BTreeSet::new(),
        tallies: Vec::with_capacity(kinds.len()),
    };
    for (index, kind) in kinds.iter().enumerate() {
        match kind {
            LocatorKind::Path {
                path,
                start_line,
                end_line,
            } => {
                listed.units.push(Unit::Exact {
                    path: (*path).to_string(),
                    start: *start_line,
                    end: *end_line,
                });
                listed.tallies.push(None);
            }
            LocatorKind::Glob { pattern } => {
                let found = expand(reader, pattern)?;
                if found.files.is_empty() {
                    return Err(all_refused(pattern, found.refused.len()));
                }
                listed.tallies.push(Some(GlobTally {
                    pattern,
                    left: found.files.len(),
                    refused: found.refused.len(),
                }));
                listed.refused.extend(found.refused);
                listed.units.extend(
                    found
                        .files
                        .into_iter()
                        .map(|path| Unit::Match { path, glob: index }),
                );
            }
        }
        if listed.units.len() > limits.max_locators {
            return Err(AppError::InvalidInput(format!(
                "too many locators ({}, max {}) after glob expansion",
                listed.units.len(),
                limits.max_locators
            )));
        }
    }
    Ok(listed)
}

/// The reads, in locator order (008's read/manifest/byte loop). Each file is
/// checked again just before it is read; a glob match refused then is left
/// out, and its glob fails the call if nothing of it remains.
fn read_all(
    reader: &dyn SourceReader,
    ready: Vec<Ready>,
    mut refused_files: BTreeSet<String>,
    mut tallies: Vec<Option<GlobTally<'_>>>,
    limits: AssemblyLimits,
) -> Result<AssembledEvidence, AppError> {
    let mut manifest = Vec::with_capacity(ready.len());
    let mut sections = Vec::with_capacity(ready.len());
    let mut raw_units = Vec::with_capacity(ready.len());
    let mut total: usize = 0;
    for unit in ready {
        let (path, start, end, content) = match unit {
            Ready::Exact(checked, start, end) => {
                let content = reader
                    .read_checked(&checked, start, end)
                    .map_err(|failure| from_failure(checked.spelling(), failure))?;
                (checked.spelling().to_string(), start, end, content)
            }
            Ready::Match(path, glob) => {
                match reader
                    .check(&path)
                    .and_then(|checked| reader.read_checked(&checked, None, None))
                {
                    Ok(content) => (path, None, None, content),
                    Err(CheckFailure::Refused(_)) => {
                        if let Some(tally) = tallies.get_mut(glob).and_then(Option::as_mut) {
                            tally.left = tally.left.saturating_sub(1);
                            tally.refused += 1;
                            if tally.left == 0 {
                                return Err(all_refused(tally.pattern, tally.refused));
                            }
                        }
                        refused_files.insert(path);
                        continue;
                    }
                    Err(CheckFailure::Other(err)) => return Err(err),
                }
            }
        };
        total = total.saturating_add(usize::try_from(content.bytes).unwrap_or(usize::MAX));
        if total > limits.max_bytes {
            return Err(AppError::InvalidInput(format!(
                "assembled evidence exceeds {} bytes (at locator '{}')",
                limits.max_bytes, path
            )));
        }
        sections.push(format!(
            "===== {} =====\n{}",
            header(&path, start, end),
            content.text
        ));
        manifest.push(ManifestEntry {
            path,
            start_line: start,
            end_line: end,
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
        refused: refused_files.into_iter().collect(),
    })
}

/// A line range's shape, from the locator alone: both bounds or neither, and
/// a start of at least 1 that does not pass the end. The same texts the
/// reader gives.
fn check_range(path: &str, start: Option<u32>, end: Option<u32>) -> Result<(), AppError> {
    match (start, end) {
        (None, None) => Ok(()),
        (Some(start), Some(end)) if start >= 1 && start <= end => Ok(()),
        (Some(start), Some(end)) => Err(AppError::InvalidInput(format!(
            "locator '{path}' has an invalid line range {start}..={end}"
        ))),
        _ => Err(AppError::InvalidInput(format!(
            "locator '{path}' must give both start_line and end_line, or neither"
        ))),
    }
}

/// The error for a refused exact path. A colon on Windows (rule 8) is told
/// apart: its likely cause is a line number pasted after a file name.
fn refused(path: &str, rule: Rule) -> AppError {
    AppError::InvalidInput(if rule == Rule::Colon {
        format!(
            "locator refused: {path} - a colon is not accepted in a path; give line numbers \
             as start_line and end_line"
        )
    } else {
        format!(
            "locator refused: {path} - it is, or resolves to, a file whose name marks it as a \
             likely secret"
        )
    })
}

/// The error for a failed check or checked read of an exact path.
fn from_failure(path: &str, failure: CheckFailure) -> AppError {
    match failure {
        CheckFailure::Refused(rule) => refused(path, rule),
        CheckFailure::Other(err) => err,
    }
}

/// The error for a glob whose every match is refused.
fn all_refused(pattern: &str, count: usize) -> AppError {
    AppError::InvalidInput(format!(
        "glob matched only files refused as likely secrets ({count}): {pattern}"
    ))
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
mod disk_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::grounded::secrets::{Platform, Rule};
    use crate::traits::source::{
        CheckFailure, Checked, ListedFile, MockSourceReader, SourceContent,
    };

    fn limits() -> AssemblyLimits {
        AssemblyLimits {
            max_bytes: 262_144,
            max_locators: 64,
            platform: Platform::Other,
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

    /// A recording reader that serves `body:<path>` for every file, fails
    /// the check of any in `fails`, and (for glob tests) lists `list`.
    fn reader_failing(fails: &'static [&'static str], list: &'static [&'static str]) -> Recorder {
        Recorder {
            fail_on_check: fails.to_vec(),
            listing: list.iter().map(|p| ((*p).to_string(), false)).collect(),
            ..Recorder::default()
        }
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
        let reader = reader_failing(&[], &["src/a.rs", "src/b.rs", "src/c.rs"]);
        let tight = AssemblyLimits {
            max_bytes: 262_144,
            max_locators: 2,
            platform: Platform::Other,
        };
        let err = assemble(&reader, &[glob_loc("src/*.rs")], tight).unwrap_err();
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
        mock.expect_check().returning(|path| {
            Ok(Checked::new(
                path.to_string(),
                std::path::PathBuf::from(path),
            ))
        });
        mock.expect_read_checked().returning(|_, _, _| {
            Ok(SourceContent {
                text: "x".repeat(100),
                bytes: 100,
            })
        });
        let tight = AssemblyLimits {
            max_bytes: 150,
            max_locators: 64,
            platform: Platform::Other,
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

    // ---- 055: the refusal, through a reader that records every call ----

    /// A stand-in reader that logs every call.
    #[derive(Default)]
    struct Recorder {
        /// What a flagged listing returns: path and refused mark.
        listing: Vec<(String, bool)>,
        refuse_on_check: Vec<&'static str>,
        fail_on_check: Vec<&'static str>,
        refuse_on_read: Vec<&'static str>,
        fail_on_read: Vec<&'static str>,
        log: std::sync::Mutex<Vec<String>>,
    }

    impl Recorder {
        fn listing(files: &[(&str, bool)]) -> Self {
            Self {
                listing: files.iter().map(|(p, r)| ((*p).to_string(), *r)).collect(),
                ..Self::default()
            }
        }

        fn note(&self, what: String) {
            self.log.lock().unwrap().push(what);
        }

        fn calls(&self, kind: &str) -> Vec<String> {
            let prefix = format!("{kind}:");
            self.log
                .lock()
                .unwrap()
                .iter()
                .filter_map(|line| line.strip_prefix(&prefix).map(str::to_string))
                .collect()
        }

        fn reads(&self) -> usize {
            self.calls("read_checked").len()
        }

        fn body(path: &str) -> SourceContent {
            let text = format!("body:{path}");
            let bytes = text.len() as u64;
            SourceContent { text, bytes }
        }

        fn listed<'a>(&'a self, under: &'a str) -> impl Iterator<Item = &'a (String, bool)> {
            let inside = format!("{under}/");
            self.listing
                .iter()
                .filter(move |(path, _)| under.is_empty() || path.starts_with(&inside))
        }
    }

    impl SourceReader for Recorder {
        fn root_display(&self) -> String {
            "R".to_string()
        }

        fn check(&self, path: &str) -> Result<Checked, CheckFailure> {
            self.note(format!("check:{path}"));
            if self.refuse_on_check.contains(&path) {
                return Err(CheckFailure::Refused(Rule::PartName));
            }
            if self.fail_on_check.contains(&path) {
                return Err(CheckFailure::Other(AppError::InvalidInput(format!(
                    "source not found: {path}"
                ))));
            }
            Ok(Checked::new(
                path.to_string(),
                std::path::PathBuf::from(path),
            ))
        }

        fn read_checked(
            &self,
            checked: &Checked,
            _: Option<u32>,
            _: Option<u32>,
        ) -> Result<SourceContent, CheckFailure> {
            let path = checked.spelling();
            self.note(format!("read_checked:{path}"));
            if self.refuse_on_read.contains(&path) {
                return Err(CheckFailure::Refused(Rule::PartName));
            }
            if self.fail_on_read.contains(&path) {
                return Err(CheckFailure::Other(AppError::InvalidInput(format!(
                    "source not found: {path}"
                ))));
            }
            Ok(Self::body(path))
        }

        fn list_files(&self, under: &str, _: bool) -> Result<Vec<ListedFile>, AppError> {
            self.note(format!("list_files:{under}"));
            Ok(self
                .listed(under)
                .map(|(path, refused)| ListedFile {
                    path: path.clone(),
                    refused: *refused,
                })
                .collect())
        }
    }

    fn limits_with(max_locators: usize, platform: Platform) -> AssemblyLimits {
        AssemblyLimits {
            max_bytes: 262_144,
            max_locators,
            platform,
        }
    }

    fn refused_text(path: &str) -> String {
        format!(
            "invalid input: locator refused: {path} - it is, or resolves to, a file whose name \
             marks it as a likely secret"
        )
    }

    fn bounds(path: &str, start: Option<u32>, end: Option<u32>) -> SourceLocator {
        SourceLocator {
            path: Some(path.to_string()),
            glob: None,
            start_line: start,
            end_line: end,
        }
    }

    #[test]
    fn a_secret_name_written_in_the_call_is_refused_before_anything_is_checked_or_read() {
        for secret in [
            ".env",
            ".env.local",
            "x.pem",
            "a.tfstate.b",
            "id_rsa.bak",
            ".ssh/k",
            ".git/config",
            ".config/gh/x",
        ] {
            let reader = Recorder::default();
            let err = assemble(
                &reader,
                &[path_loc("a.rs"), path_loc(secret)],
                limits_with(64, Platform::Other),
            )
            .unwrap_err();
            assert_eq!(err.to_string(), refused_text(secret), "{secret}");
            assert!(reader.calls("check").is_empty(), "{secret}");
            assert_eq!(reader.reads(), 0, "{secret}");
        }
    }

    /// SC-005: refused only by what it resolves to, so this passes only if
    /// every exact path is checked before any file is read.
    #[test]
    fn every_exact_path_is_checked_before_any_file_is_read() {
        let reader = Recorder {
            refuse_on_check: vec!["docs/known_hosts"],
            ..Recorder::default()
        };
        let err = assemble(
            &reader,
            &[path_loc("a.rs"), path_loc("docs/known_hosts")],
            limits_with(64, Platform::Other),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), refused_text("docs/known_hosts"));
        assert_eq!(reader.calls("check"), ["a.rs", "docs/known_hosts"]);
        assert_eq!(reader.reads(), 0);
    }

    #[test]
    fn an_exact_path_refused_or_failing_at_read_time_fails_the_call() {
        let reader = Recorder {
            refuse_on_read: vec!["b.rs"],
            ..Recorder::default()
        };
        let err = assemble(
            &reader,
            &[path_loc("a.rs"), path_loc("b.rs")],
            limits_with(64, Platform::Other),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), refused_text("b.rs"));
        let reader = Recorder {
            fail_on_read: vec!["b.rs"],
            ..Recorder::default()
        };
        let err = assemble(
            &reader,
            &[path_loc("a.rs"), path_loc("b.rs")],
            limits_with(64, Platform::Other),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "invalid input: source not found: b.rs");
    }

    #[test]
    fn a_bad_line_range_is_refused_before_anything_is_checked() {
        for (loc, text) in [
            (
                bounds("b.rs", Some(3), None),
                "locator 'b.rs' must give both start_line and end_line, or neither",
            ),
            (
                bounds("b.rs", Some(0), Some(2)),
                "locator 'b.rs' has an invalid line range 0..=2",
            ),
            (
                bounds("b.rs", Some(5), Some(2)),
                "locator 'b.rs' has an invalid line range 5..=2",
            ),
        ] {
            let reader = Recorder::default();
            let err = assemble(
                &reader,
                &[path_loc("a.rs"), loc],
                limits_with(64, Platform::Other),
            )
            .unwrap_err();
            assert_eq!(err.to_string(), format!("invalid input: {text}"));
            assert!(reader.calls("check").is_empty());
            assert_eq!(reader.reads(), 0);
        }
    }

    #[test]
    fn on_windows_a_colon_gets_its_own_message() {
        let reader = Recorder::default();
        let err = assemble(
            &reader,
            &[path_loc("src/a.rs:12")],
            limits_with(64, Platform::Windows),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid input: locator refused: src/a.rs:12 - a colon is not accepted in a path; \
             give line numbers as start_line and end_line"
        );
        assert!(reader.calls("check").is_empty());
    }

    #[test]
    fn a_glob_leaves_refused_files_out_and_lists_them() {
        let reader =
            Recorder::listing(&[("src/a.rs", false), ("src/b.rs", false), ("src/k.rs", true)]);
        let out = assemble(
            &reader,
            &[glob_loc("src/*.rs")],
            limits_with(64, Platform::Other),
        )
        .unwrap();
        let paths: Vec<&str> = out.manifest.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs", "src/b.rs"]);
        assert_eq!(out.refused, ["src/k.rs"]);
        assert!(!reader.calls("check").contains(&"src/k.rs".to_string()));
        assert!(!reader
            .calls("read_checked")
            .contains(&"src/k.rs".to_string()));
    }

    #[test]
    fn a_glob_whose_every_match_is_refused_fails_the_call() {
        let reader = Recorder::listing(&[("src/j.rs", true), ("src/k.rs", true)]);
        let err = assemble(
            &reader,
            &[glob_loc("src/*.rs")],
            limits_with(64, Platform::Other),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid input: glob matched only files refused as likely secrets (2): src/*.rs"
        );
        assert_eq!(reader.reads(), 0);
    }

    #[test]
    fn a_glob_match_refused_at_read_time_is_left_out_and_listed() {
        let reader = Recorder {
            refuse_on_check: vec!["src/b.rs"],
            refuse_on_read: vec!["src/c.rs"],
            ..Recorder::listing(&[
                ("src/a.rs", false),
                ("src/b.rs", false),
                ("src/c.rs", false),
            ])
        };
        let out = assemble(
            &reader,
            &[glob_loc("src/*.rs")],
            limits_with(64, Platform::Other),
        )
        .unwrap();
        let paths: Vec<&str> = out.manifest.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs"]);
        assert_eq!(out.refused, ["src/b.rs", "src/c.rs"]);

        let reader = Recorder {
            refuse_on_check: vec!["lone/a.rs"],
            ..Recorder::listing(&[("lone/a.rs", false)])
        };
        let err = assemble(
            &reader,
            &[glob_loc("lone/*.rs")],
            limits_with(64, Platform::Other),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid input: glob matched only files refused as likely secrets (1): lone/*.rs"
        );
    }

    #[test]
    fn a_glob_match_failing_at_read_time_for_another_reason_fails_the_call() {
        let reader = Recorder {
            fail_on_read: vec!["src/b.rs"],
            ..Recorder::listing(&[("src/a.rs", false), ("src/b.rs", false)])
        };
        let err = assemble(
            &reader,
            &[glob_loc("src/*.rs")],
            limits_with(64, Platform::Other),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "invalid input: source not found: src/b.rs");
    }

    #[test]
    fn refused_files_do_not_count_toward_the_ceiling() {
        let reader = Recorder::listing(&[
            ("src/a.rs", false),
            ("src/b.rs", false),
            ("src/k1.rs", true),
            ("src/k2.rs", true),
            ("src/k3.rs", true),
        ]);
        let out = assemble(
            &reader,
            &[glob_loc("src/*.rs")],
            limits_with(2, Platform::Other),
        )
        .unwrap();
        assert_eq!(out.manifest.len(), 2);
        assert_eq!(out.refused.len(), 3);
    }

    #[test]
    fn a_file_refused_by_two_globs_is_listed_once() {
        let reader = Recorder::listing(&[("src/a.rs", false), ("src/k.rs", true)]);
        let out = assemble(
            &reader,
            &[glob_loc("src/*.rs"), glob_loc("src/{a,k}.rs")],
            limits_with(64, Platform::Other),
        )
        .unwrap();
        assert_eq!(out.refused, ["src/k.rs"]);
    }

    /// Pins: today's running count already gives these, with no check.
    #[test]
    fn the_locator_ceiling_is_a_running_count_applied_where_it_is_today() {
        let many: Vec<SourceLocator> = (0..66).map(|i| path_loc(&format!("f{i}.rs"))).collect();
        let reader = Recorder::default();
        let err = assemble(&reader, &many, limits_with(64, Platform::Other)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid input: too many locators (65, max 64) after glob expansion"
        );
        assert!(reader.calls("check").is_empty());

        let ten: Vec<(String, bool)> = (0..10).map(|i| (format!("g/{i}.rs"), false)).collect();
        let reader = Recorder {
            listing: ten,
            ..Recorder::default()
        };
        let mut mixed = vec![glob_loc("g/*.rs")];
        mixed.extend((0..60).map(|i| path_loc(&format!("f{i}.rs"))));
        let err = assemble(&reader, &mixed, limits_with(64, Platform::Other)).unwrap_err();
        assert!(
            err.to_string().contains("too many locators (65, max 64)"),
            "{err}"
        );
        assert!(reader.calls("check").is_empty());

        let reader = Recorder::listing(&[("g/a.rs", false)]);
        let mut late_glob: Vec<SourceLocator> =
            (0..65).map(|i| path_loc(&format!("f{i}.rs"))).collect();
        late_glob.push(glob_loc("g/*.rs"));
        let err = assemble(&reader, &late_glob, limits_with(64, Platform::Other)).unwrap_err();
        assert!(
            err.to_string().contains("too many locators (65, max 64)"),
            "{err}"
        );
        assert!(reader.calls("list_files").is_empty());
    }
}
