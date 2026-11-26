//! Entities for the 066 `unverified_fact` detector (FR-006, research D6).
//!
//! Extracts file paths, version strings, code-shaped identifiers and hashes,
//! and numbers from a final message, and looks each up verbatim (after the
//! same normalization) in the turn's own evidence. No names, no dates. Nothing
//! here is sent anywhere: the detector runs in code on the machine.

use crate::checkpoint::fnv1a64;

/// What kind of entity was found; the names are the record schema's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    /// A token containing a slash once backslashes are normalized.
    Path,
    /// Dotted digits such as `1.14.0` (a leading `v` dropped).
    Version,
    /// A code-shaped token of 7+ characters: letters, digits, `_` and `-`,
    /// with a digit, an underscore, or a capital after the first character.
    Identifier,
    /// A run of 3+ digits on word boundaries.
    Number,
}

impl EntityKind {
    /// The kind's name in `turn_signal_records.unmatched_entities`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Version => "version",
            Self::Identifier => "identifier",
            Self::Number => "number",
        }
    }
}

/// One entity, its text normalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    /// Its kind.
    pub kind: EntityKind,
    /// Its normalized text.
    pub text: String,
}

/// An entity not found in the evidence: its kind and a hash of its text,
/// never the text itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmatched {
    /// Its kind.
    pub kind: EntityKind,
    /// `fnv1a64` of the normalized text, as 16 hex characters.
    pub hash: String,
}

/// Backslashes and JSON escapes become forward slashes, so a path written
/// either way, or escaped inside a tool input, compares equal.
fn normalize(text: &str) -> String {
    text.replace("\\\\", "/")
        .replace("\\/", "/")
        .replace('\\', "/")
}

const EDGE_PUNCTUATION: &[char] = &[
    '"', '\'', '`', '(', ')', '[', ']', '{', '}', '<', '>', ',', ';', ':', '!', '?', '.',
];

/// The entities in `text`, in order (FR-006).
#[must_use]
pub fn extract(text: &str) -> Vec<Entity> {
    let normalized = normalize(text);
    let mut found = Vec::new();
    for raw in normalized.split_whitespace() {
        let token = raw.trim_matches(EDGE_PUNCTUATION);
        if token.is_empty() || is_iso_date(token) {
            continue;
        }
        if token.contains('/') && token.chars().any(char::is_alphanumeric) {
            found.push(entity(EntityKind::Path, token));
        } else if let Some(version) = version_of(token) {
            found.push(entity(EntityKind::Version, version));
        } else if is_identifier(token) {
            found.push(entity(EntityKind::Identifier, token));
        } else {
            found.extend(
                digit_runs(token)
                    .into_iter()
                    .map(|run| entity(EntityKind::Number, run)),
            );
        }
    }
    found
}

/// Each entity of `final_message` found in none of `evidence` (FR-006):
/// numbers on word boundaries, the other kinds as substrings, both sides
/// normalized the same way.
#[must_use]
pub fn unmatched(final_message: &str, evidence: &[&str]) -> Vec<Unmatched> {
    let evidence: Vec<String> = evidence.iter().map(|e| normalize(e)).collect();
    extract(final_message)
        .into_iter()
        .filter(|e| {
            !evidence.iter().any(|text| match e.kind {
                EntityKind::Number => contains_on_word_boundary(text, &e.text),
                _ => text.contains(&e.text),
            })
        })
        .map(|e| Unmatched {
            kind: e.kind,
            hash: format!("{:016x}", fnv1a64(e.text.as_bytes())),
        })
        .collect()
}

fn entity(kind: EntityKind, text: &str) -> Entity {
    Entity {
        kind,
        text: text.to_string(),
    }
}

fn is_iso_date(token: &str) -> bool {
    let b = token.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn version_of(token: &str) -> Option<&str> {
    let bare = token
        .strip_prefix('v')
        .or_else(|| token.strip_prefix('V'))
        .unwrap_or(token);
    let parts: Vec<&str> = bare.split('.').collect();
    (parts.len() >= 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit())))
    .then_some(bare)
}

fn is_identifier(token: &str) -> bool {
    token.chars().count() >= 7
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !token.chars().all(|c| c.is_ascii_digit())
        && (token.chars().any(|c| c.is_ascii_digit() || c == '_')
            || token.chars().skip(1).any(|c| c.is_ascii_uppercase()))
}

