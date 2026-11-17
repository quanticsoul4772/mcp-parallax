//! `SystemSourceReader`: the real, root-confined source reader (008 D2/D3).
//!
//! Confinement is canonicalize-then-prefix-check: the configured root and every
//! resolved path are canonicalized (which follows symlinks), then the resolved
//! path must be prefixed by the root. This rejects `../` traversal and symlink
//! escape before any byte is read. Reads are text-only (valid UTF-8) and support
//! whole-file or a 1-based inclusive line range.

use crate::error::{AppError, ConfigError};
use crate::traits::source::{SourceContent, SourceReader};
use std::path::{Path, PathBuf};

/// A source reader confined to one canonicalized root directory.
#[derive(Debug, Clone)]
pub struct SystemSourceReader {
    root: PathBuf,
    /// Per-file ceiling: a file larger than this is rejected by metadata
    /// *before* it is read into memory, so a single oversized locator cannot
    /// spike memory ahead of the assembly stage's total-bytes check. Set to the
    /// per-call evidence ceiling (`GROUNDED_VERIFY_MAX_BYTES`).
    max_file_bytes: u64,
}

impl SystemSourceReader {
    /// Canonicalize `root` once and confirm it is a directory. `max_file_bytes`
    /// caps any single read (the per-call evidence ceiling).
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Config`] (`GROUNDED_VERIFY_ROOT`) when the root does
    /// not exist or is not a directory — a startup misconfiguration, loud and
    /// named.
    pub fn new(root: &str, max_file_bytes: usize) -> Result<Self, AppError> {
        let canonical = std::fs::canonicalize(root)
            .map_err(|_| AppError::Config(ConfigError::Invalid("GROUNDED_VERIFY_ROOT")))?;
        if !canonical.is_dir() {
            return Err(AppError::Config(ConfigError::Invalid(
                "GROUNDED_VERIFY_ROOT",
            )));
        }
        Ok(Self {
            root: canonical,
            max_file_bytes: max_file_bytes as u64,
        })
    }

    /// Resolve a relative path within the root, rejecting any escape.
    fn resolve(&self, path: &str) -> Result<PathBuf, AppError> {
        // First screen, on the text alone: a network path that does not begin
        // with the root's own path is refused before the host is contacted.
        // Everything else goes through canonicalize-then-prefix below, which
        // is what defeats `..` and symlink escapes, and what accepts an
        // inside path however it is spelled.
        if self.is_network_path_outside_root(path) {
            return Err(self.escapes(path));
        }
        let canonical =
            std::fs::canonicalize(self.root.join(path)).map_err(|_| self.not_found(path))?;
        if !canonical.starts_with(&self.root) {
            return Err(self.escapes(path));
        }
        Ok(canonical)
    }

    /// Whether `path` is a Windows network path (`\\server\share\...`, in
    /// either slash direction, or its verbatim `\\?\UNC\` form) whose text
    /// does not begin with the root's. Compared without regard to letter
    /// case or slash direction.
    ///
    /// Only network paths are screened: resolving one contacts its host.
    /// Local paths are left to the disk check, because a text comparison
    /// would wrongly refuse an inside path spelled through a link, a mapped
    /// drive, a short name or the verbatim `\\?\C:\` form. A root that is
    /// itself on a share must be addressed by the spelling the errors print.
    fn is_network_path_outside_root(&self, path: &str) -> bool {
        if !cfg!(windows) {
            return false;
        }
        let normalize = |text: &str| {
            let slashed = text.replace('\\', "/").to_lowercase();
            slashed
                .strip_prefix("//?/unc/")
                .map_or_else(|| slashed.clone(), |share| format!("//{share}"))
        };
        let given = normalize(path);
        // `//?/c:/...` and `//./...` are local device forms, not hosts.
        if !given.starts_with("//") || given.starts_with("//?/") || given.starts_with("//./") {
            return false;
        }
        let root = normalize(&self.root_display());
        let root = root.trim_end_matches('/');
        !(given == root || given.starts_with(&format!("{root}/")))
    }

    /// The outside-the-root error, naming the root.
    fn escapes(&self, path: &str) -> AppError {
        AppError::InvalidInput(format!(
            "locator escapes the source root {}: {path} - only files inside that root can \
             be read",
            self.root_display()
        ))
    }

    /// The not-found error: it names the root and the locator forms that
    /// work, because the usual cause is a path relative to another project.
    fn not_found(&self, path: &str) -> AppError {
        AppError::InvalidInput(format!(
            "source not found under the source root {}: {path} - give a path relative to \
             that root, or an absolute path inside it",
            self.root_display()
        ))
    }
}

