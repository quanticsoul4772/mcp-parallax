//! Glob expansion (009 D2/D3).
//!
//! Translate the pattern, list the files under its leading folder via the
//! (non-symlink-following) reader seam, filter by the matcher, and sort for
//! determinism. Pure over the seam — unit-tested with a mock file list.

use crate::error::AppError;
use crate::grounded::glob::translate::translate;
use crate::traits::source::SourceReader;

/// Expand a glob to the deterministic, sorted set of matching root-relative
/// file paths.
///
/// # Errors
///
/// [`AppError::InvalidInput`] for a malformed pattern, a pattern starting with
/// `!`, one with no plain relative folder in front, or zero matches;
/// [`AppError::Storage`] if the folder cannot be walked.
pub fn expand(reader: &dyn SourceReader, pattern: &str) -> Result<Vec<String>, AppError> {
    let compiled = translate(pattern)?;
    // Whole-pattern negation selects across the whole root by definition, so
    // it cannot be confined to a folder; it is refused by name.
    if pattern.starts_with('!') {
        return Err(AppError::InvalidInput(
            "a glob starting with ! (everything except a pattern) is not accepted: it would \
             search the whole source root - name a folder and use !(...) inside it, for \
             example src/!(test)*.rs"
                .to_string(),
        ));
    }
    let mut selected: Vec<String> = Vec::new();
    match search_scope(pattern)? {
        // Only the root's own files: no descent, so nothing below is walked.
        Scope::RootFiles => selected.extend(reader.list_files("", false)?),
        Scope::Folders(folders) => {
            for folder in &folders {
                selected.extend(reader.list_files(folder, true)?);
            }
        }
    }
    selected.retain(|path| compiled.matches(path));
    selected.sort();
    selected.dedup();
    if selected.is_empty() {
        return Err(AppError::InvalidInput(format!(
            "glob matched no files under the source root {}: {pattern}",
            reader.root_display()
        )));
    }
    Ok(selected)
}

/// Where a glob is searched.
#[derive(Debug, PartialEq, Eq)]
enum Scope {
    /// A glob with no `/`: the files directly in the root, no descent.
    RootFiles,
    /// The folders the glob starts with; each is walked.
    Folders(Vec<String>),
}

/// Characters that make a segment a pattern, not a plain name.
const PATTERN_CHARS: [char; 6] = ['*', '?', '[', '{', '(', '\\'];

/// Decide, from the pattern's text alone, where it is searched.
///
/// Under a root that holds many projects a glob must not walk all of them: it
/// could be slow and could quote a same-named file from the wrong project. So
/// a glob is searched only in the folder it names — its leading plain
/// segments, or each name of a leading brace list — or, with no `/` at all,
/// among the root's own files. Anything else is refused, before any listing.
///
/// A folder part that is empty, `.`, `..`, or a drive or network path is
/// refused too: several of those resolve to the root itself, and a network
/// path would be touched before the inside-the-root check. None could match a
/// root-relative path anyway.
fn search_scope(pattern: &str) -> Result<Scope, AppError> {
    let not_relative = || {
        AppError::InvalidInput(format!(
            "a glob must be relative to the source root and start with a plain folder name \
             (no ., .. or absolute path): {pattern} - for example src/*.rs"
        ))
    };
    let no_folder = || {
        AppError::InvalidInput(format!(
            "glob needs a folder in front: {pattern} - start it with the folder to search, \
             using / between folders, for example src/*.rs"
        ))
    };
    // A drive letter or a leading backslash is a Windows path, whatever the
    // segments look like once split on `/`.
    if pattern.starts_with('\\') || pattern.as_bytes().get(1) == Some(&b':') {
        return Err(not_relative());
    }
    let segments: Vec<&str> = pattern.split('/').collect();
    let Some((_file, folders)) = segments.split_last() else {
        return Err(no_folder());
    };
    if folders.is_empty() {
        // No `/`. A backslash here is a Windows separator in disguise.
        return if pattern.contains('\\') {
            Err(no_folder())
        } else {
            Ok(Scope::RootFiles)
        };
    }
    // A leading brace list of plain names stands for several folders.
    let (heads, rest) = brace_list(folders[0])
        .map_or_else(|| (Vec::new(), folders), |names| (names, &folders[1..]));
    let plain: Vec<&str> = rest
        .iter()
        .take_while(|segment| !segment.contains(PATTERN_CHARS))
        .copied()
        .collect();
    let named = heads.iter().chain(plain.iter());
    if named
        .clone()
        .any(|name| name.is_empty() || matches!(*name, "." | "..") || name.contains(':'))
    {
        return Err(not_relative());
    }
    let tail = plain.join("/");
    let with_tail = |head: &str| {
        if tail.is_empty() {
            head.to_string()
        } else {
            format!("{head}/{tail}")
        }
    };
    if !heads.is_empty() {
        return Ok(Scope::Folders(heads.iter().map(|h| with_tail(h)).collect()));
    }
    if plain.is_empty() {
        return Err(no_folder());
    }
    Ok(Scope::Folders(vec![tail]))
}

