//! Tests for the Jev measurement library (064 T029).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use super::*;
use crate::jev::policy::Band;

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} vs {expected}"
    );
}

fn items(pairs: &[(Decision, bool)]) -> Vec<(Decision, bool)> {
    pairs.to_vec()
}

#[test]
fn confusion_counts_are_hand_computed() {
    use Decision::{No, Undecided, Yes};
    let c = Confusion::from_pairs(&items(&[
        (Yes, true),
        (Yes, true),
        (Yes, false),
        (No, true),
        (No, false),
        (No, false),
        (Undecided, true),
        (Undecided, false),
    ]));
    assert_eq!((c.tp, c.fp, c.tn, c.fn_), (2, 1, 2, 1));
    assert_eq!((c.undecided_positive, c.undecided_negative), (1, 1));
    close(c.precision().unwrap(), 2.0 / 3.0, 1e-12);
    close(c.recall().unwrap(), 2.0 / 3.0, 1e-12);
    assert_eq!(c.positives(), 4);
}

#[test]
fn undefined_precision_and_recall_are_none_not_nan() {
    let c = Confusion::from_pairs(&items(&[(Decision::No, false)]));
    assert_eq!(c.precision(), None);
    assert_eq!(c.recall(), None);
    assert_eq!(skip_recall(&c), None);
}

#[test]
fn excluded_and_missing_items_count_as_undecided() {
    assert_eq!(Decision::from(Band::Missing), Decision::Undecided);
    assert_eq!(Decision::from(Band::Middle), Decision::Undecided);
    assert_eq!(Decision::from(Band::Yes), Decision::Yes);
    assert_eq!(Decision::from(Band::No), Decision::No);
    assert_eq!(Decision::from(None::<Band>), Decision::Undecided);
}

#[test]
fn skip_recall_counts_only_skipped_positives_as_misses() {
    use Decision::{No, Undecided, Yes};
    // Skip-gate reading: `No` means "no harm, skip the big model".
    let c = Confusion::from_pairs(&items(&[
        (Yes, true),
        (Undecided, true),
        (No, true),
        (No, false),
    ]));
    // Three harmful positives; only the skipped one is a miss.
    close(skip_recall(&c).unwrap(), 2.0 / 3.0, 1e-12);
    assert_eq!(c.skips(), 2);
}

#[test]
fn wilson_and_clopper_pearson_match_published_values() {
    let (lo, hi) = wilson(8, 10, 1.959_964).unwrap();
    close(lo, 0.4902, 1e-3);
    close(hi, 0.9433, 1e-3);
    let (lo, hi) = clopper_pearson(8, 10, 0.05).unwrap();
    close(lo, 0.4439, 1e-3);
    close(hi, 0.9748, 1e-3);
    close(clopper_pearson_lower(59, 59, 0.05).unwrap(), 0.9505, 1e-3);
    close(clopper_pearson_lower(0, 10, 0.05).unwrap(), 0.0, 1e-12);
    assert_eq!(wilson(0, 0, 1.96), None);
    assert_eq!(clopper_pearson_lower(1, 0, 0.05), None);
}

#[test]
fn the_threshold_grid_is_the_distinct_observed_scores_in_hundredths() {
    assert_eq!(
        threshold_grid(&[0.314, 0.31, 0.9, 0.0, 1.0, 0.9]),
        [0, 31, 90, 100]
    );
}

#[test]
fn the_chosen_threshold_is_the_smallest_whose_recall_bound_meets_the_target() {
    // (P(safe to skip), harmful). Skipping happens at P >= threshold.
    let mut scored: Vec<(f64, bool)> = Vec::new();
    for _ in 0..100 {
        scored.push((0.2, true)); // harmful items Jev scores low
    }
    for _ in 0..100 {
        scored.push((0.95, false)); // safe items Jev scores high
    }
    scored.push((0.6, true)); // one harmful item at 0.6
                              // At 0.60 the 0.6 item is skipped (a miss): bound 100/101 lower ~0.95.
                              // At 0.95 nothing harmful is skipped: bound 101/101 lower ~0.971.
    let t = choose_skip_threshold(&scored, 0.96, 0.05).unwrap();
    close(t, 0.95, 1e-12);
    assert_eq!(choose_skip_threshold(&scored, 0.999, 0.05), None);
}

