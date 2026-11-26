//! The end-of-turn review skip (066 FR-009): pinned tables and a pure
//! decision. Off in 066: both tables are empty, so nothing can skip, and
//! `PARALLAX_JEV_MODE_TURN_REVIEW=on` is refused at startup.

use crate::checkpoint::fnv1a64;

/// Share of skippable turns that get the review anyway (promotion rule 8;
/// 066 clarification: 10%, the top of the 5-10% range).
pub const AUDIT_PERCENT: u64 = 10;

/// The no-band thresholds for the two turn-review questions, pinned to the
/// Jev model, the big model and the question version (promotion rule 7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThresholdEntry {
    /// The Jev model the thresholds were measured on.
    pub jev_model: &'static str,
    /// The big model whose review labeled the data.
    pub big_model: &'static str,
    /// The turn-review question version asked.
    pub question_version: u32,
    /// `contradiction` below this is "no".
    pub contradiction_no: f64,
    /// `capture_worthy` below this is "no".
    pub capture_worthy_no: f64,
}

/// Pinned thresholds; empty until the promotion feature fills them.
pub const TURN_REVIEW_THRESHOLDS: &[ThresholdEntry] = &[];

/// A recorded order check (FR-010), keyed the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderCheckEntry {
    /// The Jev model checked.
    pub jev_model: &'static str,
    /// The big model at the time.
    pub big_model: &'static str,
    /// The question version checked.
    pub question_version: u32,
}

/// Recorded order checks; empty until one is run and recorded.
pub const TURN_REVIEW_ORDER_CHECKED: &[OrderCheckEntry] = &[];

/// The entry for exactly this Jev model, big model and question version.
#[must_use]
pub fn thresholds_for(
    table: &[ThresholdEntry],
    jev_model: &str,
    big_model: &str,
    question_version: u32,
) -> Option<ThresholdEntry> {
    table
        .iter()
        .find(|e| {
            e.jev_model == jev_model
                && e.big_model == big_model
                && e.question_version == question_version
        })
        .copied()
}

/// Whether an order check is recorded for exactly this key.
#[must_use]
pub fn order_checked(
    table: &[OrderCheckEntry],
    jev_model: &str,
    big_model: &str,
    question_version: u32,
) -> bool {
    table.iter().any(|e| {
        e.jev_model == jev_model
            && e.big_model == big_model
            && e.question_version == question_version
    })
}

/// Whether the turn is in the audit share (research D7).
#[must_use]
pub fn in_audit_share(eval_id: &str) -> bool {
    fnv1a64(eval_id.as_bytes()) % 100 < AUDIT_PERCENT
}

/// Everything the skip decision reads.
#[derive(Debug, Clone)]
pub struct SkipInput<'a> {
    /// The pinned entry for this run's key; `None` when there is none.
    pub thresholds: Option<ThresholdEntry>,
    /// Memory is configured (the skip applies only then).
    pub memory_configured: bool,
    /// Jev's P(yes) for `contradiction`.
    pub contradiction: Option<f64>,
    /// Jev's P(yes) for `capture_worthy`.
    pub capture_worthy: Option<f64>,
    /// Memory, preference and transcript candidates found.
    pub candidates: usize,
    /// The state sent to Jev was truncated.
    pub truncated: bool,
    /// The evaluation id (decides the audit share).
    pub eval_id: &'a str,
}

/// What to do with the review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipDecision {
    /// Run the review as today.
    Review,
    /// Skip it (`review_skipped_by = jev`).
    Skip,
    /// It would skip, but the turn is audited (`review_skipped_by = audit`).
    Audit,
}

