//! The report's turn-signal section (066 T028): how often each signal fired,
//! the exclusion reasons, and the kinds of the unmatched entities.

pub use crate::storage::turn_signals::TurnSignalRecord;
use std::collections::BTreeMap;

/// Counts over every stored turn-signal row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnSignalSummary {
    /// Rows counted.
    pub rows: usize,
    /// `pushback_reversal` results: `fired`, `not_fired` or a reason.
    pub pushback: BTreeMap<String, usize>,
    /// `unverified_fact` results: `fired`, `not_fired` or a reason.
    pub unverified: BTreeMap<String, usize>,
    /// Unmatched entities by kind.
    pub unmatched_kinds: BTreeMap<String, usize>,
}

/// Count each result and each unmatched entity kind.
#[must_use]
pub fn turn_signal_summary(rows: &[TurnSignalRecord]) -> TurnSignalSummary {
    let mut s = TurnSignalSummary {
        rows: rows.len(),
        ..TurnSignalSummary::default()
    };
    for row in rows {
        *s.pushback.entry(row.pushback_result.clone()).or_default() += 1;
        *s.unverified
            .entry(row.unverified_result.clone())
            .or_default() += 1;
        for entity in row.unmatched_entities.as_array().into_iter().flatten() {
            if let Some(kind) = entity["kind"].as_str() {
                *s.unmatched_kinds.entry(kind.to_string()).or_default() += 1;
            }
        }
    }
    s
}

fn counts(map: &BTreeMap<String, usize>) -> String {
    map.iter()
        .map(|(key, n)| format!("{key} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The report's lines for the summary; only the header when there are no
/// rows.
#[must_use]
pub fn summary_lines(s: &TurnSignalSummary) -> Vec<String> {
    let mut lines = vec![format!("== turn_signals ({} rows)", s.rows)];
    if s.rows == 0 {
        return lines;
    }
    lines.push(format!("  pushback_reversal: {}", counts(&s.pushback)));
    lines.push(format!("  unverified_fact: {}", counts(&s.unverified)));
    if !s.unmatched_kinds.is_empty() {
        lines.push(format!(
            "  unmatched entities: {}",
            counts(&s.unmatched_kinds)
        ));
    }
    lines
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn row(pushback: &str, unverified: &str, kinds: &[&str]) -> TurnSignalRecord {
        TurnSignalRecord {
            id: "r".into(),
            created_at: Utc.with_ymd_and_hms(2026, 11, 26, 12, 0, 0).unwrap(),
            eval_id: "e".into(),
            harness_session_id: None,
            transcript_ref: None,
            turn_start_uuid: None,
            final_uuid: None,
            pushback_result: pushback.into(),
            unverified_result: unverified.into(),
            unmatched_entities: kinds
                .iter()
                .map(|k| json!({ "kind": k, "hash": "0" }))
                .collect(),
            recommendations: json!({}),
        }
    }

    #[test]
    fn the_summary_counts_each_result_and_each_unmatched_kind() {
        let rows = [
            row("fired", "fired", &["version", "path"]),
            row("no previous final message", "fired", &["version"]),
            row("not_fired", "not_fired", &[]),
            row("tool call after prompt", "turn start not found", &[]),
        ];
        let s = turn_signal_summary(&rows);
        assert_eq!(s.rows, 4);
        let count = |map: &BTreeMap<String, usize>, key: &str| map.get(key).copied().unwrap_or(0);
        assert_eq!(count(&s.pushback, "fired"), 1);
        assert_eq!(count(&s.pushback, "not_fired"), 1);
        assert_eq!(count(&s.pushback, "no previous final message"), 1);
        assert_eq!(count(&s.pushback, "tool call after prompt"), 1);
        assert_eq!(count(&s.unverified, "fired"), 2);
        assert_eq!(count(&s.unverified, "turn start not found"), 1);
        assert_eq!(count(&s.unmatched_kinds, "version"), 2);
        assert_eq!(count(&s.unmatched_kinds, "path"), 1);
    }

    #[test]
    fn the_printed_section_names_every_count() {
        let printed = summary_lines(&turn_signal_summary(&[row(
            "fired",
            "not_fired",
            &["number"],
        )]));
        assert_eq!(
            printed,
            [
                "== turn_signals (1 rows)",
                "  pushback_reversal: fired 1",
                "  unverified_fact: not_fired 1",
                "  unmatched entities: number 1",
            ]
        );
    }

    #[test]
    fn no_rows_print_only_the_header() {
        assert_eq!(
            summary_lines(&turn_signal_summary(&[])),
            ["== turn_signals (0 rows)"]
        );
    }
}