#[test]
fn splits_are_deterministic_and_disjoint() {
    let ids: Vec<String> = (0..1000).map(|i| format!("item-{i}")).collect();
    let first: Vec<Split> = ids.iter().map(|id| split(id, 30)).collect();
    let again: Vec<Split> = ids.iter().map(|id| split(id, 30)).collect();
    assert_eq!(first, again);
    let confirm = first.iter().filter(|s| **s == Split::Confirm).count();
    assert!((250..=350).contains(&confirm), "{confirm}");
}

#[test]
fn every_confirm_look_is_logged_with_the_question_version() {
    let mut log = ConfirmLog::default();
    log.look("contradiction", 1, "first promotion check");
    log.look("contradiction", 1, "second look");
    assert_eq!(log.looks().len(), 2);
    assert_eq!(log.looks_for("contradiction", 1), 2);
    assert_eq!(log.looks_for("contradiction", 2), 0);
}

#[test]
fn the_cluster_bootstrap_is_seeded_and_resamples_whole_clusters() {
    use Decision::{No, Yes};
    let clusters = vec![
        vec![(Yes, true), (Yes, true), (No, true)],
        vec![(Yes, true), (No, false)],
        vec![(No, true), (Yes, true)],
    ];
    let a = cluster_bootstrap(&clusters, Confusion::recall, 500, 7).unwrap();
    let b = cluster_bootstrap(&clusters, Confusion::recall, 500, 7).unwrap();
    assert_eq!(a, b);
    assert!(a.0 <= a.1);
    // Identical clusters give a degenerate interval.
    let same = vec![vec![(Yes, true), (No, true)]; 4];
    let (lo, hi) = cluster_bootstrap(&same, Confusion::recall, 200, 1).unwrap();
    close(lo, 0.5, 1e-12);
    close(hi, 0.5, 1e-12);
    assert_eq!(cluster_bootstrap(&[], Confusion::recall, 10, 1), None);
}

#[test]
fn baselines_always_no_and_always_yes() {
    let labels = [true, true, false];
    let yes = Confusion::always(Decision::Yes, &labels);
    let no = Confusion::always(Decision::No, &labels);
    assert_eq!(yes.recall(), Some(1.0));
    assert_eq!(no.recall(), Some(0.0));
    assert_eq!(skip_recall(&no), Some(0.0));
    assert_eq!(no.skips(), 3);
    assert_eq!(yes.skips(), 0);
}

#[test]
fn promotion_decision_returns_its_reasons() {
    use Decision::{No, Yes};
    // 59 harmful positives, none skipped; 40 safe items skipped.
    let mut pairs = vec![(Yes, true); 59];
    pairs.extend(vec![(No, false); 40]);
    let passing = PromotionInput {
        confusion: Confusion::from_pairs(&pairs),
        clusters: 12,
        target_recall: 0.95,
        alpha: 0.05,
        min_positives: 59,
    };
    let decision = promotion_decision(&passing);
    assert!(decision.promote, "{:?}", decision.reasons);
    assert!(decision.reasons.is_empty());

    let failing = PromotionInput {
        confusion: Confusion::from_pairs(&[(Yes, true); 10]),
        clusters: 0,
        target_recall: 0.95,
        alpha: 0.05,
        min_positives: 59,
    };
    let decision = promotion_decision(&failing);
    assert!(!decision.promote);
    let joined = decision.reasons.join(" | ");
    for expected in ["positives", "recall", "baseline", "clusters"] {
        assert!(joined.contains(expected), "missing {expected}: {joined}");
    }
}
