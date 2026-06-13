//! The source-read boundary (grounded-verify capability, 008).
//!
//! The implementation owns all read hygiene — root confinement (canonicalize +
//! prefix-check, defeating `../` and symlink escape), UTF-8/text-only, and
//! line-range bounds — so the assembly stage never touches the filesystem and
//! the whole feature tests through a mock. Reads are bounded and local, so the
//! seam is synchronous.

use crate::error::AppError;

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

    /// List every regular file under the configured root, as root-relative,
    /// `/`-separated paths. Used by glob expansion (009); symlinks are not
    /// followed, so nothing outside the root is listed.
    ///
    /// # Errors
    ///
    /// [`AppError::Storage`] if the root cannot be walked.
    fn list_files(&self) -> Result<Vec<String>, AppError>;
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
    }
}
