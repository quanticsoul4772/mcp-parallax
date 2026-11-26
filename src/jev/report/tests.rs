#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::checkpoint::review::{CaptureKind, ReviewRaw};
use crate::jev::shadow::{turn_label, ReviewOutcome};
use serde_json::json;

fn turn_questions(contradiction: f64, capture: Option<f64>) -> Value {
    let mut qs = vec![json!({ "id": "contradiction", "version": 2, "answer": contradiction })];
    if let Some(p) = capture {
        qs.push(json!({ "id": "capture_worthy", "version": 2, "answer": p }));
    }
    Value::Array(qs)
}

const fn raw(contradicts: bool, violates: bool, capture_kind: CaptureKind) -> ReviewRaw {
    ReviewRaw {
        contradicts,
        violates,
        capture_kind,
    }
}

const fn outcome(
    contradiction_flagged: bool,
    violation_flagged: bool,
    captured: bool,
) -> ReviewOutcome {
    ReviewOutcome {
        contradiction_flagged,
        violation_flagged,
        captured,
    }
}

/// A label exactly as the server writes it, for a turn without memory pairs
/// and with capture on.
fn label(raw: ReviewRaw, did: ReviewOutcome) -> Value {
    turn_label(&raw, false, true, did)
}

fn harmful(l: &Value) -> bool {
    turn_item(Some(l), &turn_questions(0.2, Some(0.1)), Some("s"))
        .unwrap()
        .harmful
}

#[test]
fn a_turn_is_harmful_only_when_the_review_actually_flagged_or_captured() {
    let none = CaptureKind::None;
    let flagged_contradiction = label(raw(true, false, none), outcome(true, false, false));
    assert!(harmful(&flagged_contradiction));
    // A raw violation with no preferences listed raises no flag.
    let unflagged_violation = label(raw(false, true, none), outcome(false, false, false));
    assert!(!harmful(&unflagged_violation));
    let flagged_violation = label(raw(false, true, none), outcome(false, true, false));
    assert!(harmful(&flagged_violation));
    // A raw capture kind the review did not turn into a proposal.
    let unproposed = label(
        raw(false, false, CaptureKind::Lesson),
        outcome(false, false, false),
    );
    assert!(!harmful(&unproposed));
    let proposed = label(
        raw(false, false, CaptureKind::Lesson),
        outcome(false, false, true),
    );
    assert!(harmful(&proposed));
    let capture_off = turn_label(
        &raw(false, false, CaptureKind::Lesson),
        false,
        false,
        outcome(false, false, false),
    );
    assert!(!harmful(&capture_off));
}

#[test]
fn every_memory_pair_turn_is_unlabeled_flagged_or_not() {
    let qs = turn_questions(0.2, None);
    let none = CaptureKind::None;
    let violation = turn_label(
        &raw(true, true, none),
        true,
        true,
        outcome(true, true, false),
    );
    assert_eq!(
        turn_item(Some(&violation), &qs, None).unwrap_err(),
        Skip::Unlabeled
    );
    let capture = turn_label(
        &raw(false, false, CaptureKind::Skill),
        true,
        true,
        outcome(false, false, true),
    );
    assert_eq!(
        turn_item(Some(&capture), &qs, None).unwrap_err(),
        Skip::Unlabeled
    );
    let clean = turn_label(
        &raw(true, false, none),
        true,
        true,
        outcome(true, false, false),
    );
    assert_eq!(
        turn_item(Some(&clean), &qs, None).unwrap_err(),
        Skip::Unlabeled
    );
}

#[test]
fn labels_without_the_review_outcome_are_unlabeled() {
    let qs = turn_questions(0.2, None);
    let old = json!({ "contradicts": true, "violates": false, "capture_kind": null });
    assert_eq!(
        turn_item(Some(&old), &qs, None).unwrap_err(),
        Skip::Unlabeled
    );
    assert_eq!(turn_item(None, &qs, None).unwrap_err(), Skip::Unlabeled);
}

#[test]
fn a_turns_safe_score_is_one_minus_the_larger_answer() {
    let l = label(
        raw(false, false, CaptureKind::None),
        outcome(false, false, false),
    );
    let item = turn_item(Some(&l), &turn_questions(0.2, Some(0.35)), Some("s")).unwrap();
    assert!((item.safe.unwrap() - 0.65).abs() < 1e-12);
    assert_eq!(item.cluster.as_deref(), Some("s"));
    let item = turn_item(Some(&l), &turn_questions(0.3, None), None).unwrap();
    assert!((item.safe.unwrap() - 0.7).abs() < 1e-12);
    assert_eq!(item.cluster, None);
}