/// Runs of 3+ ASCII digits in `token` not touching a letter or digit.
fn digit_runs(token: &str) -> Vec<&str> {
    let bytes = token.as_bytes();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let before = start.checked_sub(1).map(|j| bytes[j]);
        let after = bytes.get(i).copied();
        let bounded = !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric());
        if i - start >= 3 && bounded {
            runs.push(&token[start..i]);
        }
    }
    runs
}

fn contains_on_word_boundary(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    haystack.match_indices(needle).any(|(at, _)| {
        let before = at.checked_sub(1).map(|j| bytes[j]);
        let after = bytes.get(at + needle.len()).copied();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn kinds_and_texts(text: &str) -> Vec<(EntityKind, String)> {
        extract(text)
            .into_iter()
            .map(|e| (e.kind, e.text))
            .collect()
    }

    #[test]
    fn a_path_with_backslashes_matches_its_json_escaped_form_in_the_evidence() {
        let message = r"I edited C:\src\lib.rs as asked.";
        let evidence = r#"{"file_path":"C:\\src\\lib.rs"}"#;
        assert_eq!(
            kinds_and_texts(message),
            vec![(EntityKind::Path, "C:/src/lib.rs".to_string())]
        );
        assert!(unmatched(message, &[evidence]).is_empty());
    }

    #[test]
    fn an_escaped_forward_slash_in_the_evidence_matches_a_plain_path() {
        let evidence = r#"{"file_path":"src\/checkpoint\/run.rs"}"#;
        assert!(unmatched("see src/checkpoint/run.rs.", &[evidence]).is_empty());
    }

    #[test]
    fn version_strings_are_extracted_without_a_leading_v() {
        assert_eq!(
            kinds_and_texts("pinned to 1.14.0, not v2.3"),
            vec![
                (EntityKind::Version, "1.14.0".to_string()),
                (EntityKind::Version, "2.3".to_string()),
            ]
        );
    }

    #[test]
    fn code_shaped_tokens_of_seven_or_more_characters_are_identifiers() {
        for token in ["a60f12d", "toolu_f01", "parseTurn", "066-jev-turn-end"] {
            assert_eq!(
                kinds_and_texts(&format!("the {token} one")),
                vec![(EntityKind::Identifier, token.to_string())],
                "{token}"
            );
        }
    }

    #[test]
    fn plain_words_short_tokens_and_capitalized_words_are_not_identifiers() {
        for text in ["because", "tokenize", "Because", "abc12", "Parallax"] {
            assert!(extract(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn numbers_of_three_or_more_digits_match_only_on_word_boundaries() {
        assert_eq!(
            kinds_and_texts("failed at line 142 of 42"),
            vec![(EntityKind::Number, "142".to_string())]
        );
        assert_eq!(unmatched("line 142", &["only 42 here"]).len(), 1);
        assert_eq!(unmatched("line 142", &["1420 lines"]).len(), 1);
        assert!(unmatched("line 142", &["at line 142."]).is_empty());
    }

    #[test]
    fn iso_dates_and_clock_times_are_not_entities() {
        assert!(extract("on 2026-11-26 at 12:30:00").is_empty());
    }

    #[test]
    fn evidence_from_any_source_counts_as_a_match() {
        let message = "version 1.2.3 in src/lib.rs, line 4096";
        let evidence = ["parser = \"1.2.3\"", "read src/lib.rs", "error at 4096"];
        assert!(unmatched(message, &evidence).is_empty());
    }

    #[test]
    fn an_unmatched_entity_carries_its_kind_and_a_hash_not_its_text() {
        let found = unmatched("see a60f12d", &["nothing relevant"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, EntityKind::Identifier);
        assert_eq!(found[0].hash.len(), 16);
        assert!(found[0].hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!found[0].hash.contains("a60f12d"));
        assert_eq!(found[0].hash, unmatched("see a60f12d", &[""])[0].hash);
    }

    #[test]
    fn kinds_have_the_names_the_record_schema_lists() {
        assert_eq!(EntityKind::Path.as_str(), "path");
        assert_eq!(EntityKind::Version.as_str(), "version");
        assert_eq!(EntityKind::Identifier.as_str(), "identifier");
        assert_eq!(EntityKind::Number.as_str(), "number");
    }
}
