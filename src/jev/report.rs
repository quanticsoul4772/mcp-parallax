//! The back-test report's §7 tables (064 FR-008, SC-005).
//!
//! Turns stored Jev rows into skip-gate items and computes, on the tune split
//! only, the plan §7 figures: positives by §7.1, the threshold chosen by the
//! rule stated in advance, recall and skip-share bounds, the harmful share
//! among skipped items, the always-skip and never-skip baselines, cluster
//! counts with a cluster bootstrap, and a promotion dry run with every failed
//! criterion. The confirm split stays sealed: nothing here reads it.

use crate::jev::metrics::{
    choose_skip_threshold, clopper_pearson, clopper_pearson_lower, cluster_bootstrap,
    promotion_decision, skip_recall, wilson, Confusion, Decision, Promotion, PromotionInput,
};
use serde_json::Value;
use std::collections::BTreeMap;

/// Share of rows (by id) in the sealed confirm split.
pub const CONFIRM_PERCENT: u64 = 30;
/// The skip-gate recall target (plan §7.4).
pub const TARGET_RECALL: f64 = 0.95;
/// One-sided level for every bound.
pub const ALPHA: f64 = 0.05;
/// Harmful positives needed for 95% recall with no misses (plan §7.4).
pub const MIN_POSITIVES: u32 = 59;
/// Normal quantile for a one-sided bound at [`ALPHA`].
const Z_ONE_SIDED: f64 = 1.644_853_626_951_472_2;
const BOOTSTRAP_ITERATIONS: usize = 2000;
const BOOTSTRAP_SEED: u64 = 64;

/// Plan §7.2 criteria the dry run does not check, because the plan sets no
/// number for them yet (065/066 decide).
pub const UNCHECKED_CRITERIA: [&str; 2] = [
    "skip-share minimum (plan 7.2): the plan sets no number yet",
    "wrong-settle cap (plan 7.2): the plan sets no number yet",
];

/// One tune-set item of a skip gate.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Jev's P(safe): skipping the big model is considered at or above the
    /// threshold. `None` when Jev gave no usable answer (excluded, failed,
    /// rejected): such an item always goes to the big model.
    pub safe: Option<f64>,
    /// Plan §7.1: the class whose miss causes harm.
    pub harmful: bool,
    /// The run or session the item came from; `None` when unknown, and then
    /// it counts toward no cluster.
    pub cluster: Option<String>,
}

/// Why a row is left out of the tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// No label, or a label that does not settle §7.1.
    Unlabeled,
}

/// Whether a row's error class marks a Jev call abandoned because verify
/// finished first: in shadow, verify was faster, not a Jev failure.
#[must_use]
pub fn is_abandoned(error_class: Option<&str>) -> bool {
    error_class == Some(crate::jev::shadow::ABANDONED_ERROR)
}

fn noul(questions: &Value, id: &str) -> Option<f64> {
    questions.as_array()?.iter().find(|q| q["id"] == id)?["answer"].as_f64()
}

/// A turn-review row as a skip-gate item. Harmful when the review flagged a
/// contradiction or a violation, or captured something; P(safe) is one minus
/// the larger of the contradiction and capture answers.
///
/// # Errors
///
/// [`Skip::Unlabeled`] without a label that settles harm: no label, an
/// unknown contradiction label (memory pairs were present, spec D11),
/// flagged or not, or an unknown capture kind. A labeled turn without the
/// contradiction answer is kept, unscored.
pub fn turn_item(
    label: Option<&Value>,
    questions: &Value,
    cluster: Option<&str>,
) -> Result<Item, Skip> {
    let label = label.ok_or(Skip::Unlabeled)?;
    let captured = match &label["capture_kind"] {
        Value::Null => false,
        Value::String(kind) if kind == "none" => false,
        Value::String(kind) if kind == "skill" || kind == "lesson" => true,
        _ => return Err(Skip::Unlabeled),
    };
    let violates = label["violates"].as_bool().ok_or(Skip::Unlabeled)?;
    let contradicts = label["contradicts"].as_bool().ok_or(Skip::Unlabeled)?;
    let capture = noul(questions, "capture_worthy").unwrap_or(0.0);
    Ok(Item {
        safe: noul(questions, "contradiction").map(|c| 1.0 - c.max(capture)),
        harmful: contradicts || violates || captured,
        cluster: cluster.map(str::to_string),
    })
}

