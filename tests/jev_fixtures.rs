//! 064 T031: the committed Jev fixtures are synthetic and secret-free.
//!
//! Every non-empty line of every fixture carries the provenance marker, and
//! the redactor's own patterns find nothing to remove in any of them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::jev::redact::Redactor;
use std::path::Path;

const MARKER: &str = "\"provenance\": \"synthetic-064\"";

#[test]
fn every_jev_fixture_is_marked_synthetic_and_has_nothing_to_redact() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jev");
    let mut files = 0;
    for entry in std::fs::read_dir(&folder).expect("fixture folder") {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        files += 1;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            assert!(
                line.contains(MARKER),
                "{}: unmarked line {line}",
                path.display()
            );
        }
        let redacted = Redactor::new(vec![]).redact(&text).unwrap();
        assert_eq!(
            redacted.redactions(),
            0,
            "{}: secret-shaped text: {}",
            path.display(),
            redacted.as_str()
        );
    }
    assert!(files > 0, "no fixtures found");
}
