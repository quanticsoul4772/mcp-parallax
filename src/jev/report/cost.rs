//! The turn signals' downstream-cost estimate (068 FR-008, plan D4).

use crate::checkpoint::COOLDOWN_WINDOW_MS;
use crate::storage::jev::JevRecord;
use crate::traits::classifier::JevSite;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

/// The yes cutoffs the estimate is reported at (plan D4 note): no single
/// yes is fixed before promotion.
pub const CUTOFFS: [f64; 3] = [0.5, 0.7, 0.9];

/// The turn-signal questions (068 plan D0).
pub const KINDS: [&str; 2] = ["pushback_reversal", "unverified_fact"];

/// One signal's estimate at one cutoff.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalCost {
    /// The question id.
    pub kind: String,
    /// The yes cutoff.
    pub cutoff: f64,
    /// Answers at or above the cutoff.
    pub yes: usize,
    /// Yeses the cooldown would have suppressed: the same session had a yes
    /// for the same kind, at this cutoff, within `COOLDOWN_WINDOW_MS` before.
    pub suppressed: usize,
    /// Yeses not suppressed: each would force one continuation.
    pub continuations: usize,
    /// Those yeses' correctives, each priced at the mean cost of its tool's
    /// invocation records.
    pub corrective_cost_usd: f64,
    /// Those yeses whose recommended tool has no invocation records.
    pub unpriced: usize,
}

/// One answered turn-signal row.
struct Answer<'a> {
    kind: &'a str,
    session: &'a str,
    at: DateTime<Utc>,
    p: f64,
    recommends: Option<&'a str>,
}

/// The estimate per signal and cutoff, from turn-signal rows and the
/// `(tool, cost_usd)` pairs of invocation records. Rows from other sites,
/// and rows with no answer or no session, count nowhere.
#[must_use]
pub fn signal_costs(rows: &[JevRecord], tool_costs: &[(&str, f64)]) -> Vec<SignalCost> {
    let mut answers: Vec<Answer<'_>> = rows
        .iter()
        .filter(|r| r.site == JevSite::TurnSignals)
        .filter_map(|r| {
            let entry = r.questions_json.get(0)?;
            Some(Answer {
                kind: entry.get("id")?.as_str()?,
                session: r.harness_session_id.as_deref()?,
                at: r.created_at,
                p: entry.get("answer")?.as_f64()?,
                recommends: entry.get("recommends").and_then(Value::as_str),
            })
        })
        .collect();
    answers.sort_by_key(|a| a.at);
    let window = Duration::milliseconds(COOLDOWN_WINDOW_MS);
    let mean = |tool: &str| -> Option<f64> {
        let costs: Vec<f64> = tool_costs
            .iter()
            .filter(|(t, _)| *t == tool)
            .map(|(_, c)| *c)
            .collect();
        let n = f64::from(u32::try_from(costs.len()).unwrap_or(u32::MAX));
        (!costs.is_empty()).then(|| costs.iter().sum::<f64>() / n)
    };
    let mut out = Vec::with_capacity(KINDS.len() * CUTOFFS.len());
    for kind in KINDS {
        for cutoff in CUTOFFS {
            let yeses: Vec<&Answer<'_>> = answers
                .iter()
                .filter(|a| a.kind == kind && a.p >= cutoff)
                .collect();
            let mut cost = SignalCost {
                kind: kind.to_string(),
                cutoff,
                yes: yeses.len(),
                suppressed: 0,
                continuations: 0,
                corrective_cost_usd: 0.0,
                unpriced: 0,
            };
            // As the real cooldown: only a delivered yes starts a window
            // (068 review finding 2).
            let mut delivered: Vec<&Answer<'_>> = Vec::new();
            for answer in &yeses {
                let suppressed = delivered.iter().any(|earlier| {
                    earlier.session == answer.session && answer.at - earlier.at <= window
                });
                if suppressed {
                    cost.suppressed += 1;
                    continue;
                }
                delivered.push(answer);
                cost.continuations += 1;
                if let Some(tool) = answer.recommends {
                    match mean(tool) {
                        Some(m) => cost.corrective_cost_usd += m,
                        None => cost.unpriced += 1,
                    }
                }
            }
            out.push(cost);
        }
    }
    out
}