/// A research-claim row as a skip-gate item. Harmful when the ensemble
/// refuted or contested the claim; P(safe) is the smallest `supports`
/// probability across the claim's sources.
///
/// # Errors
///
/// [`Skip::Unlabeled`] without a recognized support label. A labeled claim
/// without every source's `supports` probability is kept, unscored.
pub fn claim_item(
    label: Option<&Value>,
    questions: &Value,
    cluster: Option<&str>,
) -> Result<Item, Skip> {
    let harmful = match label.and_then(|l| l["support"].as_str()) {
        Some("refuted" | "contested") => true,
        Some("confirmed" | "unverified") => false,
        _ => return Err(Skip::Unlabeled),
    };
    let safe = questions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|q| {
            q["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("relation_"))
        })
        .map(|q| q["probabilities"]["supports"].as_f64())
        .collect::<Option<Vec<f64>>>()
        .and_then(|ps| ps.into_iter().reduce(f64::min));
    Ok(Item {
        safe,
        harmful,
        cluster: cluster.map(str::to_string),
    })
}

/// The plan §7 figures for one site's tune items.
#[derive(Debug, Clone)]
pub struct SiteTables {
    /// Items in the tables, scored or not.
    pub items: usize,
    /// Items without a usable Jev answer: never skipped.
    pub unscored: usize,
    /// Items without a cluster: in the tables, in no cluster.
    pub unclustered: usize,
    /// Scored, clustered items: the evidence for recall, its bound, the
    /// bootstrap and the dry run.
    pub evidence: usize,
    /// Harmful items (§7.1 positives), scored or not.
    pub harmful: u32,
    /// Harmful items Jev never scored: always sent to the big model, and no
    /// evidence of Jev's skill.
    pub unscored_harmful: u32,
    /// The skip threshold by the rule stated in advance; `None` when no
    /// threshold meets the target (then nothing is skipped).
    pub threshold: Option<f64>,
    /// Decisions at the threshold over every item: skip (`No`) or send to the
    /// big model (`Yes`).
    pub confusion: Confusion,
    /// Share of harmful evidence items not skipped.
    pub skip_recall: Option<f64>,
    /// One-sided Clopper-Pearson lower bound on that recall.
    pub recall_lower: Option<f64>,
    /// Share of items skipped.
    pub skip_share: Option<f64>,
    /// One-sided Wilson lower bound on the skip share, at [`ALPHA`].
    pub skip_share_lower: Option<f64>,
    /// Harmful share among skipped items (the wrong-settle rate for claims).
    pub harmful_among_skipped: Option<f64>,
    /// One-sided Clopper-Pearson upper bound on that share.
    pub harmful_among_skipped_upper: Option<f64>,
    /// Baseline: skip everything.
    pub always_skip: Confusion,
    /// Baseline: skip nothing.
    pub never_skip: Confusion,
    /// Distinct clusters among the evidence items.
    pub clusters: usize,
    /// Cluster-bootstrap 95% interval on skip recall.
    pub recall_bootstrap: Option<(f64, f64)>,
    /// `promotion_decision()` on the evidence items: a dry run, not a
    /// promotion.
    pub dry_run: Promotion,
    /// Plan §7.2 criteria the dry run does not check.
    pub unchecked: [&'static str; 2],
}

fn ratio(numerator: u32, denominator: u32) -> Option<f64> {
    (denominator > 0).then(|| f64::from(numerator) / f64::from(denominator))
}

/// Compute the §7 tables for one site's tune items.
#[must_use]
pub fn site_tables(items: &[Item]) -> SiteTables {
    let scored: Vec<(f64, bool)> = items
        .iter()
        .filter_map(|i| i.safe.map(|s| (s, i.harmful)))
        .collect();
    let threshold = choose_skip_threshold(&scored, TARGET_RECALL, ALPHA);
    let decide = |item: &Item| match (threshold, item.safe) {
        (Some(t), Some(safe)) if safe >= t => Decision::No,
        _ => Decision::Yes,
    };
    let pairs: Vec<(Decision, bool)> = items.iter().map(|i| (decide(i), i.harmful)).collect();
    let confusion = Confusion::from_pairs(&pairs);
    let labels: Vec<bool> = items.iter().map(|i| i.harmful).collect();
    // Recall, its bound, the bootstrap and the dry run rest on one set: the
    // scored, clustered items. Unscored positives say nothing of Jev's skill.
    let mut by_cluster: BTreeMap<&str, Vec<(Decision, bool)>> = BTreeMap::new();
    for (item, pair) in items.iter().zip(&pairs) {
        if let (Some(cluster), Some(_)) = (&item.cluster, item.safe) {
            by_cluster.entry(cluster.as_str()).or_default().push(*pair);
        }
    }
    let clusters: Vec<Vec<(Decision, bool)>> = by_cluster.into_values().collect();
    let evidence: Vec<(Decision, bool)> = clusters.iter().flatten().copied().collect();
    let evidence_confusion = Confusion::from_pairs(&evidence);
    let n = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let skips = confusion.skips();
    let evidence_positives = evidence_confusion.positives();
    SiteTables {
        items: items.len(),
        unscored: items.iter().filter(|i| i.safe.is_none()).count(),
        unclustered: items.iter().filter(|i| i.cluster.is_none()).count(),
        evidence: evidence.len(),
        harmful: confusion.positives(),
        unscored_harmful: u32::try_from(
            items
                .iter()
                .filter(|i| i.harmful && i.safe.is_none())
                .count(),
        )
        .unwrap_or(u32::MAX),
        threshold,
        confusion,
        skip_recall: skip_recall(&evidence_confusion),
        recall_lower: clopper_pearson_lower(
            evidence_positives - evidence_confusion.fn_,
            evidence_positives,
            ALPHA,
        ),
        skip_share: ratio(skips, n),
        skip_share_lower: wilson(skips, n, Z_ONE_SIDED).map(|(lower, _)| lower),
        harmful_among_skipped: ratio(confusion.fn_, skips),
        harmful_among_skipped_upper: clopper_pearson(confusion.fn_, skips, 2.0 * ALPHA)
            .map(|(_, upper)| upper),
        always_skip: Confusion::always(Decision::No, &labels),
        never_skip: Confusion::always(Decision::Yes, &labels),
        clusters: clusters.len(),
        recall_bootstrap: cluster_bootstrap(
            &clusters,
            skip_recall,
            BOOTSTRAP_ITERATIONS,
            BOOTSTRAP_SEED,
        ),
        dry_run: promotion_decision(&PromotionInput {
            confusion: evidence_confusion,
            clusters: clusters.len(),
            target_recall: TARGET_RECALL,
            alpha: ALPHA,
            min_positives: MIN_POSITIVES,
        }),
        unchecked: UNCHECKED_CRITERIA,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turn_questions(contradiction: f64, capture: Option<f64>) -> serde_json::Value {
        let mut qs = vec![json!({ "id": "contradiction", "answer": contradiction })];
        if let Some(p) = capture {
            qs.push(json!({ "id": "capture_worthy", "answer": p }));
        }
        serde_json::Value::Array(qs)
    }

    fn label(
        contradicts: &serde_json::Value,
        violates: bool,
        capture: &serde_json::Value,
    ) -> serde_json::Value {
        json!({ "contradicts": contradicts, "violates": violates, "capture_kind": capture })
    }

    #[test]
    fn a_turn_is_harmful_when_the_review_flagged_or_captured() {
        let qs = turn_questions(0.2, Some(0.1));
        let harmful = |l: serde_json::Value| turn_item(Some(&l), &qs, Some("s")).unwrap().harmful;
        assert!(harmful(label(&json!(true), false, &json!("none"))));
        assert!(harmful(label(&json!(false), true, &json!("none"))));
        assert!(harmful(label(&json!(false), false, &json!("lesson"))));
        assert!(!harmful(label(&json!(false), false, &json!("none"))));
        assert!(!harmful(label(&json!(false), false, &json!(null))));
    }

    #[test]
    fn a_turn_whose_contradiction_label_is_unknown_is_unlabeled_flagged_or_not() {
        let qs = turn_questions(0.2, None);
        let l = label(&json!(null), false, &json!("none"));
        assert_eq!(turn_item(Some(&l), &qs, None).unwrap_err(), Skip::Unlabeled);
        let flagged = label(&json!(null), true, &json!("lesson"));
        assert_eq!(
            turn_item(Some(&flagged), &qs, None).unwrap_err(),
            Skip::Unlabeled
        );
        assert_eq!(turn_item(None, &qs, None).unwrap_err(), Skip::Unlabeled);
        let odd = label(&json!(false), false, &json!("habit"));
        assert_eq!(
            turn_item(Some(&odd), &qs, None).unwrap_err(),
            Skip::Unlabeled
        );
    }

    #[test]
    fn a_turns_safe_score_is_one_minus_the_larger_answer() {
        let l = label(&json!(false), false, &json!("none"));
        let item = turn_item(Some(&l), &turn_questions(0.2, Some(0.35)), Some("s")).unwrap();
        assert!((item.safe.unwrap() - 0.65).abs() < 1e-12);
        assert_eq!(item.cluster.as_deref(), Some("s"));
        let item = turn_item(Some(&l), &turn_questions(0.3, None), None).unwrap();
        assert!((item.safe.unwrap() - 0.7).abs() < 1e-12);
        assert_eq!(item.cluster, None);
    }

    #[test]
    fn a_labeled_turn_without_answers_is_kept_unscored() {
        let l = label(&json!(true), false, &json!("none"));
        let missing = json!([{ "id": "contradiction" }]);
        let item = turn_item(Some(&l), &missing, Some("s")).unwrap();
        assert_eq!(item.safe, None);
        assert!(item.harmful);
    }

    fn relation(id: &str, supports: f64) -> serde_json::Value {
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
        let mut items: Vec<Item> = (0..59)
            .map(|i| item(0.1, true, &format!("c{}", i % 7)))
            .collect();
        items.extend((0..41).map(|i| item(0.9, false, &format!("c{}", i % 7))));
        let t = site_tables(&items);
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
}
