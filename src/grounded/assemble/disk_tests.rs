//! 055 SC-002: the refusal on a real disk, through assembly with the real
//! reader, so the text rules and the resolved-path rules are both in play.

use super::assemble;
use crate::grounded::reader::SystemSourceReader;
use crate::grounded::{AssemblyLimits, SourceLocator, HOST_PLATFORM};
use std::fs;
use std::path::Path;

fn limits() -> AssemblyLimits {
    AssemblyLimits {
        max_bytes: 262_144,
        max_locators: 64,
        platform: HOST_PLATFORM,
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

fn reader(root: &Path) -> SystemSourceReader {
    SystemSourceReader::new(root.to_str().unwrap(), 262_144).unwrap()
}

/// The root as a caller would type it: its real path, without `\\?\`.
#[cfg(windows)]
fn shown(root: &Path) -> String {
    let real = fs::canonicalize(root).unwrap();
    let real = real.to_str().unwrap();
    real.strip_prefix(r"\\?\").unwrap_or(real).to_string()
}

fn refused_text(path: &str) -> String {
    format!(
        "invalid input: locator refused: {path} - it is, or resolves to, a file whose name \
         marks it as a likely secret"
    )
}

#[cfg(windows)]
fn colon_text(path: &str) -> String {
    format!(
        "invalid input: locator refused: {path} - a colon is not accepted in a path; give line \
         numbers as start_line and end_line"
    )
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// A folder link: a junction on Windows, a symbolic link elsewhere. Fails the
/// test with a message if it cannot be made.
fn dir_link(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .expect("could not run mklink");
        assert!(
            out.status.success(),
            "could not make a junction {} -> {}: {}",
            link.display(),
            target.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    #[cfg(unix)]
    {
        let made = std::os::unix::fs::symlink(target, link);
        assert!(
            made.is_ok(),
            "could not make a symbolic link {} -> {}: {made:?}",
            link.display(),
            target.display()
        );
    }
}

#[test]
fn entering_and_leaving_a_secret_folder_reads_the_ordinary_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".ssh/known_hosts", "host key");
    write(dir.path(), "README.md", "hi");
    let out = assemble(
        &reader(dir.path()),
        &[path_loc(".ssh/../README.md")],
        limits(),
    )
    .unwrap();
    assert_eq!(out.units[0].text, "hi");
}

#[test]
fn folder_rules_that_span_the_root_refuse_the_file() {
    let base = tempfile::tempdir().unwrap();
    let config = base.path().join(".config");
    write(&config, "gh/x", "token");
    let err = assemble(&reader(&config), &[path_loc("gh/x")], limits()).unwrap_err();
    assert_eq!(err.to_string(), refused_text("gh/x"));

    let repo = base.path().join("x.git");
    write(&repo, "config", "[remote]");
    let err = assemble(&reader(&repo), &[path_loc("config")], limits()).unwrap_err();
    assert_eq!(err.to_string(), refused_text("config"));
}

#[test]
#[cfg(windows)]
fn a_verbatim_path_to_an_ordinary_file_is_read_with_either_slash() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", "inside");
    let root = shown(dir.path());
    let back = format!(r"\\?\{root}\a.rs");
    let forward = format!("//?/{}/a.rs", root.replace('\\', "/"));
    for path in [back, forward] {
        let out = assemble(&reader(dir.path()), &[path_loc(&path)], limits());
        assert!(out.is_ok(), "{path}: {:?}", out.err());
        assert_eq!(out.unwrap().units[0].text, "inside", "{path}");
    }
}

#[test]
#[cfg(windows)]
fn a_trailing_dot_or_space_does_not_hide_a_secret_name() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "id_rsa", "private key");
    let root = shown(dir.path());
    for path in [format!(r"{root}\id_rsa."), format!(r"{root}\id_rsa ")] {
        let err = assemble(&reader(dir.path()), &[path_loc(&path)], limits()).unwrap_err();
        assert_eq!(err.to_string(), refused_text(&path), "{path:?}");
    }
}

/// Before 055 the reader resolved `notes.txt:hidden` to a path inside the
/// root and returned the stream's contents (recorded 2026-11-18 on this
/// machine: canonicalize kept the `:hidden` suffix and the read returned
/// "stream contents"). Rule 8 now refuses it from its text.
#[test]
#[cfg(windows)]
fn a_hidden_data_stream_is_refused_with_the_colon_message() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notes.txt", "base file");
    write(dir.path(), ".env", "KEY=1");
    for (stream, body) in [("notes.txt:hidden", "stream contents"), (".env:s", "more")] {
        fs::write(dir.path().join(stream), body).unwrap();
        let back = fs::read_to_string(dir.path().join(stream));
        assert_eq!(
            back.as_deref().ok(),
            Some(body),
            "no stream was made: {stream}"
        );
        let err = assemble(&reader(dir.path()), &[path_loc(stream)], limits()).unwrap_err();
        assert_eq!(err.to_string(), colon_text(stream));
    }
}

#[test]
fn a_link_into_a_secret_folder_is_refused_without_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".ssh/known_hosts", "host key");
    dir_link(&dir.path().join("docs"), &dir.path().join(".ssh"));
    let err = assemble(
        &reader(dir.path()),
        &[path_loc("docs/known_hosts")],
        limits(),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), refused_text("docs/known_hosts"));
    assert!(!err.to_string().contains(".ssh"), "{err}");
}

#[test]
fn a_glob_does_not_walk_through_a_folder_link() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "a");
    write(dir.path(), "elsewhere/b.rs", "b");
    write(dir.path(), ".ssh/c.rs", "c");
    dir_link(
        &dir.path().join("src").join("linked"),
        &dir.path().join("elsewhere"),
    );
    dir_link(
        &dir.path().join("src").join("keys"),
        &dir.path().join(".ssh"),
    );
    let glob = SourceLocator {
        path: None,
        glob: Some("src/**/*.rs".to_string()),
        start_line: None,
        end_line: None,
    };
    let out = assemble(&reader(dir.path()), &[glob], limits()).unwrap();
    let paths: Vec<&str> = out.manifest.iter().map(|m| m.path.as_str()).collect();
    assert_eq!(paths, ["src/a.rs"]);
    assert!(out.refused.is_empty());
}

#[test]
#[cfg(unix)]
fn a_link_to_a_secret_file_is_refused_and_a_colon_is_ordinary() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".env", "KEY=1");
    write(dir.path(), "a:b.txt", "colon");
    std::os::unix::fs::symlink(dir.path().join(".env"), dir.path().join("notes.txt")).unwrap();
    let err = assemble(&reader(dir.path()), &[path_loc("notes.txt")], limits()).unwrap_err();
    assert_eq!(err.to_string(), refused_text("notes.txt"));
    assert!(!err.to_string().contains(".env"), "{err}");
    let out = assemble(&reader(dir.path()), &[path_loc("a:b.txt")], limits()).unwrap();
    assert_eq!(out.units[0].text, "colon");
}
