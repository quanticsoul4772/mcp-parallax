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

pub mod signals;

/// Share of rows (by id) in the sealed confirm split.
pub const CONFIRM_PERCENT: u64 = 30;
/// The skip-gate recall target (plan §7.4).
pub const TARGET_RECALL: f64 = 0.95;
/// One-sided level for every bound.
pub const ALPHA: f64 = 0.05;
/// Harmful positives needed for 95% recall with no misses (plan §7.4).
pub const MIN_POSITIVES: u32 = 59;
/// The turn-review question version whose rows count toward promotion
/// (066 FR-009): version 2 declares criteria; version-1 rows are left out.
pub const TURN_REVIEW_VERSION: u64 = 2;
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
    /// Asked with turn-review questions older than [`TURN_REVIEW_VERSION`],
    /// or with no version recorded (066).
    OldQuestionVersion,
}

/// Whether a row's error class marks a Jev call abandoned because verify
/// finished first: in shadow, verify was faster, not a Jev failure.
#[must_use]
pub fn is_abandoned(error_class: Option<&str>) -> bool {
    error_class == Some(crate::jev::shadow::ABANDONED_ERROR)
}

/// Whether every turn-review question on the row is at [`TURN_REVIEW_VERSION`]
/// and the contradiction question is there to say so.
fn at_turn_review_version(questions: &Value) -> bool {
    let Some(entries) = questions.as_array() else {
        return false;
    };
    let turn = |q: &&Value| q["id"] == "contradiction" || q["id"] == "capture_worthy";
    entries.iter().any(|q| q["id"] == "contradiction")
        && entries
            .iter()
            .filter(turn)
            .all(|q| q["version"].as_u64() == Some(TURN_REVIEW_VERSION))
}

fn noul(questions: &Value, id: &str) -> Option<f64> {
    questions.as_array()?.iter().find(|q| q["id"] == id)?["answer"].as_f64()
}

/// A turn-review row as a skip-gate item.
///
/// Harmful (plan §7.1) when the review raised a contradiction or violation
/// flag, or stored a capture, as the label's outcome fields record; P(safe)
/// is one minus the larger of the contradiction and capture answers.
///
/// # Errors
///
/// [`Skip::OldQuestionVersion`] when the questions are not at
/// [`TURN_REVIEW_VERSION`]. [`Skip::Unlabeled`] without a label that settles harm: no label, a label
/// written before the outcome fields existed, or any turn with memory-derived
/// candidates (no contradiction outcome, spec D11), flagged or not. A labeled
/// turn without the contradiction answer is kept, unscored.
pub fn turn_item(
    label: Option<&Value>,
    questions: &Value,
    cluster: Option<&str>,
) -> Result<Item, Skip> {
    if !at_turn_review_version(questions) {
        return Err(Skip::OldQuestionVersion);
    }
    let label = label.ok_or(Skip::Unlabeled)?;
    let violation = label["violation_flagged"]
        .as_bool()
        .ok_or(Skip::Unlabeled)?;
    let captured = match &label["captured"] {
        Value::Null => false,
        Value::Bool(captured) => *captured,
        _ => return Err(Skip::Unlabeled),
    };
    // A memory-pair turn has no contradiction outcome (D11); leaving all of
    // them out, flagged or not, keeps the sample from depending on outcome.
    let contradiction = label["contradiction_flagged"]
        .as_bool()
        .ok_or(Skip::Unlabeled)?;
    let harmful = contradiction || violation || captured;
    let capture = noul(questions, "capture_worthy").unwrap_or(0.0);
    Ok(Item {
        safe: noul(questions, "contradiction").map(|c| 1.0 - c.max(capture)),
        harmful,
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

pub mod injection;

#[cfg(test)]
mod tests;