impl SourceReader for SystemSourceReader {
    fn read(
        &self,
        path: &str,
        start_line: Option<u32>,
        end_line: Option<u32>,
    ) -> Result<SourceContent, AppError> {
        let range = match (start_line, end_line) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => {
                return Err(AppError::InvalidInput(format!(
                    "locator '{path}' must give both start_line and end_line, or neither"
                )))
            }
        };

        let resolved = self.resolve(path)?;
        // Reject an oversized file by metadata before reading it into memory.
        let on_disk = std::fs::metadata(&resolved)
            .map_err(|_| self.not_found(path))?
            .len();
        if on_disk > self.max_file_bytes {
            return Err(AppError::InvalidInput(format!(
                "source '{path}' is {on_disk} bytes, over the per-file limit of {} \
                 (GROUNDED_VERIFY_MAX_BYTES); raise the ceiling to read it",
                self.max_file_bytes
            )));
        }
        let raw = std::fs::read(&resolved).map_err(|_| self.not_found(path))?;
        let text = String::from_utf8(raw)
            .map_err(|_| AppError::InvalidInput(format!("source is not text: {path}")))?;
        if text.is_empty() {
            return Err(AppError::InvalidInput(format!("source is empty: {path}")));
        }

        let selected = match range {
            None => text,
            Some((start, end)) => slice_lines(&text, start, end, path)?,
        };
        if selected.is_empty() {
            return Err(AppError::InvalidInput(format!(
                "source range is empty: {path}"
            )));
        }
        let bytes = selected.len() as u64;
        Ok(SourceContent {
            text: selected,
            bytes,
        })
    }

    fn list_files(&self, under: &str, recurse: bool) -> Result<Vec<String>, AppError> {
        // Walk only the named folder. One that does not exist (or is a file)
        // lists nothing; one that resolves outside the root is refused by the
        // same canonicalize-then-prefix check as a read.
        let Ok(start) = std::fs::canonicalize(self.root.join(under)) else {
            return Ok(Vec::new());
        };
        if !start.starts_with(&self.root) {
            return Err(self.escapes(under));
        }
        // A named folder that is really the root (a link back to it, or on
        // Windows a name of dots and spaces) would walk everything. Only the
        // empty name means the root, and that is listed without descent.
        if !under.is_empty() && start == self.root {
            return Err(AppError::InvalidInput(format!(
                "the glob's folder resolves to the source root itself: {under} - name a \
                 folder inside the root"
            )));
        }
        if !start.is_dir() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        // Depth 1 is the folder's own entries: no descent.
        let depth = if recurse { usize::MAX } else { 1 };
        for entry in walkdir::WalkDir::new(&start)
            .follow_links(false)
            .max_depth(depth)
        {
            let entry =
                entry.map_err(|e| AppError::Storage(format!("walking source root: {e}")))?;
            if !entry.file_type().is_file() {
                continue;
            }
            if let Ok(rel) = entry.path().strip_prefix(&self.root) {
                files.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(files)
    }

    fn root_display(&self) -> String {
        display_path(&self.root)
    }
}

/// A canonical path as a caller would type it. Windows canonical paths carry
/// a verbatim prefix: `\\?\C:\dir` is shown as `C:\dir`, and a network share
/// `\\?\UNC\server\share\dir` as `\\server\share\dir`. Anything else is shown
/// unchanged.
fn display_path(path: &Path) -> String {
    let shown = path.display().to_string();
    if let Some(share) = shown.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{share}");
    }
    shown.strip_prefix(r"\\?\").unwrap_or(&shown).to_string()
}

/// Take the 1-based inclusive line range `[start, end]` from `text`. The start
/// past end-of-file is an error (008 edge case); the end is clamped to the file
/// length.
fn slice_lines(text: &str, start: u32, end: u32, path: &str) -> Result<String, AppError> {
    if start < 1 || start > end {
        return Err(AppError::InvalidInput(format!(
            "locator '{path}' has an invalid line range {start}..={end}"
        )));
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let line_count = lines.len();
    let start_idx = start as usize;
    if start_idx > line_count {
        return Err(AppError::InvalidInput(format!(
            "line range start {start} is past the end of '{path}' ({line_count} lines)"
        )));
    }
    let end_idx = (end as usize).min(line_count);
    Ok(lines[start_idx - 1..end_idx].join("\n"))
}

/// Expose the canonical root for diagnostics/tests.
impl AsRef<Path> for SystemSourceReader {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;

    fn root_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in files {
            let p = dir.path().join(name);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(p, body).unwrap();
        }
        dir
    }

    fn reader(dir: &tempfile::TempDir) -> SystemSourceReader {
        SystemSourceReader::new(dir.path().to_str().unwrap(), 262_144).unwrap()
    }

    /// The root a caller should be shown for `dir`: its real path without the
    /// Windows verbatim prefix. This repeats the code's own prefix rule, so it
    /// is not an independent check of that rule; the independent checks are
    /// in `the_displayed_root_is_the_configured_directory`.
    fn expected_root(dir: &tempfile::TempDir) -> String {
        let real = fs::canonicalize(dir.path()).unwrap();
        let shown = real.to_str().unwrap();
        let shown = shown.strip_prefix(r"\\?\").unwrap_or(shown).to_string();
        assert!(Path::new(&shown).is_absolute() && Path::new(&shown).is_dir());
        shown
    }

    /// The displayed root is the configured directory, not merely something
    /// the message and the test agree on.
    #[test]
    fn the_displayed_root_is_the_configured_directory() {
        let dir = root_with(&[("a.rs", "inside")]);
        let reader = reader(&dir);
        let shown = reader.root_display();
        assert_eq!(shown, expected_root(&dir));
        // Independent of any prefix handling: the path shown names the very
        // directory that was configured.
        assert_eq!(
            fs::canonicalize(&shown).unwrap(),
            fs::canonicalize(dir.path()).unwrap()
        );
        // Usable as given: an absolute locator built on it reads the file.
        let built = Path::new(&shown).join("a.rs");
        assert_eq!(
            reader
                .read(built.to_str().unwrap(), None, None)
                .unwrap()
                .text,
            "inside"
        );
    }

    /// A network path that does not begin with the root's own path is refused
    /// from its text, so the host is never contacted. Every other path is
    /// resolved on disk and then checked, as it always was: that is what
    /// accepts an inside path however it is spelled (a link, a mapped drive,
    /// a short name, the verbatim form).
    #[test]
    fn a_network_path_outside_the_root_is_refused_from_its_text() {
        let dir = root_with(&[("a.rs", "inside")]);
        let reader = reader(&dir);
        for path in [
            "//no-such-host.invalid/share/x.txt",
            r"\\no-such-host.invalid\share\x.txt",
            r"\\?\UNC\no-such-host.invalid\share\x.txt",
        ] {
            assert!(reader.is_network_path_outside_root(path), "{path}");
            let err = reader.read(path, None, None).unwrap_err();
            assert!(
                err.to_string().contains("locator escapes the source root"),
                "{path}: {err}"
            );
        }
        // Not network paths: these go to the disk check, unscreened.
        let inside = dir.path().join("a.rs");
        let verbatim = fs::canonicalize(&inside).unwrap();
        for path in [
            "gone.rs",
            "../elsewhere/x.txt",
            inside.to_str().unwrap(),
            verbatim.to_str().unwrap(),
            "C:/somewhere/else.txt",
            "/somewhere/else.txt",
        ] {
            assert!(!reader.is_network_path_outside_root(path), "{path}");
        }
        // An inside path in its canonical (on Windows, verbatim) spelling is
        // read, not refused.
        assert_eq!(
            reader
                .read(verbatim.to_str().unwrap(), None, None)
                .unwrap()
                .text,
            "inside"
        );
        // An outside local path is refused by the disk check, with no read.
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("there.txt"), "top secret").unwrap();
        let there = outside.path().join("there.txt");
        let err = reader
            .read(there.to_str().unwrap(), None, None)
            .unwrap_err();
        assert!(
            err.to_string().contains("locator escapes the source root"),
            "{err}"
        );
    }

    /// The text screen compares as Windows does: letter case and slash
    /// direction do not matter, so an inside path typed either way is read.
    #[test]
    #[cfg(windows)]
    fn an_inside_absolute_path_is_accepted_in_any_case_and_slash_direction() {
        let dir = root_with(&[("a.rs", "inside")]);
        let reader = reader(&dir);
        let typed = format!("{}/a.rs", reader.root_display());
        for path in [
            typed.replace('\\', "/"),
            typed.replace('/', "\\"),
            typed.to_uppercase(),
            typed.to_lowercase(),
        ] {
            assert_eq!(
                reader.read(&path, None, None).unwrap().text,
                "inside",
                "{path}"
            );
        }
    }

    /// Windows canonical paths carry a prefix no caller types. A drive path
    /// loses it; a network share becomes `\\server\share\...`, not
    /// `UNC\server\share\...`. Any other path is shown unchanged.
    #[test]
    fn canonical_windows_prefixes_are_shown_as_a_caller_would_type_them() {
        for (canonical, shown) in [
            (r"\\?\C:\Projects\one", r"C:\Projects\one"),
            (r"\\?\UNC\server\share\dir", r"\\server\share\dir"),
            (r"C:\Projects\one", r"C:\Projects\one"),
            ("/home/user/projects", "/home/user/projects"),
        ] {
            assert_eq!(display_path(Path::new(canonical)), shown, "{canonical}");
        }
    }

    #[test]
    fn whole_file_read_returns_verbatim_with_byte_len() {
        let dir = root_with(&[("a.rs", "fn main() {}\n")]);
        let got = reader(&dir).read("a.rs", None, None).unwrap();
        assert_eq!(got.text, "fn main() {}\n");
        assert_eq!(got.bytes, 13);
    }

    #[test]
    fn line_range_is_one_based_inclusive() {
        let dir = root_with(&[("a.rs", "one\ntwo\nthree\nfour\n")]);
        let got = reader(&dir).read("a.rs", Some(2), Some(3)).unwrap();
        assert_eq!(got.text, "two\nthree");
    }

    #[test]
    fn range_end_past_eof_clamps_but_start_past_eof_errors() {
        let dir = root_with(&[("a.rs", "one\ntwo\n")]);
        // end past EOF clamps to what exists.
        let clamped = reader(&dir).read("a.rs", Some(1), Some(99)).unwrap();
        assert!(clamped.text.starts_with("one\ntwo"));
        // start past EOF is a named error.
        let err = reader(&dir).read("a.rs", Some(50), Some(60)).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
        assert!(err.to_string().contains("past the end"));
    }

    #[test]
    fn missing_file_is_a_named_not_found_error() {
        let dir = root_with(&[("a.rs", "x")]);
        let reader = reader(&dir);
        let err = reader.read("gone.rs", None, None).unwrap_err();
        // Mined calls 2026-10/11: 19 of 53 failed "source not found" because
        // the caller, working in another project, could not know the root.
        // The error names it and says what form of locator works.
        let root = expected_root(&dir);
        assert!(
            err.to_string().contains(&format!(
                "source not found under the source root {root}: gone.rs - give a path \
                 relative to that root, or an absolute path inside it"
            )),
            "{err}"
        );
    }

    /// An absolute path is accepted when it lies inside the root, and rejected
    /// with the root named when it does not (21 of 53 mined calls).
    #[test]
    fn an_absolute_path_works_inside_the_root_and_is_named_outside_it() {
        let dir = root_with(&[("a.rs", "inside")]);
        let reader = reader(&dir);
        let inside = dir.path().join("a.rs");
        assert_eq!(
            reader
                .read(inside.to_str().unwrap(), None, None)
                .unwrap()
                .text,
            "inside"
        );

        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
        let elsewhere = outside.path().join("secret.txt");
        let err = reader
            .read(elsewhere.to_str().unwrap(), None, None)
            .unwrap_err();
        let root = expected_root(&dir);
        assert!(
            err.to_string().contains(&format!(
                "locator escapes the source root {root}: {} - only files inside that root \
                 can be read",
                elsewhere.to_str().unwrap()
            )),
            "{err}"
        );
    }

    #[test]
    fn empty_file_is_a_named_error() {
        let dir = root_with(&[("empty.rs", "")]);
        let err = reader(&dir).read("empty.rs", None, None).unwrap_err();
        assert!(err.to_string().contains("source is empty"));
    }

    #[test]
    fn non_text_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("b.bin"), [0xff, 0xfe, 0x00, 0x9c]).unwrap();
        let err = reader(&dir).read("b.bin", None, None).unwrap_err();
        assert!(err.to_string().contains("not text"));
    }

    #[test]
    fn traversal_outside_the_root_is_rejected() {
        // A file that exists outside the root.
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
        let dir = root_with(&[("a.rs", "x")]);
        // Reach for it with `..` segments.
        let escape = format!(
            "../{}/secret.txt",
            outside.path().file_name().unwrap().to_str().unwrap()
        );
        let err = reader(&dir).read(&escape, None, None).unwrap_err();
        // Either "not found" (canonicalization landed elsewhere) or an explicit
        // escape — in both cases no content is returned.
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn mismatched_single_bound_is_rejected() {
        let dir = root_with(&[("a.rs", "one\ntwo\n")]);
        let err = reader(&dir).read("a.rs", Some(1), None).unwrap_err();
        assert!(err.to_string().contains("both start_line and end_line"));
    }

    #[test]
    #[cfg(unix)]
    fn symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
        let dir = root_with(&[("a.rs", "x")]);
        symlink(
            outside.path().join("secret.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();
        let err = reader(&dir).read("link.txt", None, None).unwrap_err();
        assert!(err.to_string().contains("escapes the source root"));
    }

    #[test]
    fn nonexistent_root_is_a_config_error() {
        let err = SystemSourceReader::new("/definitely/not/a/real/dir/xyzzy", 262_144).unwrap_err();
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    #[cfg(unix)]
    fn list_files_does_not_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.rs"), "x").unwrap();
        let dir = root_with(&[("top/a.rs", "x"), ("top/sub/b.rs", "y")]);
        symlink(outside.path(), dir.path().join("top").join("link")).unwrap();
        let files = reader(&dir).list_files("top", true).unwrap();
        assert!(files.iter().any(|f| f == "top/a.rs"));
        assert!(files.iter().any(|f| f == "top/sub/b.rs"));
        assert!(!files.iter().any(|f| f.contains("secret")));
    }

    /// Only the named folder is walked, paths come back root-relative, a
    /// missing folder lists nothing, and a folder outside the root is refused.
    #[test]
    fn list_files_walks_only_the_named_folder() {
        let dir = root_with(&[
            ("a.rs", "x"),
            ("src/b.rs", "x"),
            ("src/x/c.rs", "x"),
            ("other/d.rs", "x"),
        ]);
        let reader = reader(&dir);
        let mut files = reader.list_files("src", true).unwrap();
        files.sort();
        assert_eq!(files, ["src/b.rs", "src/x/c.rs"]);
        assert_eq!(reader.list_files("src/x", true).unwrap(), ["src/x/c.rs"]);
        assert!(reader.list_files("nope", true).unwrap().is_empty());
        // A file is not a folder to walk.
        assert!(reader.list_files("a.rs", true).unwrap().is_empty());
        let err = reader.list_files("..", true).unwrap_err();
        assert!(err.to_string().contains("escapes the source root"), "{err}");
        // A named folder that turns out to be the root itself is refused: the
        // whole root must never be walked for a glob. `.` stands in here for
        // every spelling that resolves to the root, such as a link pointing
        // back at it.
        let err = reader.list_files(".", true).unwrap_err();
        assert!(
            err.to_string()
                .contains("resolves to the source root itself"),
            "{err}"
        );
        // A review reasoned that on Windows a name of dots would be stripped
        // and resolve to the root. Run, it does not: the root is held in its
        // verbatim form, under which such a name is taken literally. It
        // names nothing, so nothing is listed and nothing is walked.
        assert!(reader.list_files("...", true).unwrap().is_empty());
        assert!(reader.list_files("src. ", true).unwrap().is_empty());
        // Without descent: only the files directly in the folder. For the
        // root itself that is its own files and nothing from the folders
        // below it.
        assert_eq!(reader.list_files("", false).unwrap(), ["a.rs"]);
        assert_eq!(reader.list_files("src", false).unwrap(), ["src/b.rs"]);
    }

    #[test]
    fn a_file_over_the_per_file_limit_is_rejected_before_read() {
        let dir = root_with(&[("big.rs", "0123456789")]); // 10 bytes
        let reader = SystemSourceReader::new(dir.path().to_str().unwrap(), 5).unwrap();
        let err = reader.read("big.rs", None, None).unwrap_err();
        assert!(err.to_string().contains("per-file limit"), "{err}");
    }
}
