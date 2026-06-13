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
}

impl SystemSourceReader {
    /// Canonicalize `root` once and confirm it is a directory.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Config`] (`GROUNDED_VERIFY_ROOT`) when the root does
    /// not exist or is not a directory — a startup misconfiguration, loud and
    /// named.
    pub fn new(root: &str) -> Result<Self, AppError> {
        let canonical = std::fs::canonicalize(root)
            .map_err(|_| AppError::Config(ConfigError::Invalid("GROUNDED_VERIFY_ROOT")))?;
        if !canonical.is_dir() {
            return Err(AppError::Config(ConfigError::Invalid(
                "GROUNDED_VERIFY_ROOT",
            )));
        }
        Ok(Self { root: canonical })
    }

    /// Resolve a relative path within the root, rejecting any escape.
    fn resolve(&self, path: &str) -> Result<PathBuf, AppError> {
        let canonical = std::fs::canonicalize(self.root.join(path))
            .map_err(|_| AppError::InvalidInput(format!("source not found: {path}")))?;
        if !canonical.starts_with(&self.root) {
            return Err(AppError::InvalidInput(format!(
                "locator escapes the source root: {path}"
            )));
        }
        Ok(canonical)
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
        let raw = std::fs::read(&resolved)
            .map_err(|_| AppError::InvalidInput(format!("source not found: {path}")))?;
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
        SystemSourceReader::new(dir.path().to_str().unwrap()).unwrap()
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
        let err = reader(&dir).read("gone.rs", None, None).unwrap_err();
        assert!(err.to_string().contains("source not found: gone.rs"));
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
        let err = SystemSourceReader::new("/definitely/not/a/real/dir/xyzzy").unwrap_err();
        assert!(matches!(err, AppError::Config(_)));
    }
}