#[test]
fn a_labeled_turn_without_answers_is_kept_unscored() {
    let l = label(
        raw(true, false, CaptureKind::None),
        outcome(true, false, false),
    );
    let missing = json!([{ "id": "contradiction", "version": 2 }]);
    let item = turn_item(Some(&l), &missing, Some("s")).unwrap();
    assert_eq!(item.safe, None);
    assert!(item.harmful);
}

fn relation(id: &str, supports: f64) -> Value {
    json!({ "id": id, "answer": "supports",
            "probabilities": { "says_nothing": 0.0, "contradicts": 1.0 - supports, "supports": supports } })
}

#[test]
fn a_claim_is_harmful_when_the_ensemble_refuted_or_contested_it() {
    let qs = json!([relation("relation_1", 0.9)]);
    let harmful = |s: &str| {
        claim_item(Some(&json!({ "support": s })), &qs, Some("run"))
            .unwrap()
            .harmful
    };
    assert!(harmful("refuted"));
    assert!(harmful("contested"));
    assert!(!harmful("confirmed"));
    assert!(!harmful("unverified"));
    let odd = json!({ "support": "maybe" });
    assert_eq!(
        claim_item(Some(&odd), &qs, None).unwrap_err(),
        Skip::Unlabeled
    );
}

#[test]
fn a_claims_safe_score_is_the_smallest_supports_probability() {
    let qs = json!([
        relation("relation_1", 0.9),
        { "id": "same_subject_1", "answer": 0.2 },
        relation("relation_2", 0.6),
    ]);
    let l = json!({ "support": "confirmed" });
    let item = claim_item(Some(&l), &qs, Some("run")).unwrap();
    assert!((item.safe.unwrap() - 0.6).abs() < 1e-12);
    assert_eq!(item.cluster.as_deref(), Some("run"));
    let none = json!([{ "id": "same_subject_1", "answer": 0.2 }]);
    assert_eq!(claim_item(Some(&l), &none, None).unwrap().safe, None);
    let unanswered = json!([{ "id": "relation_1" }]);
    let refuted = json!({ "support": "refuted" });
    let item = claim_item(Some(&refuted), &unanswered, None).unwrap();
    assert_eq!(item.safe, None);
    assert!(item.harmful);
}

fn item(safe: f64, harmful: bool, cluster: &str) -> Item {
    Item {
        safe: Some(safe),
        harmful,
        cluster: Some(cluster.into()),
    }
}

fn separated() -> Vec<Item> {
    let mut items: Vec<Item> = (0..59)
        .map(|i| item(0.1, true, &format!("c{}", i % 7)))
        .collect();
    items.extend((0..41).map(|i| item(0.9, false, &format!("c{}", i % 7))));
    items
}

#[test]
fn unscored_items_are_never_skipped_but_count_in_every_share() {
    let mut items = separated();
    items.extend((0..10).map(|i| Item {
        safe: None,
        harmful: i < 2,
        cluster: Some("c0".into()),
    }));
    let t = site_tables(&items);
    assert_eq!(t.threshold, Some(0.9));
    assert_eq!(t.items, 110);
    assert_eq!(t.unscored, 10);
    assert_eq!(t.harmful, 61);
    assert_eq!(t.confusion.skips(), 41);
    assert!((t.skip_share.unwrap() - 41.0 / 110.0).abs() < 1e-12);
    assert_eq!(t.skip_recall, Some(1.0));
}

#[test]
fn the_skip_share_lower_bound_is_one_sided_at_alpha() {
    let t = site_tables(&separated());
    let lower = t.skip_share_lower.unwrap();
    assert!((0.332..0.333).contains(&lower), "{lower}");
}

#[test]
fn items_without_a_cluster_are_not_a_cluster() {
    let mut items = separated();
    for i in &mut items {
        i.cluster = None;
    }
    let t = site_tables(&items);
    assert_eq!(t.clusters, 0);
    assert_eq!(t.unclustered, 100);
    assert_eq!(t.recall_bootstrap, None);
    assert!(t
        .dry_run
        .reasons
        .iter()
        .any(|r| r == "no clusters reported"));
}

#[test]
fn the_dry_run_names_the_criteria_it_does_not_check() {
    let t = site_tables(&separated());
    assert_eq!(t.unchecked, UNCHECKED_CRITERIA);
    assert!(UNCHECKED_CRITERIA
        .iter()
        .any(|c| c.contains("skip-share minimum")));
    assert!(UNCHECKED_CRITERIA
        .iter()
        .any(|c| c.contains("wrong-settle cap")));
}

