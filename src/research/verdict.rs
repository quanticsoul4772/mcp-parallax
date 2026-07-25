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

/// Overall answer confidence: the mean support of the findings the answer
/// asserts (021 FR-001).
///
/// **Not weighted by coverage.** It was until 2026-07-25, and that multiplier
/// was a defect: coverage was derived by subtracting the length of the
/// synthesis pass's free-form gap list from the count of scoped sub-questions,
/// two lists with no correspondence, and the gap cap exceeded the sub-question
/// cap — so the term reached exactly zero by construction. Two live runs
/// reported confidence 0 for factually correct answers whose every claim had
/// survived refute-biased verification at ~0.78. A confidence of exactly 0
/// asserts certainty of falsehood, and a caller that learns the field reads 0
/// on correct answers stops reading it at all.
///
/// Breadth of resolution did not disappear; it moved to [`coverage`], which is
/// published in its own right. Folding two quantities into one number is what
/// made the result indiscriminate.
///
/// Zero is reserved for the case it genuinely describes: no claim was
/// supported (FR-008).
#[must_use]
pub fn overall_confidence(finding_confidences: &[f32]) -> f32 {
    if finding_confidences.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)] // counts are ensemble-scale
    let mean = finding_confidences.iter().sum::<f32>() / (finding_confidences.len() as f32);
    mean.clamp(0.0, 1.0)
}

/// Proportion of the run's scoped sub-questions that no gap claims (021
/// FR-002).
///
/// `targets` are the synthesis pass's per-gap keys: 1-based sub-question
/// numbers, with `0` meaning the gap concerns no single sub-question. Keys
/// outside `1..=sub_questions` identify nothing in this run and are discarded
/// (FR-006); a sub-question named by several gaps counts unsettled **once**
/// (FR-004) — counting it repeatedly is what let a verbose gap list annihilate
/// a well-supported answer.
///
/// A run that scoped nothing has nothing unsettled, so coverage is 1.0
/// (FR-007) rather than a division by zero.
///
/// The association comes from the keys, never from comparing gap text to
/// sub-question text: whether one natural-language string answers another is a
/// semantic relation, not a syntactic one, so a lexical rule for it is
/// reproducibly wrong rather than merely imprecise.
#[must_use]
pub fn coverage(sub_questions: usize, targets: &[u32]) -> f32 {
    if sub_questions == 0 {
        return 1.0;
    }
    let mut claimed = vec![false; sub_questions];
    for &target in targets {
        // 0 = concerns no single sub-question; out of range = none here.
        if let Some(slot) = (target as usize)
            .checked_sub(1)
            .and_then(|i| claimed.get_mut(i))
        {
            *slot = true;
        }
    }
    let settled = claimed.iter().filter(|claimed| !**claimed).count();
    #[allow(clippy::cast_precision_loss)] // bounded by MAX_SUB_QUESTIONS
    let ratio = (settled as f32) / (sub_questions as f32);
    ratio.clamp(0.0, 1.0)
}

/// The share of verified claims that verification refuted (021 FR-009a).
///
/// Reported beside [`overall_confidence`] rather than folded into it. The
/// answer does not assert refuted claims, so confidence is right to ignore
/// them — but without this figure a run that refuted nine of ten claims would
/// report the same confidence as one that refuted none, which is the same
/// indiscriminate fold the coverage multiplier was removed for.
///
/// Zero when nothing was verified: defined, never a division by zero.
#[must_use]
pub fn refutation_rate(refuted: usize, verified: usize) -> f32 {
    if verified == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)] // counts are claim-scale
    let rate = (refuted as f32) / (verified as f32);
    rate.clamp(0.0, 1.0)
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

    // 021 FR-001 / SC-004: confidence is the findings' mean support and nothing
    // else. It is NOT reduced by unsettled sub-questions — that reduction is
    // the defect this feature removes, and it drove correct answers to
    // exactly 0. Breadth lives in `coverage`, reported separately.
    #[test]
    fn overall_confidence_is_the_findings_mean_and_ignores_breadth() {
        assert!((overall_confidence(&[0.9, 0.9, 0.9]) - 0.9).abs() < 1e-6);
        assert!((overall_confidence(&[0.6, 0.8]) - 0.7).abs() < 1e-6);
        // FR-008 / SC-004: zero is reserved for "no claim was supported" —
        // the one case the value genuinely describes.
        assert_eq!(overall_confidence(&[]), 0.0);
        // A single well-supported finding is not penalised for being alone.
        assert!((overall_confidence(&[0.78]) - 0.78).abs() < 1e-6);
    }

    // 021 FR-009a: refuted claims are excluded from confidence (the answer
    // does not assert them) and surfaced as their own rate instead, so a run
    // whose evidence largely fell apart is distinguishable from one whose
    // evidence held.
    #[test]
    fn refutation_rate_is_the_refuted_share_of_verified_claims() {
        assert!((refutation_rate(0, 10) - 0.0).abs() < 1e-6);
        assert!((refutation_rate(5, 10) - 0.5).abs() < 1e-6);
        assert!((refutation_rate(10, 10) - 1.0).abs() < 1e-6);
        // No claim verified: defined, not a division by zero.
        assert_eq!(refutation_rate(0, 0), 0.0);
        // Never outside 0..=1 even if the counts are inconsistent.
        assert!((0.0..=1.0).contains(&refutation_rate(12, 10)));
    }

    // 021 T012: the coverage boundary table (data-model.md). Each row is a
    // requirement, and the duplicate-target row is the specific arithmetic
    // whose absence produced the observed collapse.
    #[test]
    fn coverage_counts_unclaimed_sub_questions_deterministically() {
        // FR-007: nothing was scoped, so nothing is unsettled.
        assert!((coverage(0, &[]) - 1.0).abs() < f32::EPSILON);
        // No gap targets anything: fully settled.
        assert!((coverage(3, &[]) - 1.0).abs() < f32::EPSILON);
        // Every sub-question claimed: nothing settled. Legal, and no longer
        // able to reach `confidence`.
        assert!((coverage(3, &[1, 2, 3]) - 0.0).abs() < f32::EPSILON);
        // Partial.
        assert!((coverage(4, &[2, 3]) - 0.5).abs() < f32::EPSILON);
        // FR-004: several gaps on one sub-question count it unsettled ONCE.
        // Counting them repeatedly is what drove coverage negative-then-zero.
        assert!((coverage(3, &[2, 2, 2, 2, 2]) - (2.0 / 3.0)).abs() < 1e-6);
        // FR-006: an out-of-range key identifies no sub-question of this run
        // and is discarded rather than corrupting the count.
        assert!((coverage(2, &[9]) - 1.0).abs() < f32::EPSILON);
        assert!((coverage(2, &[1, 42]) - 0.5).abs() < f32::EPSILON);
        // FR-009: 0 means "concerns no single sub-question" — a grounding-gate
        // gap, say — and must not suppress coverage.
        assert!((coverage(2, &[0, 0]) - 1.0).abs() < f32::EPSILON);
        // Always a proportion.
        for targets in [vec![], vec![1], vec![1, 1, 2], vec![0, 7]] {
            assert!((0.0..=1.0).contains(&coverage(2, &targets)), "{targets:?}");
        }
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