/// The names in a segment that is exactly a brace list of two or more plain
/// names (`{src,tests}`); `None` for anything else.
fn brace_list(segment: &str) -> Option<Vec<&str>> {
    let inner = segment.strip_prefix('{')?.strip_suffix('}')?;
    let names: Vec<&str> = inner.split(',').collect();
    let plain = |name: &&str| !name.contains(PATTERN_CHARS) && !name.contains('}');
    (names.len() >= 2 && names.iter().all(plain)).then_some(names)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::source::MockSourceReader;

    fn reader_with(files: &'static [&'static str]) -> MockSourceReader {
        let mut mock = MockSourceReader::new();
        // Like the real reader: only files under the folder asked for, and
        // without descent only the files directly in it.
        mock.expect_list_files().returning(move |under, recurse| {
            let inside = if under.is_empty() {
                String::new()
            } else {
                format!("{under}/")
            };
            Ok(files
                .iter()
                .filter(|file| file.starts_with(&inside))
                .filter(|file| recurse || !file[inside.len()..].contains('/'))
                .map(|s| (*s).to_string())
                .collect())
        });
        mock
    }

    /// A glob with no slash matches only the files directly in the root: it
    /// is listed without descent, so it cannot walk the projects below.
    #[test]
    fn a_glob_with_no_slash_lists_only_the_roots_own_files() {
        let mut reader = MockSourceReader::new();
        reader
            .expect_list_files()
            .withf(|under, recurse| under.is_empty() && !*recurse)
            .returning(|_, _| {
                Ok(["README.md", "Cargo.toml", "notes.md"]
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect())
            });
        assert_eq!(expand(&reader, "*.md").unwrap(), ["README.md", "notes.md"]);
        assert_eq!(expand(&reader, "Cargo.*").unwrap(), ["Cargo.toml"]);
        assert_eq!(expand(&reader, "README.md").unwrap(), ["README.md"]);
        // Through the folder-aware stand-in: nothing below the root is seen.
        let reader = reader_with(&["a.md", "docs/b.md", "docs/deep/c.md"]);
        assert_eq!(expand(&reader, "*.md").unwrap(), ["a.md"]);
        assert_eq!(expand(&reader, "**.md").unwrap(), ["a.md"]);
    }

    /// A first segment that is a brace list of plain folder names searches
    /// each named folder, and only those.
    #[test]
    fn a_leading_brace_list_searches_each_named_folder() {
        let listed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = listed.clone();
        let mut reader = MockSourceReader::new();
        reader.expect_list_files().returning(move |under, recurse| {
            assert!(recurse);
            seen.lock().unwrap().push(under.to_string());
            Ok(match under {
                "src" => vec!["src/a.rs".to_string(), "src/x/b.rs".to_string()],
                "tests" => vec!["tests/t.rs".to_string(), "tests/t.txt".to_string()],
                _ => Vec::new(),
            })
        });
        assert_eq!(
            expand(&reader, "{src,tests}/**/*.rs").unwrap(),
            ["src/a.rs", "src/x/b.rs", "tests/t.rs"]
        );
        assert_eq!(*listed.lock().unwrap(), ["src", "tests"]);

        // Plain folder names after the list narrow each folder further.
        listed.lock().unwrap().clear();
        reader.expect_root_display().return_const("R".to_string());
        let err = expand(&reader, "{src,tests}/unit/*.rs").unwrap_err();
        assert!(err.to_string().contains("matched no files"), "{err}");
        assert_eq!(*listed.lock().unwrap(), ["src/unit", "tests/unit"]);
    }

    /// A glob with no folder in front is refused before anything is listed.
    /// Under a root that holds many projects it would search all of them and
    /// could quote a same-named file from the wrong one.
    #[test]
    fn a_glob_with_no_folder_in_front_is_refused_without_listing() {
        // No expectations: listing anything would panic the mock.
        let reader = MockSourceReader::new();
        for pattern in [
            "**/agent.py",
            r"src\*.rs",
            "?x/y.rs",
            "{a,*}/x.rs",
            "x*/{a,b}/y.rs",
        ] {
            let err = expand(&reader, pattern).unwrap_err();
            assert!(
                err.to_string().contains(&format!(
                    "glob needs a folder in front: {pattern} - start it with the folder to \
                     search, using / between folders, for example src/*.rs"
                )),
                "{pattern}: {err}"
            );
        }
    }

    /// Whole-pattern negation ("every file except ...") selects across the
    /// whole root by definition, so it is refused by name, without listing.
    /// Negation inside a segment is a different thing and still works.
    #[test]
    fn whole_pattern_negation_is_refused_by_name() {
        let reader = MockSourceReader::new();
        for pattern in ["!src/*.rs", "!*.rs", "!(a)/x.rs"] {
            let err = expand(&reader, pattern).unwrap_err();
            assert!(
                err.to_string().contains(
                    "a glob starting with ! (everything except a pattern) is not accepted: \
                     it would search the whole source root - name a folder and use !(...) \
                     inside it, for example src/!(test)*.rs"
                ),
                "{pattern}: {err}"
            );
        }
        // Inside a segment, after a folder: accepted, and only `src` is listed.
        let mut reader = MockSourceReader::new();
        reader
            .expect_list_files()
            .withf(|under, _| under == "src")
            .returning(|_, _| Ok(vec!["src/a.rs".to_string(), "src/test_a.rs".to_string()]));
        assert_eq!(expand(&reader, "src/!(test)*.rs").unwrap(), ["src/a.rs"]);
    }

    /// A folder part that is empty, absolute, a drive or network path, or has
    /// a `.` or `..` segment is refused before anything is listed or resolved:
    /// several of these resolve to the root itself (a whole-root walk), and a
    /// network path would be contacted before the inside-the-root check. None
    /// could match anyway — patterns are matched against root-relative paths.
    #[test]
    fn a_glob_folder_that_is_not_a_plain_relative_folder_is_refused_without_listing() {
        // No expectations: listing or resolving anything would panic the mock.
        let reader = MockSourceReader::new();
        for pattern in [
            "./**/x.py",
            "/*.rs",
            "src/../**/x.py",
            "../other/*.rs",
            "src/./a/*.rs",
            "//server/share/dir/*.rs",
            "C:/root/src/*.rs",
            // Written the Windows way: the trouble is the backslashes, not a
            // missing folder.
            r"C:\proj\src\*.rs",
            r"\src\*.rs",
            r"\\server\share\x/*.rs",
            // A brace list is a list of plain folder names, held to the same
            // rule.
            "{a,..}/x.rs",
            "{src,C:}/x.rs",
        ] {
            let err = expand(&reader, pattern).unwrap_err();
            assert!(
                err.to_string().contains(&format!(
                    "a glob must be relative to the source root and start with a plain \
                     folder name (no ., .. or absolute path): {pattern} - for example \
                     src/*.rs"
                )),
                "{pattern}: {err}"
            );
        }
        // A folder whose NAME starts with a dot is an ordinary folder.
        let mut reader = MockSourceReader::new();
        reader
            .expect_list_files()
            .withf(|under, _| under == ".github/workflows")
            .returning(|_, _| Ok(vec![".github/workflows/ci.yml".to_string()]));
        assert_eq!(
            expand(&reader, ".github/workflows/*.yml").unwrap(),
            [".github/workflows/ci.yml"]
        );
    }

    /// Only the glob's leading folder is listed: the folder names before the
    /// first one that carries a pattern character.
    #[test]
    fn a_glob_is_listed_only_inside_its_leading_folder() {
        for (pattern, folder) in [
            ("src/*.rs", "src"),
            ("src/app/**/*.rs", "src/app"),
            ("a/b/c*/d.rs", "a/b"),
            ("docs/guide/intro.md", "docs/guide"),
        ] {
            let mut reader = MockSourceReader::new();
            reader
                .expect_list_files()
                .withf(move |under, recurse| under == folder && *recurse)
                .times(1)
                .returning(|_, _| Ok(Vec::new()));
            reader.expect_root_display().return_const("R".to_string());
            // Nothing is listed back, so the outcome is the no-match error;
            // what is pinned here is the folder that was asked for.
            let err = expand(&reader, pattern).unwrap_err();
            assert!(err.to_string().contains("matched no files"), "{pattern}");
        }
    }

    /// A glob that matches nothing names the root it was matched under: the
    /// caller is usually globbing for a file that lives in another project.
    #[test]
    fn a_glob_matching_nothing_names_the_root() {
        let mut reader = reader_with(&["src/a.rs"]);
        reader
            .expect_root_display()
            .return_const("D:\\Projects\\one".to_string());
        let err = expand(&reader, "src/**/missing.py").unwrap_err();
        assert!(
            err.to_string().contains(
                "glob matched no files under the source root D:\\Projects\\one: \
                 src/**/missing.py"
            ),
            "{err}"
        );
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
        let mut reader = reader_with(&["a.rs"]);
        reader.expect_root_display().return_const("R".to_string());
        let err = expand(&reader, "nope/*.rs").unwrap_err();
        assert!(err
            .to_string()
            .contains("matched no files under the source root R: nope/*.rs"));
    }

    #[test]
    fn malformed_pattern_propagates_named() {
        let reader = reader_with(&["a.rs"]);
        let err = expand(&reader, "@(a").unwrap_err();
        assert!(err.to_string().contains("malformed glob pattern"));
    }
}
