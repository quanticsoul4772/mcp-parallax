//! Glob expansion (009 D2/D3).
//!
//! Translate the pattern, list the root's files via the (non-symlink-following)
//! reader seam, filter by the matcher, and sort for determinism. Pure over the
//! seam — unit-tested with a mock file list.

use crate::error::AppError;
use crate::grounded::glob::translate::translate;
use crate::traits::source::SourceReader;

/// Expand a glob to the deterministic, sorted set of matching root-relative
/// file paths.
///
/// # Errors
///
/// [`AppError::InvalidInput`] for a malformed pattern or zero matches;
/// [`AppError::Storage`] if the root cannot be walked.
pub fn expand(reader: &dyn SourceReader, pattern: &str) -> Result<Vec<String>, AppError> {
    let compiled = translate(pattern)?;
    let mut selected: Vec<String> = reader
        .list_files()?
        .into_iter()
        .filter(|path| compiled.matches(path))
        .collect();
    selected.sort();
    selected.dedup();
    if selected.is_empty() {
        return Err(AppError::InvalidInput(format!(
            "glob matched no files: {pattern}"
        )));
    }
    Ok(selected)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::source::MockSourceReader;

    fn reader_with(files: &'static [&'static str]) -> MockSourceReader {
        let mut mock = MockSourceReader::new();
        mock.expect_list_files()
            .returning(move || Ok(files.iter().map(|s| (*s).to_string()).collect()));
        mock
    }

    #[test]
    fn expands_sorted_and_filtered_to_the_match_set() {
        let reader = reader_with(&["src/b.rs", "src/a.rs", "src/x.txt", "tests/t.rs"]);
        let got = expand(&reader, "src/*.rs").unwrap();
        assert_eq!(got, vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
    }

    #[test]
    fn doublestar_is_recursive() {
        let reader = reader_with(&["src/a.rs", "src/x/b.rs", "src/x/y/c.rs", "top.rs"]);
        let got = expand(&reader, "src/**/*.rs").unwrap();
        assert_eq!(
            got,
            vec![
                "src/a.rs".to_string(),
                "src/x/b.rs".to_string(),
                "src/x/y/c.rs".to_string()
            ]
        );
    }

    #[test]
    fn zero_match_is_a_named_error() {
        let reader = reader_with(&["a.rs"]);
        let err = expand(&reader, "nope/*.rs").unwrap_err();
        assert!(err.to_string().contains("matched no files: nope/*.rs"));
    }

    #[test]
    fn malformed_pattern_propagates_named() {
        let reader = reader_with(&["a.rs"]);
        let err = expand(&reader, "@(a").unwrap_err();
        assert!(err.to_string().contains("malformed glob pattern"));
    }
}
