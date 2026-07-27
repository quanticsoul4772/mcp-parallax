//! Reading the two operator-facing documents: what default each *states*.
//!
//! Only ever a structured marker — the README's Default column, `--help`'s
//! `(default: X)` — and never the surrounding prose (FR-010). Prose carries
//! ranges, ceilings and version strings that are not defaults, and a check that
//! cries wolf gets silenced rather than corrected, which is this module's own
//! failure mode aimed the other way.

use super::Fact;

/// A default stated in a document, read from its structured marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatedDefault {
    /// The variable the document claims this default belongs to.
    pub name: String,
    /// The value the document states.
    pub value: String,
}

/// Defaults a README-style table states, read from the **Default column only**.
///
/// Never the Purpose column (FR-010). Prose carries numbers that are not
/// defaults — ranges like `1..=20`, ceilings quoted inside explanations,
/// version strings — and a reverse scan over it would produce false positives.
/// A check that cries wolf gets silenced rather than corrected, which is this
/// module's own failure mode aimed the other way.
#[must_use]
pub fn stated_in_table(table: &str) -> Vec<StatedDefault> {
    let mut out = Vec::new();
    for line in table.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // | name | required | default | purpose |  ->  ["", name, req, def, purpose, ""]
        if cells.len() < 5 {
            continue;
        }
        let name = cells[1].trim_matches('`');
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            continue;
        }
        let value = cells[3].trim_matches('`');
        if is_absence_sentinel(value) {
            continue;
        }
        out.push(StatedDefault {
            name: name.to_string(),
            value: value.to_string(),
        });
    }
    out
}

/// Whether a stated default is really a statement that there is none.
///
/// Shared by both documents: one wrote `—`, the other `empty`, and a filter
/// living in only one of them is how the two drift.
pub(super) fn is_absence_sentinel(value: &str) -> bool {
    matches!(value.trim(), "" | "—" | "-" | "unset" | "empty" | "none")
}

/// Indentation at which `--help` starts a variable's entry. Continuation lines
/// are indented past it.
pub(super) const ENTRY_COLUMN: usize = 4;

/// Defaults `--help` states, read from its `(default: X)` marker only.
///
/// The same discipline as [`stated_in_table`] applied to the other document:
/// one structured marker, never the surrounding description.
///
/// An entry's description wraps, and the marker may land on a continuation
/// line — `RESEARCH_CONCURRENCY` states its default that way. So the marker is
/// attached to the entry it belongs to rather than required to share a line
/// with the name. A blank line or a `SECTION:` header ends the entry, so a
/// marker can never attach across the gap to an unrelated variable.
#[must_use]
pub fn stated_in_help(help: &str) -> Vec<StatedDefault> {
    let mut out: Vec<StatedDefault> = Vec::new();
    let mut current: Option<String> = None;
    for line in help.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            current = None;
            continue;
        }
        let ident: String = trimmed
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        let after = &trimmed[ident.len()..];
        if !ident.is_empty() && after.starts_with(':') {
            // A section header, not a variable.
            current = None;
            continue;
        }
        // A name must start at the entry column. Any uppercase word on a
        // continuation line otherwise becomes an entry: a description reading
        // "API waits this long before failing (default: 120000)" captured the
        // marker under the name `API`, leaving the real variable reported as
        // undocumented in a document that documents it correctly. The live
        // text is already one word from this — `VERIFY_MAX_CLAIM_CHARS is
        // honoured as a` sits on a continuation line today and survives only
        // because it carries no marker.
        let indent = line.len() - trimmed.len();
        if ident.len() >= 3 && after.starts_with(' ') && indent == ENTRY_COLUMN {
            current = Some(ident);
        }
        let Some(name) = current.clone() else {
            continue;
        };
        // A second marker for the same name is recorded, never skipped, so
        // `assert_no_contradictory_defaults` can report a document that
        // contradicts itself instead of the first value silently winning.
        let Some(at) = line.find("(default: ") else {
            continue;
        };
        let rest = &line[at + "(default: ".len()..];
        let Some(close) = rest.find(')') else {
            continue;
        };
        let value = rest[..close].trim();
        // The same sentinel filter the table uses. `--help` writes
        // `(default: empty)` for a variable whose absence gates a capability;
        // that is a statement about having no default, not a default.
        if is_absence_sentinel(value) {
            continue;
        }
        out.push(StatedDefault {
            name,
            value: value.to_string(),
        });
    }
    out
}

/// Fail if one document states two different defaults for the same variable.
///
/// A document contradicting itself was invisible while the first value silently
/// won.
///
/// # Panics
///
/// Panics naming the variable and both values.
pub fn assert_no_contradictory_defaults(doc: &str, stated: &[StatedDefault]) {
    for (i, a) in stated.iter().enumerate() {
        for b in &stated[i + 1..] {
            assert!(
                !(a.name == b.name && a.value != b.value),
                "DEFAULT_CONTRADICTORY: {doc} states both `{}` and `{}` as the default for \
                 `{}`. One document, two answers — whichever is read first would silently win.",
                a.value,
                b.value,
                a.name
            );
        }
    }
}

/// Fail if a document states a default for a variable that carries none
/// (FR-009).
///
/// The forward direction iterates variables the source still contains, so a row
/// left behind after a variable was removed is invisible to it.
///
/// # Panics
///
/// Panics naming the document, the variable, and the value it stated.
pub fn assert_no_phantom_defaults(facts: &[Fact], doc: &str, stated: &[StatedDefault]) {
    for s in stated {
        assert!(
            facts.iter().any(|f| f.name == s.name),
            "DEFAULT_PHANTOM: {doc} states a default of `{}` for `{}`, which the configuration \
             does not read with a default. Either the variable was removed and the row outlived \
             it, or the row was written for a variable that never had one.",
            s.value,
            s.name
        );
    }
}
