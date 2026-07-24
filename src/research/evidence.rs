//! Claim-relevant evidence excerpts for the verification hop (004 D3/D4
//! amendment, 2026-07-24).
//!
//! The refute-biased verifier must judge a claim against the fetched source
//! text, not against the judging model's priors — a prior-only judge
//! systematically refutes any true fact newer than its training cutoff,
//! which is exactly the class of question research exists to answer. These
//! pure functions select the bounded slice of a source's readable text most
//! relevant to a claim; the pipeline hands it to the verify context.

use std::collections::BTreeSet;

/// Max characters of excerpt per source in a verify context.
pub const EVIDENCE_EXCERPT_MAX_CHARS: usize = 4_000;
/// Max sources excerpted per claim (corroboration saturates at 3 in
/// [`crate::research::verdict::claim_confidence`] — the same bound).
pub const EVIDENCE_SOURCES_MAX: usize = 3;

/// Minimum word length counted toward claim/paragraph overlap — drops
/// articles and glue words without a stopword list.
const OVERLAP_WORD_MIN_CHARS: usize = 3;

/// Deterministically select the excerpt of `text` most relevant to `claim`,
/// capped at `max_chars` characters.
///
/// Paragraph-scored word overlap: the paragraph sharing the most distinct
/// claim words anchors the excerpt, which then grows by alternately
/// appending the following and preceding paragraphs while the cap allows.
/// Text already within the cap returns whole; zero overlap anywhere anchors
/// at the head (claims concentrate early — the extraction cap's own
/// assumption).
#[must_use]
pub fn evidence_excerpt(text: &str, claim: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let paragraphs: Vec<&str> = text
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if paragraphs.is_empty() {
        return text.chars().take(max_chars).collect();
    }

    let claim_words = overlap_words(claim);
    let anchor = paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| (i, overlap_words(p).intersection(&claim_words).count()))
        .max_by(|(i, a), (j, b)| a.cmp(b).then(j.cmp(i))) // ties → earliest
        .map_or(0, |(i, _)| i);

    // Grow around the anchor: alternately the next and previous paragraph,
    // preserving original order in the assembled excerpt.
    let mut budget = max_chars;
    let mut lo = anchor;
    let mut hi = anchor;
    let mut selected: Vec<usize> = Vec::new();
    let take = |index: usize, budget: &mut usize, selected: &mut Vec<usize>| {
        let cost = paragraphs[index].chars().count() + usize::from(!selected.is_empty());
        if cost > *budget {
            return false;
        }
        *budget -= cost;
        selected.push(index);
        true
    };
    if !take(anchor, &mut budget, &mut selected) {
        // The anchor alone exceeds the cap — return its head.
        return paragraphs[anchor].chars().take(max_chars).collect();
    }
    loop {
        let grew_next = hi + 1 < paragraphs.len() && take(hi + 1, &mut budget, &mut selected);
        if grew_next {
            hi += 1;
        }
        let grew_prev = lo > 0 && take(lo - 1, &mut budget, &mut selected);
        if grew_prev {
            lo -= 1;
        }
        if !grew_next && !grew_prev {
            break;
        }
    }
    selected.sort_unstable();
    selected
        .iter()
        .map(|&i| paragraphs[i])
        .collect::<Vec<_>>()
        .join("\n")
}

/// The distinct lowercase alphanumeric words of `text` counted toward
/// overlap (length ≥ [`OVERLAP_WORD_MIN_CHARS`]).
fn overlap_words(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= OVERLAP_WORD_MIN_CHARS)
        .map(String::from)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn short_text_returns_whole() {
        let text = "Rust 1.97.0 was released on July 9, 2026.";
        assert_eq!(evidence_excerpt(text, "irrelevant claim", 4_000), text);
    }

    #[test]
    fn anchor_is_the_overlapping_paragraph_deep_in_the_text() {
        let filler = "unrelated filler prose about something else entirely.";
        let mut lines: Vec<String> = (0..200).map(|i| format!("{filler} ({i})")).collect();
        lines.push("The compiler version 1.97.0 shipped on 2026-07-09.".to_string());
        let text = lines.join("\n");
        let excerpt = evidence_excerpt(&text, "Rust 1.97.0 was released on 2026-07-09", 200);
        assert!(excerpt.contains("1.97.0 shipped"), "{excerpt}");
    }

    #[test]
    fn cap_is_respected_and_growth_prefers_neighbors() {
        let paragraphs: Vec<String> = (0..50)
            .map(|i| {
                if i == 25 {
                    format!("item {i} special marker tokens")
                } else {
                    format!("item {i} plain body")
                }
            })
            .collect();
        let text = paragraphs.join("\n");
        let excerpt = evidence_excerpt(&text, "special marker tokens", 100);
        assert!(excerpt.chars().count() <= 100);
        assert!(excerpt.contains("special marker"));
        // Neighbors of the anchor, not distant paragraphs.
        assert!(excerpt.contains("item 24") || excerpt.contains("item 26"));
        assert!(!excerpt.contains("item 0 "));
        assert!(!excerpt.contains("item 49"));
    }

    #[test]
    fn zero_overlap_anchors_at_the_head() {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i} content here")).collect();
        let text = lines.join("\n");
        let excerpt = evidence_excerpt(&text, "zzz qqq xxx", 80);
        assert!(excerpt.starts_with("line 0"), "{excerpt}");
    }

    #[test]
    fn oversized_single_paragraph_returns_its_head() {
        let text = format!("alpha {}", "word ".repeat(2_000));
        let excerpt = evidence_excerpt(&text, "alpha", 50);
        assert_eq!(excerpt.chars().count(), 50);
        assert!(excerpt.starts_with("alpha"));
    }

    #[test]
    fn ties_resolve_to_the_earliest_paragraph() {
        let text = format!(
            "match target early\n{}\nmatch target late",
            "filler line without overlap\n".repeat(60)
        );
        let excerpt = evidence_excerpt(&text, "match target", 40);
        assert!(excerpt.contains("early"), "{excerpt}");
        assert!(!excerpt.contains("late"), "{excerpt}");
    }
}