/// FR-009: skip only when every condition holds; anything missing reviews.
#[must_use]
pub fn skip_decision(input: &SkipInput<'_>) -> SkipDecision {
    let Some(t) = input.thresholds else {
        return SkipDecision::Review;
    };
    let (Some(contradiction), Some(capture)) = (input.contradiction, input.capture_worthy) else {
        return SkipDecision::Review;
    };
    let confident_no = contradiction < t.contradiction_no && capture < t.capture_worthy_no;
    if !input.memory_configured || input.truncated || input.candidates > 0 || !confident_no {
        return SkipDecision::Review;
    }
    if in_audit_share(input.eval_id) {
        SkipDecision::Audit
    } else {
        SkipDecision::Skip
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const JEV: &str = "jev-1.14.0";
    const BIG: &str = "claude-opus-4-8";

    fn entry() -> ThresholdEntry {
        ThresholdEntry {
            jev_model: JEV,
            big_model: BIG,
            question_version: 2,
            contradiction_no: 0.1,
            capture_worthy_no: 0.2,
        }
    }

    /// An evaluation id outside the audit share, found by search.
    fn outside_audit() -> String {
        (0..10_000)
            .map(|n| format!("eval-{n}"))
            .find(|id| !in_audit_share(id))
            .unwrap()
    }

    fn inside_audit() -> String {
        (0..10_000)
            .map(|n| format!("eval-{n}"))
            .find(|id| in_audit_share(id))
            .unwrap()
    }

    fn skippable(eval_id: &str) -> SkipInput<'_> {
        SkipInput {
            thresholds: Some(entry()),
            memory_configured: true,
            contradiction: Some(0.05),
            capture_worthy: Some(0.1),
            candidates: 0,
            truncated: false,
            eval_id,
        }
    }

    #[test]
    fn both_pinned_tables_are_empty_in_066() {
        assert!(TURN_REVIEW_THRESHOLDS.is_empty());
        assert!(TURN_REVIEW_ORDER_CHECKED.is_empty());
        assert_eq!(thresholds_for(TURN_REVIEW_THRESHOLDS, JEV, BIG, 2), None);
        assert!(!order_checked(TURN_REVIEW_ORDER_CHECKED, JEV, BIG, 2));
    }

    #[test]
    fn a_threshold_entry_is_found_only_on_all_three_keys() {
        let table = [entry()];
        assert_eq!(thresholds_for(&table, JEV, BIG, 2), Some(entry()));
        assert_eq!(thresholds_for(&table, "jev-1.15.0", BIG, 2), None);
        assert_eq!(thresholds_for(&table, JEV, "claude-sonnet-5-5", 2), None);
        assert_eq!(thresholds_for(&table, JEV, BIG, 1), None);
    }

    #[test]
    fn an_order_check_entry_is_found_only_on_all_three_keys() {
        let table = [OrderCheckEntry {
            jev_model: JEV,
            big_model: BIG,
            question_version: 2,
        }];
        assert!(order_checked(&table, JEV, BIG, 2));
        assert!(!order_checked(&table, JEV, BIG, 1));
        assert!(!order_checked(&table, JEV, "other", 2));
    }

    #[test]
    fn the_audit_share_is_the_bottom_ten_percent_of_the_eval_id_hash() {
        for n in 0..200 {
            let id = format!("eval-{n}");
            assert_eq!(
                in_audit_share(&id),
                crate::checkpoint::fnv1a64(id.as_bytes()) % 100 < AUDIT_PERCENT
            );
        }
        let audited = (0..10_000)
            .filter(|n| in_audit_share(&format!("eval-{n}")))
            .count();
        assert!((800..=1200).contains(&audited), "{audited}");
    }

    #[test]
    fn every_condition_met_outside_the_audit_share_skips() {
        let id = outside_audit();
        assert_eq!(skip_decision(&skippable(&id)), SkipDecision::Skip);
    }

    #[test]
    fn a_turn_in_the_audit_share_that_would_skip_is_audited() {
        let id = inside_audit();
        assert_eq!(skip_decision(&skippable(&id)), SkipDecision::Audit);
    }

    #[test]
    fn each_failed_or_missing_condition_runs_the_review() {
        let id = outside_audit();
        let cases: [(&str, SkipInput<'_>); 8] = [
            (
                "no thresholds",
                SkipInput {
                    thresholds: None,
                    ..skippable(&id)
                },
            ),
            (
                "memory off",
                SkipInput {
                    memory_configured: false,
                    ..skippable(&id)
                },
            ),
            (
                "a candidate",
                SkipInput {
                    candidates: 1,
                    ..skippable(&id)
                },
            ),
            (
                "truncated",
                SkipInput {
                    truncated: true,
                    ..skippable(&id)
                },
            ),
            (
                "contradiction missing",
                SkipInput {
                    contradiction: None,
                    ..skippable(&id)
                },
            ),
            (
                "capture missing",
                SkipInput {
                    capture_worthy: None,
                    ..skippable(&id)
                },
            ),
            (
                "contradiction at its threshold",
                SkipInput {
                    contradiction: Some(0.1),
                    ..skippable(&id)
                },
            ),
            (
                "capture above its threshold",
                SkipInput {
                    capture_worthy: Some(0.5),
                    ..skippable(&id)
                },
            ),
        ];
        for (name, input) in cases {
            assert_eq!(skip_decision(&input), SkipDecision::Review, "{name}");
        }
    }
}
