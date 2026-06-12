//! Support labels and confidence — pure, deterministic functions
//! (research.md 004 D7; Principle V: the model never emits a label or a
//! confidence, these functions do).

use crate::modes::verify::VerdictKind;
use crate::research::Support;

/// Weight of vote agreement in claim confidence.
const W_AGREEMENT: f32 = 0.6;
/// Weight of independent-source corroboration (saturates at 3).
const W_SOURCES: f32 = 0.25;
/// Weight of mean source credibility.
const W_CREDIBILITY: f32 = 0.15;

/// Map a verify-ensemble result to a support label (FR-004).
///
/// **Order-sensitive** (analysis I2): the contested band is checked *before*
/// the aggregate verdict, because the verify ensemble resolves ties to
/// refuted — trusting the aggregate first would silently drop genuinely
/// contested claims. The band uses the integer rule `3·majority ≤ 2·completed`
/// (winning share ≤ 2/3): K=2 1–1 and K=3 2–1 are contested; K=1 never is.
#[must_use]
pub fn support(completed: u32, agreement: f64, verdict: VerdictKind, n_sources: usize) -> Support {
    // Reconstruct the majority count from the agreement ratio — exact for
    // ensemble-scale counts.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let majority = (agreement * f64::from(completed)).round() as u32;

    if 3 * majority <= 2 * completed {
        return Support::Contested;
    }
    if verdict == VerdictKind::Refuted {
        return Support::Refuted;
    }
    if n_sources >= 2 {
        Support::Confirmed
    } else {
        Support::Unverified
    }
}

/// Per-claim confidence: vote agreement + corroboration + credibility,
/// clamped to 0..=1. Weights are constants tuned offline, never at runtime.
#[must_use]
pub fn claim_confidence(agreement: f64, n_sources: usize, mean_credibility: f32) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let agreement = agreement as f32;
    #[allow(clippy::cast_precision_loss)]
    let corroboration = (n_sources.min(3) as f32) / 3.0;
    (W_AGREEMENT * agreement)
        .mul_add(1.0, W_SOURCES * corroboration)
        .mul_add(1.0, W_CREDIBILITY * mean_credibility)
        .clamp(0.0, 1.0)
}

/// Overall answer confidence: the mean of finding confidences, weighted by
/// coverage of the scoped sub-questions (FR-005) — settling 3 of 7
/// sub-questions caps confidence at 3/7 of the findings' mean.
#[must_use]
pub fn overall_confidence(
    finding_confidences: &[f32],
    settled_sub_questions: usize,
    total_sub_questions: usize,
) -> f32 {
    if finding_confidences.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let mean = finding_confidences.iter().sum::<f32>() / (finding_confidences.len() as f32);
    let coverage = if total_sub_questions == 0 {
        1.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        {
            (settled_sub_questions.min(total_sub_questions) as f32) / (total_sub_questions as f32)
        }
    };
    (mean * coverage).clamp(0.0, 1.0)
}

/// Heuristic source credibility — conservative and explainable (spec
/// assumption).
///
/// A base for any fetched page plus a bonus for documentation-class domains;
/// corroboration is handled separately in [`claim_confidence`].
#[must_use]
pub fn source_credibility(host: &str) -> f32 {
    const DOC_CLASS: &[&str] = &[".gov", ".edu", ".org"];
    let host = host.to_lowercase();
    let base = 0.5;
    let bonus = if DOC_CLASS.iter().any(|suffix| {
        host.ends_with(suffix) || host.contains(&format!("{suffix}.")) // ccTLD forms like .gov.uk
    }) {
        0.2
    } else {
        0.0
    };
    base + bonus
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
#[allow(clippy::suboptimal_flops)]
mod tests {
    use super::*;

    // Analysis I2: the contested band must catch what the verify ensemble's
    // tie-to-refuted resolution would otherwise silently drop.
    #[test]
    fn split_votes_are_contested_before_the_aggregate_verdict_is_trusted() {
        // K=2, 1–1: verify aggregates to refuted at agreement 0.5.
        assert_eq!(support(2, 0.5, VerdictKind::Refuted, 2), Support::Contested);
        // K=3, 2–1 refuted: share exactly 2/3 — still contested.
        assert_eq!(
            support(3, 2.0 / 3.0, VerdictKind::Refuted, 2),
            Support::Contested
        );
        // K=3, 2–1 supported: same band from the other side.
        assert_eq!(
            support(3, 2.0 / 3.0, VerdictKind::Supported, 2),
            Support::Contested
        );
    }

    #[test]
    fn decisive_votes_map_by_verdict_and_source_count() {
        // K=1 can never be contested (share 1).
        assert_eq!(support(1, 1.0, VerdictKind::Refuted, 1), Support::Refuted);
        assert_eq!(
            support(1, 1.0, VerdictKind::Supported, 1),
            Support::Unverified
        );
        assert_eq!(
            support(1, 1.0, VerdictKind::Supported, 2),
            Support::Confirmed
        );
        // K=3 unanimous.
        assert_eq!(support(3, 1.0, VerdictKind::Refuted, 3), Support::Refuted);
        assert_eq!(
            support(3, 1.0, VerdictKind::Supported, 3),
            Support::Confirmed
        );
        // K=5, 4–1: share 0.8 > 2/3 — decided.
        assert_eq!(
            support(5, 0.8, VerdictKind::Supported, 1),
            Support::Unverified
        );
    }

    #[test]
    fn claim_confidence_is_monotone_and_clamped() {
        let lo = claim_confidence(0.5, 1, 0.5);
        let hi = claim_confidence(1.0, 3, 0.7);
        assert!(lo < hi);
        assert!((0.0..=1.0).contains(&lo) && (0.0..=1.0).contains(&hi));
        // Corroboration saturates at 3 sources.
        assert_eq!(
            claim_confidence(1.0, 3, 0.5),
            claim_confidence(1.0, 30, 0.5)
        );
    }

    // FR-005: coverage penalizes unanswered sub-questions.
    #[test]
    fn overall_confidence_is_coverage_weighted() {
        let findings = [0.9, 0.9, 0.9];
        let full = overall_confidence(&findings, 7, 7);
        let partial = overall_confidence(&findings, 3, 7);
        assert!(partial < full);
        assert!((partial - 0.9 * (3.0 / 7.0)).abs() < 1e-6);
        // No findings → zero, regardless of coverage.
        assert_eq!(overall_confidence(&[], 7, 7), 0.0);
        // Zero sub-questions → no penalty.
        assert_eq!(overall_confidence(&[0.8], 0, 0), 0.8);
    }

    #[test]
    fn credibility_is_conservative_and_bounded() {
        assert!(source_credibility("example.com") < source_credibility("nist.gov"));
        assert_eq!(
            source_credibility("nist.gov"),
            source_credibility("ons.gov.uk")
        );
        for host in ["example.com", "nist.gov", "mit.edu", "wikipedia.org"] {
            let c = source_credibility(host);
            assert!((0.0..=1.0).contains(&c), "{host}: {c}");
        }
    }
}
