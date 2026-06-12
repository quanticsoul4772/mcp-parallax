//! The grounding gate — pure, deterministic (FR-003; research.md 004 D7).
//!
//! The model can only fabricate a citation *token*, never a finding or a
//! label (those are server-assembled) — and a token is a string check. The
//! gate validates every `[sN]` in the answer prose and every finding's
//! source ids against the fetched-source map, rejects *near-miss* citation
//! forms (`[S3]`, `[s1, s2]`, unclosed `[s12`) instead of ignoring them, and
//! prunes uncited sources. Violations are returned as exact descriptions for
//! the single retry.
//!
//! Scope note: answer citations are validated against the *fetched*-source
//! map (the fabrication check), not narrowed to finding-backed sources — a
//! cited source that contributed no surviving claim is real, just weak; the
//! per-finding check still requires every finding to cite fetched sources.

use std::collections::BTreeSet;

/// One pass over the prose: strictly-valid `[sN]` tokens (first-appearance
/// order, deduplicated) and near-miss citation attempts (verbatim excerpts).
fn scan(answer: &str) -> (Vec<String>, Vec<String>) {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    let mut malformed = Vec::new();
    let bytes = answer.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // A citation attempt starts `[` + `s`/`S` + digit.
        if bytes[i] == b'['
            && i + 2 < bytes.len()
            && matches!(bytes[i + 1], b's' | b'S')
            && bytes[i + 2].is_ascii_digit()
        {
            let digits_start = i + 2;
            let mut j = digits_start;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if bytes[i + 1] == b's' && j < bytes.len() && bytes[j] == b']' {
                // Strict form: lowercase s, digits, immediate close.
                let id = &answer[i + 1..j];
                if seen.insert(id.to_string()) {
                    valid.push(id.to_string());
                }
                i = j + 1;
                continue;
            }
            // Near-miss: uppercase S, a comma list, trailing junk, or an
            // unclosed bracket. Capture a short excerpt for the violation.
            let end = answer[i..]
                .char_indices()
                .take(20)
                .last()
                .map_or(answer.len(), |(offset, c)| i + offset + c.len_utf8());
            let close = answer[i..end].find(']').map(|p| i + p + 1);
            malformed.push(answer[i..close.unwrap_or(end)].to_string());
            i = close.unwrap_or(j);
            continue;
        }
        i += 1;
    }
    (valid, malformed)
}

/// Extract strictly-valid `[sN]` citation tokens from prose, in order of
/// first appearance, deduplicated.
#[must_use]
pub fn citation_tokens(answer: &str) -> Vec<String> {
    scan(answer).0
}

/// The gate's success output: the source ids to keep, answer-citation order
/// first, then finding-only ids in input order (uncited sources are pruned —
/// "no listed source is uncited dead weight").
#[derive(Debug, PartialEq, Eq)]
pub struct Grounded {
    /// Source ids that survive pruning.
    pub kept_source_ids: Vec<String>,
}

/// Validate the synthesis against the fetched-source map.
///
/// # Errors
///
/// Returns the exact violations (for the one retry prompt) when the answer
/// cites an unfetched source, uses a malformed citation form, or a finding
/// references an unfetched source.
pub fn ground(
    answer: &str,
    finding_source_ids: &[Vec<String>],
    fetched_ids: &BTreeSet<String>,
) -> Result<Grounded, Vec<String>> {
    let mut violations = Vec::new();

    let (answer_citations, malformed) = scan(answer);
    for excerpt in &malformed {
        violations.push(format!(
            "the answer contains a malformed citation {excerpt:?} — cite exactly one source \
             per token in the form [s3]"
        ));
    }
    for id in &answer_citations {
        if !fetched_ids.contains(id) {
            violations.push(format!(
                "the answer cites [{id}] but no source {id} was fetched in this run"
            ));
        }
    }

    for (index, ids) in finding_source_ids.iter().enumerate() {
        if ids.is_empty() {
            violations.push(format!("finding {index} carries no source"));
        }
        for id in ids {
            if !fetched_ids.contains(id) {
                violations.push(format!(
                    "finding {index} references source {id}, which was not fetched in this run"
                ));
            }
        }
    }

    if !violations.is_empty() {
        return Err(violations);
    }

    // Keep answer-cited sources first (citation order), then finding-only
    // sources; everything uncited is pruned.
    let mut kept = answer_citations;
    let mut seen: BTreeSet<String> = kept.iter().cloned().collect();
    for ids in finding_source_ids {
        for id in ids {
            if seen.insert(id.clone()) {
                kept.push(id.clone());
            }
        }
    }
    Ok(Grounded {
        kept_source_ids: kept,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn fetched(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn tokens_parse_in_order_deduplicated() {
        let answer = "Per [s3], X holds [s1]; [s3] repeats. [s] and [x2] are not citations.";
        assert_eq!(citation_tokens(answer), ["s3", "s1"]);
    }

    #[test]
    fn a_fabricated_citation_token_is_a_named_violation() {
        let err = ground("Cited [s9].", &[], &fetched(&["s1", "s2"])).unwrap_err();
        assert_eq!(err.len(), 1);
        assert!(
            err[0].contains("[s9]") && err[0].contains("was fetched"),
            "{}",
            err[0]
        );
    }

    #[test]
    fn near_miss_citations_are_violations_not_ignored() {
        for (answer, marker) in [
            ("Holds [S3].", "[S3]"),                   // uppercase
            ("Holds [s1, s2].", "[s1,"),               // comma list
            ("Holds [s12 and more text here", "[s12"), // unclosed
            ("Holds [s4x].", "[s4x]"),                 // trailing junk
        ] {
            let err = ground(answer, &[], &fetched(&["s1", "s2", "s3", "s4", "s12"])).unwrap_err();
            assert!(
                err.iter()
                    .any(|v| v.contains("malformed") && v.contains(marker)),
                "{answer}: {err:?}"
            );
        }
    }

    #[test]
    fn a_finding_with_an_unfetched_or_missing_source_is_a_violation() {
        let err = ground(
            "All grounded [s1].",
            &[vec!["s1".into()], vec![], vec!["s7".into()]],
            &fetched(&["s1"]),
        )
        .unwrap_err();
        assert_eq!(err.len(), 2);
        assert!(err[0].contains("finding 1") && err[0].contains("no source"));
        assert!(err[1].contains("finding 2") && err[1].contains("s7"));
    }

    #[test]
    fn uncited_sources_are_pruned_and_order_is_citation_first() {
        let grounded = ground(
            "B first [s2], then A [s1].",
            &[vec!["s1".into()], vec!["s4".into()]],
            &fetched(&["s1", "s2", "s3", "s4"]),
        )
        .unwrap();
        // s3 was fetched but cited nowhere — pruned. Citation order, then
        // finding-only.
        assert_eq!(grounded.kept_source_ids, ["s2", "s1", "s4"]);
    }

    #[test]
    fn clean_pass_with_no_citations_keeps_nothing() {
        let grounded = ground("No citations at all.", &[], &fetched(&["s1"])).unwrap();
        assert!(grounded.kept_source_ids.is_empty());
    }
}