/// The rows whose turn call wrote its checkpoint record.
///
/// `recorded` holds those records' ids. A call Claude Code cancelled leaves
/// rows with no record; it delivered nothing, so the estimate leaves them out
/// (068 review finding 1).
#[must_use]
pub fn delivered_rows<S: std::hash::BuildHasher>(
    rows: &[JevRecord],
    recorded: &std::collections::HashSet<&str, S>,
) -> Vec<JevRecord> {
    rows.iter()
        .filter(|r| r.eval_id.as_deref().is_some_and(|id| recorded.contains(id)))
        .cloned()
        .collect()
}

/// The report's lines for the estimate (plan D4).
#[must_use]
pub fn summary_lines(costs: &[SignalCost]) -> Vec<String> {
    let mut lines = vec![
        "turn-signal downstream cost: continuations are counted with no dollar figure; \
         Parallax records no cost for the harness model's turn"
            .to_string(),
    ];
    lines.extend(costs.iter().map(|c| {
        format!(
            "  {} at p >= {}: {} yes, {} would be suppressed, {} continuation(s), \
             correctives ${:.4} ({} unpriced)",
            c.kind,
            c.cutoff,
            c.yes,
            c.suppressed,
            c.continuations,
            c.corrective_cost_usd,
            c.unpriced
        )
    }));
    lines
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{signal_costs, SignalCost, CUTOFFS};
    use crate::storage::jev::JevRecord;
    use crate::traits::classifier::JevSite;
    use chrono::{Duration, TimeZone, Utc};

    fn row(session: &str, kind: &str, p: f64, minutes: i64, recommends: Option<&str>) -> JevRecord {
        let mut r = crate::jev::shadow::tests_support::claim_row();
        r.site = JevSite::TurnSignals;
        r.harness_session_id = Some(session.into());
        r.created_at =
            Utc.with_ymd_and_hms(2026, 11, 27, 12, 0, 0).unwrap() + Duration::minutes(minutes);
        r.questions_json = serde_json::json!([
            { "id": kind, "version": 1, "option_order": ["true", "false"],
              "answer": p, "probabilities": null, "band": null,
              "recommends": recommends }
        ]);
        r
    }

    fn find<'a>(costs: &'a [SignalCost], kind: &str, cutoff: f64) -> &'a SignalCost {
        costs
            .iter()
            .find(|c| c.kind == kind && (c.cutoff - cutoff).abs() < 1e-9)
            .unwrap()
    }

    // 068 T016 (FR-008, plan D4): per signal and cutoff, the yeses, the ones
    // the cooldown would have suppressed (recomputed per cutoff from the
    // rows: an earlier yes, same session and kind, within the window), the
    // continuations the rest would trigger, and their correctives priced at
    // the mean cost of that tool's invocation records; a yes whose tool has
    // no records is counted as unpriced.
    #[test]
    fn the_estimate_counts_yeses_suppression_continuations_and_corrective_cost() {
        let rows = vec![
            row("s1", "pushback_reversal", 0.95, 0, Some("verify")),
            row("s1", "pushback_reversal", 0.6, 10, Some("verify")),
            row("s1", "pushback_reversal", 0.8, 50, Some("verify")),
            row("s2", "pushback_reversal", 0.95, 5, Some("verify")),
            row("s1", "unverified_fact", 0.55, 0, None),
            row("s1", "unverified_fact", 0.95, 20, Some("research")),
        ];
        let tool_costs = [("verify", 0.02), ("verify", 0.04), ("check", 0.5)];
        let costs = signal_costs(&rows, &tool_costs);
        assert_eq!(CUTOFFS, [0.5, 0.7, 0.9]);
        assert_eq!(costs.len(), 6);

        // (kind, cutoff, yes, suppressed, continuations, cost, unpriced)
        let expected = [
            ("pushback_reversal", 0.5, 4, 1, 3, 0.09, 0),
            ("pushback_reversal", 0.7, 3, 0, 3, 0.09, 0),
            ("pushback_reversal", 0.9, 2, 0, 2, 0.06, 0),
            ("unverified_fact", 0.5, 2, 1, 1, 0.0, 0),
            ("unverified_fact", 0.7, 1, 0, 1, 0.0, 1),
            ("unverified_fact", 0.9, 1, 0, 1, 0.0, 1),
        ];
        for (kind, cutoff, yes, suppressed, continuations, cost, unpriced) in expected {
            let c = find(&costs, kind, cutoff);
            let case = format!("{kind} at {cutoff}");
            assert_eq!(c.yes, yes, "{case}");
            assert_eq!(c.suppressed, suppressed, "{case}");
            assert_eq!(c.continuations, continuations, "{case}");
            assert!(
                (c.corrective_cost_usd - cost).abs() < 1e-12,
                "{case}: {c:?}"
            );
            assert_eq!(c.unpriced, unpriced, "{case}");
        }
    }

    // 068 review finding 1 (timing fix): a turn call Claude Code cancelled
    // writes rows but no checkpoint record; those rows were never delivered,
    // so the estimate leaves them out.
    #[test]
    fn rows_without_a_checkpoint_record_are_left_out() {
        let mut recorded = row("s1", "pushback_reversal", 0.9, 0, Some("verify"));
        recorded.eval_id = Some("eval-recorded".into());
        let mut cancelled = row("s1", "pushback_reversal", 0.9, 5, Some("verify"));
        cancelled.eval_id = Some("eval-cancelled".into());
        let mut unlinked = row("s1", "pushback_reversal", 0.9, 10, Some("verify"));
        unlinked.eval_id = None;
        let ids = std::collections::HashSet::from(["eval-recorded"]);
        let kept = super::delivered_rows(&[recorded, cancelled, unlinked], &ids);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].eval_id.as_deref(), Some("eval-recorded"));
    }

    // 068 review finding 2: as the real cooldown, only a delivered yes starts
    // a window; yeses at 0, 20 and 40 minutes deliver at 0 and 40.
    #[test]
    fn a_suppressed_yes_does_not_restart_the_window() {
        let rows = vec![
            row("s1", "pushback_reversal", 0.9, 0, Some("verify")),
            row("s1", "pushback_reversal", 0.9, 20, Some("verify")),
            row("s1", "pushback_reversal", 0.9, 40, Some("verify")),
        ];
        let costs = signal_costs(&rows, &[("verify", 0.01)]);
        let c = find(&costs, "pushback_reversal", 0.5);
        assert_eq!((c.yes, c.suppressed, c.continuations), (3, 1, 2), "{c:?}");
    }

    // 068 T016: rows from other sites, and rows with no answer, count nowhere.
    #[test]
    fn other_sites_and_unanswered_rows_are_not_counted() {
        let mut other = row("s1", "pushback_reversal", 0.95, 0, Some("verify"));
        other.site = JevSite::TurnReview;
        let mut unanswered = row("s1", "unverified_fact", 0.95, 0, None);
        unanswered.questions_json[0]["answer"] = serde_json::Value::Null;
        let costs = signal_costs(&[other, unanswered], &[]);
        assert!(costs.iter().all(|c| c.yes == 0 && c.continuations == 0));
    }

    // 068 T016 (plan D4): the report's lines say continuations carry no
    // dollar figure, then give one line per signal and cutoff.
    #[test]
    fn summary_lines_note_the_continuation_cost_and_list_each_signal_and_cutoff() {
        let rows = vec![row("s1", "pushback_reversal", 0.95, 0, Some("verify"))];
        let lines = super::summary_lines(&signal_costs(&rows, &[("verify", 0.03)]));
        assert_eq!(lines.len(), 1 + 6);
        assert!(
            lines[0].contains("no cost for the harness model's turn"),
            "{}",
            lines[0]
        );
        assert!(
            lines.iter().any(|l| l.contains("pushback_reversal")
                && l.contains("0.5")
                && l.contains("1 yes")
                && l.contains("1 continuation")
                && l.contains("$0.0300")),
            "{lines:?}"
        );
    }
}