#[test]
fn recall_and_its_bound_count_only_positives_jev_scored() {
    let mut items: Vec<Item> = (0..50)
        .map(|i| item(0.1, true, &format!("c{}", i % 5)))
        .collect();
    items.extend((0..9).map(|_| Item {
        safe: None,
        harmful: true,
        cluster: Some("c0".into()),
    }));
    items.extend((0..41).map(|i| item(0.9, false, &format!("c{}", i % 5))));
    let t = site_tables(&items);
    assert_eq!(t.threshold, None);
    assert_eq!(t.harmful, 59);
    assert_eq!(t.unscored_harmful, 9);
    assert_eq!(t.evidence, 91);
    let lower = t.recall_lower.unwrap();
    assert!(lower < TARGET_RECALL, "{lower}");
    assert!(t
        .dry_run
        .reasons
        .iter()
        .any(|r| r.contains("50 harmful positives")));
}

#[test]
fn recall_uses_the_same_clustered_items_as_its_bootstrap() {
    let mut items: Vec<Item> = (0..93)
        .map(|i| item(0.1, true, &format!("c{}", i % 7)))
        .collect();
    items.push(Item {
        safe: Some(0.95),
        harmful: true,
        cluster: None,
    });
    items.extend((0..41).map(|i| item(0.95, false, &format!("c{}", i % 7))));
    let t = site_tables(&items);
    assert_eq!(t.threshold, Some(0.95));
    assert_eq!(t.evidence, 134);
    assert_eq!(t.skip_recall, Some(1.0));
    let (lo, hi) = t.recall_bootstrap.unwrap();
    assert!(lo <= 1.0 && hi >= 1.0);
}

#[test]
fn abandoned_rows_are_recognized_by_the_shadow_error_text() {
    assert!(is_abandoned(Some(crate::jev::shadow::ABANDONED_ERROR)));
    assert!(!is_abandoned(Some("no_time_left")));
    assert!(!is_abandoned(None));
}

#[test]
fn too_few_positives_choose_no_threshold_and_skip_nothing() {
    let items = vec![
        item(0.1, true, "a"),
        item(0.9, false, "a"),
        item(0.8, false, "b"),
    ];
    let t = site_tables(&items);
    assert_eq!(t.harmful, 1);
    assert_eq!(t.threshold, None);
    assert_eq!(t.confusion.skips(), 0);
    assert_eq!(t.clusters, 2);
    assert_eq!(t.always_skip.skips(), 3);
    assert_eq!(t.never_skip.skips(), 0);
    assert!(!t.dry_run.promote);
    assert!(t
        .dry_run
        .reasons
        .iter()
        .any(|r| r.contains("at least 59 needed")));
}

#[test]
fn enough_separated_positives_pick_the_threshold_and_pass_the_dry_run() {
    let t = site_tables(&separated());
    assert_eq!(t.threshold, Some(0.9));
    assert_eq!(t.confusion.skips(), 41);
    assert_eq!(t.skip_recall, Some(1.0));
    assert!(t.recall_lower.unwrap() >= TARGET_RECALL);
    assert_eq!(t.harmful_among_skipped, Some(0.0));
    assert!((t.skip_share.unwrap() - 0.41).abs() < 1e-12);
    assert_eq!(t.clusters, 7);
    assert!(t.recall_bootstrap.is_some());
    assert!(t.dry_run.promote, "{:?}", t.dry_run.reasons);
}

// 066 T018a: only rows asked with the version-2 turn-review questions count
// toward promotion; version-1 rows, and rows that carry no version, are left
// out under their own reason.
fn versioned_questions(version: Option<u64>) -> Value {
    let entry = |id: &str, answer: f64| {
        let mut q = json!({ "id": id, "answer": answer });
        if let Some(v) = version {
            q["version"] = json!(v);
        }
        q
    };
    Value::Array(vec![
        entry("contradiction", 0.2),
        entry("capture_worthy", 0.1),
    ])
}

#[test]
fn a_version_1_turn_row_is_left_out_as_an_old_question_version() {
    let l = label(
        raw(false, false, CaptureKind::None),
        outcome(false, false, false),
    );
    assert_eq!(
        turn_item(Some(&l), &versioned_questions(Some(1)), Some("s")).unwrap_err(),
        Skip::OldQuestionVersion
    );
}

#[test]
fn a_turn_row_without_question_versions_is_left_out_too() {
    let l = label(
        raw(false, false, CaptureKind::None),
        outcome(false, false, false),
    );
    assert_eq!(
        turn_item(Some(&l), &versioned_questions(None), Some("s")).unwrap_err(),
        Skip::OldQuestionVersion
    );
}

#[test]
fn a_version_2_turn_row_counts() {
    let l = label(
        raw(false, false, CaptureKind::None),
        outcome(false, false, false),
    );
    let item = turn_item(Some(&l), &versioned_questions(Some(2)), Some("s")).unwrap();
    assert!(!item.harmful);
    assert!((item.safe.unwrap() - 0.8).abs() < 1e-12);
    assert_eq!(TURN_REVIEW_VERSION, 2);
}
