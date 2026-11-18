//! The source-read boundary (grounded-verify capability, 008).
//!
//! The implementation owns all read hygiene — root confinement (canonicalize +
//! prefix-check, defeating `../` and symlink escape), UTF-8/text-only, and
//! line-range bounds — so the assembly stage never touches the filesystem and
//! the whole feature tests through a mock. Reads are bounded and local, so the
//! seam is synchronous.

use crate::error::AppError;
use crate::grounded::secrets::Rule;
use std::path::{Path, PathBuf};

/// A path that passed [`SourceReader::check`].
///
/// It holds the caller's spelling and the file it resolved to. Only code in
/// this crate can make one, so a read
/// through [`SourceReader::read_checked`] always follows a check (055).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    spelling: String,
    resolved: PathBuf,
}

impl Checked {
    /// A checked path. Made by a reader's `check`, and by test doubles.
    pub(crate) const fn new(spelling: String, resolved: PathBuf) -> Self {
        Self { spelling, resolved }
    }

    /// The path as the caller wrote it.
    #[must_use]
    pub fn spelling(&self) -> &str {
        &self.spelling
    }

    /// The file it resolved to when checked.
    #[must_use]
    pub fn resolved(&self) -> &Path {
        &self.resolved
    }
}

/// Why a check or a checked read failed (055). A refusal is told apart from
/// every other fault by its kind, never by an error's wording.
#[derive(Debug)]
pub enum CheckFailure {
    /// The path, as written or as resolved, is refused by this rule.
    Refused(Rule),
    /// Any other fault, carrying the same error a read gave before 055.
    Other(AppError),
}

/// One file found by [`SourceReader::list_flagged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedFile {
    /// Root-relative, `/`-separated.
    pub path: String,
    /// Whether the name rules refuse the file's whole path.
    pub refused: bool,
}

/// The verbatim content of one resolved locator, plus its byte length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceContent {
    /// The exact text read (whole file, or the requested inclusive line range).
    pub text: String,
    /// Byte length of `text` — the manifest's size figure.
    pub bytes: u64,
}

/// A source-read backend with confinement and hygiene enforced inside the
/// implementation.
#[cfg_attr(test, mockall::automock)]
pub trait SourceReader: Send + Sync {
    /// Read `path` (relative to the configured root), optionally restricted to
    /// the 1-based inclusive line range `[start_line, end_line]`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::InvalidInput`] naming the offending path for every
    /// caller-fault case — outside the root, missing, empty, non-text, or a
    /// line range whose start exceeds the file length. The error is loud and
    /// named so the assembly stage can abort the whole call (008 FR-009).
    fn read(
        &self,
        path: &str,
        start_line: Option<u32>,
        end_line: Option<u32>,
    ) -> Result<SourceContent, AppError>;

    /// List the regular files under the folder `under` (root-relative; empty
    /// for the root itself), as root-relative, `/`-separated paths. With
    /// `recurse` every file below it; without, only the files directly in it.
    /// Used by glob expansion (009), which names the folder a glob starts with
    /// so that only that folder is walked. Symlinks are not followed, so
    /// nothing outside the root is listed; a folder that does not exist lists
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`AppError::InvalidInput`] if `under` resolves outside the root;
    /// [`AppError::Storage`] if the folder cannot be walked.
    fn list_files(&self, under: &str, recurse: bool) -> Result<Vec<String>, AppError>;

    /// Decide whether `path` may be read (055): the name rules on the path as
    /// written, then confinement to the root, then the name rules on the
    /// whole resolved path. Nothing is read.
    ///
    /// # Errors
    ///
    /// [`CheckFailure::Refused`] naming the rule; [`CheckFailure::Other`]
    /// carrying the error a read gave before 055 (not found, outside the root).
    fn check(&self, path: &str) -> Result<Checked, CheckFailure>;

    /// Read a path that passed [`SourceReader::check`]. The caller's spelling
    /// is resolved and checked again first, and must still lead to the same
    /// file; then the size, text, empty and range checks a read always made.
    ///
    /// # Errors
    ///
    /// [`CheckFailure::Refused`] if it is now refused; [`CheckFailure::Other`]
    /// if it now leads elsewhere, or for any fault a read gave before 055.
    fn read_checked(
        &self,
        checked: &Checked,
        start_line: Option<u32>,
        end_line: Option<u32>,
    ) -> Result<SourceContent, CheckFailure>;

    /// As [`SourceReader::list_files`], with each file marked by whether the
    /// name rules refuse its whole path (055).
    ///
    /// # Errors
    ///
    /// As [`SourceReader::list_files`].
    fn list_flagged(&self, under: &str, recurse: bool) -> Result<Vec<ListedFile>, AppError>;

    /// The configured root as a caller should read it, for error messages.
    /// A caller working in another project cannot otherwise know where its
    /// locators are resolved.
    fn root_display(&self) -> String;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn mock_source_reader_honors_the_contract() {
        let mut mock = MockSourceReader::new();
        mock.expect_read().returning(|path, _, _| {
            Ok(SourceContent {
                text: format!("contents of {path}"),
                bytes: 10,
            })
        });

        let got = mock.read("a.rs", None, None).unwrap();
        assert!(got.text.contains("a.rs"));
        assert_eq!(got.bytes, 10);

        mock.expect_check()
            .returning(|path| Ok(Checked::new(path.to_string(), PathBuf::from(path))));
        mock.expect_read_checked().returning(|checked, _, _| {
            Ok(SourceContent {
                text: format!("contents of {}", checked.spelling()),
                bytes: 10,
            })
        });
        mock.expect_list_flagged().returning(|_, _| {
            Ok(vec![ListedFile {
                path: ".env".to_string(),
                refused: true,
            }])
        });
        let checked = mock.check("a.rs").unwrap();
        assert_eq!(checked.resolved(), Path::new("a.rs"));
        let got = mock.read_checked(&checked, None, None).unwrap();
        assert!(got.text.contains("a.rs"));
        assert!(mock.list_flagged("", false).unwrap()[0].refused);
    }
}
