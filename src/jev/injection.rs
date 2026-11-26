//! Inputs of the FR-011(e) injection back-test, ported from an earlier
//! back-test: the dataset lines.

use crate::jev::metrics::{Confusion, Decision};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The set name of a dataset line that carries none.
pub const NO_SET: &str = "(no set)";

/// One dataset item: a page, and whether it carries an injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectionItem {
    /// The page's address, sent as `page_url`.
    pub url: String,
    /// The page text, sent as `page_text`.
    pub text: String,
    /// True for an injection.
    pub label: bool,
    /// The line's `set`, or [`NO_SET`].
    pub set: String,
}

/// One set's outcomes in an injection run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectionSetRow {
    /// The set name.
    pub set: String,
    /// Calls that failed or could not be redacted.
    pub failed: u32,
    /// Flagged injections.
    pub tp: u32,
    /// Flagged clean items.
    pub fp: u32,
    /// Unflagged clean items.
    pub tn: u32,
    /// Unflagged injections.
    pub fn_: u32,
    /// Injections with no usable answer.
    pub undecided_positive: u32,
    /// Clean items with no usable answer.
    pub undecided_negative: u32,
}

/// One set's tally while counting: failed calls and answered pairs.
#[derive(Default)]
struct SetTally {
    failed: u32,
    pairs: Vec<(Decision, bool)>,
}

/// Tally each item's outcome by its set, in set-name order. `None` is a
/// failed call.
#[must_use]
pub fn tally_sets(outcomes: &[(String, Option<Decision>, bool)]) -> Vec<InjectionSetRow> {
    let mut by_set: BTreeMap<&str, SetTally> = BTreeMap::new();
    for (set, decision, label) in outcomes {
        let entry = by_set.entry(set.as_str()).or_default();
        match decision {
            Some(d) => entry.pairs.push((*d, *label)),
            None => entry.failed += 1,
        }
    }
    by_set
        .into_iter()
        .map(|(set, tally)| {
            let c = Confusion::from_pairs(&tally.pairs);
            InjectionSetRow {
                set: set.to_string(),
                failed: tally.failed,
                tp: c.tp,
                fp: c.fp,
                tn: c.tn,
                fn_: c.fn_,
                undecided_positive: c.undecided_positive,
                undecided_negative: c.undecided_negative,
            }
        })
        .collect()
}

/// Parse one dataset line, `{"text": ..., "label": true|false, "url": ...}`;
/// `None` when it is not JSON or lacks one of the three.
#[must_use]
pub fn injection_item(line: &str) -> Option<InjectionItem> {
    let item: Value = serde_json::from_str(line).ok()?;
    Some(InjectionItem {
        url: item["url"].as_str()?.to_string(),
        text: item["text"].as_str()?.to_string(),
        label: item["label"].as_bool()?,
        set: item["set"].as_str().unwrap_or(NO_SET).to_string(),
    })
}

/// Every dataset line, parsed before any call is paid for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dataset {
    /// The usable lines.
    pub items: Vec<InjectionItem>,
    /// Lines that were not JSON or lacked text, label or url.
    pub skipped: u32,
}

/// Parse every line of a dataset.
///
/// # Errors
///
/// When no line is usable, naming how many were skipped, so a run with
/// nothing to measure stops before it calls or saves anything.
pub fn parse_dataset<'a>(lines: impl IntoIterator<Item = &'a str>) -> Result<Dataset, String> {
    let mut items = Vec::new();
    let mut skipped = 0_u32;
    for line in lines {
        match injection_item(line) {
            Some(item) => items.push(item),
            None => skipped += 1,
        }
    }
    if items.is_empty() {
        return Err(format!(
            "no usable dataset line ({skipped} lines skipped: each needs text, label and url)"
        ));
    }
    Ok(Dataset { items, skipped })
}

/// Check that at least one call produced an answer before the result is
/// saved.
///
/// # Errors
///
/// When none did (a wrong key turns the client off after its first 401, so
/// every call fails), naming how many failed, so the saved result is kept.
pub fn check_answered(answered: usize, failed: u32) -> Result<(), String> {
    if answered == 0 {
        return Err(format!(
            "no call produced an answer ({failed} failed); the saved result is kept"
        ));
    }
    Ok(())
}

/// Paired decisions on the same items under the two criteria orders (065
/// T028a). Only pairs decided in both orders are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OrderFlips {
    /// Pairs decided (yes or no) in both orders.
    pub compared: u32,
    /// Yes in the first order, no in the second.
    pub yes_to_no: u32,
    /// No in the first order, yes in the second.
    pub no_to_yes: u32,
}

impl OrderFlips {
    /// Pairs whose decision changed with the order.
    #[must_use]
    pub const fn flips(&self) -> u32 {
        self.yes_to_no + self.no_to_yes
    }

    /// Flips over compared pairs; `None` when nothing was compared.
    #[must_use]
    pub fn flip_rate(&self) -> Option<f64> {
        (self.compared > 0).then(|| f64::from(self.flips()) / f64::from(self.compared))
    }

    /// McNemar's statistic with continuity correction, as the research
    /// order table computes it; 0 when nothing flipped.
    #[must_use]
    pub fn mcnemar(&self) -> f64 {
        let (b, c) = (self.yes_to_no, self.no_to_yes);
        if b + c == 0 {
            0.0
        } else {
            (f64::from(b.abs_diff(c)) - 1.0).max(0.0).powi(2) / f64::from(b + c)
        }
    }
}

/// Count flips over `(first order, second order)` decision pairs.
#[must_use]
pub fn order_flips(pairs: &[(Decision, Decision)]) -> OrderFlips {
    let mut f = OrderFlips::default();
    for pair in pairs {
        match pair {
            (Decision::Yes, Decision::Yes) | (Decision::No, Decision::No) => f.compared += 1,
            (Decision::Yes, Decision::No) => {
                f.compared += 1;
                f.yes_to_no += 1;
            }
            (Decision::No, Decision::Yes) => {
                f.compared += 1;
                f.no_to_yes += 1;
            }
            _ => {}
        }
    }
    f
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::jev::metrics::Decision;

    #[test]
    fn a_full_line_parses() {
        let item =
            injection_item(r#"{"text": "page body", "label": true, "url": "https://a.example/x"}"#)
                .unwrap();
        assert_eq!(
            item,
            InjectionItem {
                url: "https://a.example/x".into(),
                text: "page body".into(),
                label: true,
                set: NO_SET.into(),
            }
        );
    }

    #[test]
    fn a_line_keeps_its_set() {
        let item = injection_item(
            r#"{"text": "t", "label": false, "url": "https://a.example", "set": "bipia"}"#,
        )
        .unwrap();
        assert_eq!(item.set, "bipia");
    }

    #[test]
    fn outcomes_are_tallied_per_set_in_name_order() {
        let rows = tally_sets(&[
            ("clean".into(), Some(Decision::Yes), false),
            ("bipia".into(), Some(Decision::Yes), true),
            ("bipia".into(), Some(Decision::No), true),
            ("bipia".into(), None, true),
            ("bipia".into(), Some(Decision::Undecided), true),
        ]);
        assert_eq!(
            rows,
            vec![
                InjectionSetRow {
                    set: "bipia".into(),
                    failed: 1,
                    tp: 1,
                    fp: 0,
                    tn: 0,
                    fn_: 1,
                    undecided_positive: 1,
                    undecided_negative: 0,
                },
                InjectionSetRow {
                    set: "clean".into(),
                    failed: 0,
                    tp: 0,
                    fp: 1,
                    tn: 0,
                    fn_: 0,
                    undecided_positive: 0,
                    undecided_negative: 0,
                },
            ]
        );
    }

    #[test]
    fn per_set_rows_add_up_to_the_pooled_counts_and_failures() {
        let outcomes: Vec<(String, Option<Decision>, bool)> = [
            ("a", Some(Decision::Yes), true),
            ("a", Some(Decision::No), false),
            ("b", None, true),
            ("b", Some(Decision::Undecided), false),
            ("c", Some(Decision::No), true),
            ("a", None, false),
            ("c", Some(Decision::Yes), false),
            ("b", Some(Decision::Undecided), true),
        ]
        .into_iter()
        .map(|(s, d, l)| (s.to_string(), d, l))
        .collect();
        let pairs: Vec<(Decision, bool)> = outcomes
            .iter()
            .filter_map(|(_, d, l)| d.map(|d| (d, *l)))
            .collect();
        let pooled = Confusion::from_pairs(&pairs);
        let failed = outcomes.iter().filter(|(_, d, _)| d.is_none()).count();
        let rows = tally_sets(&outcomes);
        let sum = |f: fn(&InjectionSetRow) -> u32| rows.iter().map(f).sum::<u32>();
        assert_eq!(sum(|r| r.failed), u32::try_from(failed).unwrap());
        assert_eq!(sum(|r| r.tp), pooled.tp);
        assert_eq!(sum(|r| r.fp), pooled.fp);
        assert_eq!(sum(|r| r.tn), pooled.tn);
        assert_eq!(sum(|r| r.fn_), pooled.fn_);
        assert_eq!(sum(|r| r.undecided_positive), pooled.undecided_positive);
        assert_eq!(sum(|r| r.undecided_negative), pooled.undecided_negative);
    }

    #[test]
    fn a_line_missing_a_field_or_not_json_is_none() {
        for line in [
            r#"{"label": false, "url": "https://a.example"}"#,
            r#"{"text": "t", "url": "https://a.example"}"#,
            r#"{"text": "t", "label": false}"#,
            r#"{"text": "t", "label": "no", "url": "https://a.example"}"#,
            "not json",
            "",
        ] {
            assert!(injection_item(line).is_none(), "parsed: {line}");
        }
    }

    #[test]
    fn a_dataset_keeps_usable_lines_and_counts_the_rest() {
        let dataset = parse_dataset([
            r#"{"text": "t", "label": false, "url": "https://a.example"}"#,
            "not json",
            r#"{"text": "t", "label": true}"#,
        ])
        .unwrap();
        assert_eq!(dataset.items.len(), 1);
        assert_eq!(dataset.skipped, 2);
    }

    #[test]
    fn a_dataset_with_no_usable_line_is_an_error_naming_the_skips() {
        let err = parse_dataset([r#"{"text": "t", "label": true}"#, "", "x"]).unwrap_err();
        assert!(err.contains("3 lines skipped"), "{err}");
        assert!(parse_dataset(std::iter::empty::<&str>()).is_err());
    }

    #[test]
    fn a_run_with_no_answered_call_is_an_error_naming_the_failures() {
        let err = check_answered(0, 3).unwrap_err();
        assert!(err.contains("3 failed"), "{err}");
        assert!(check_answered(1, 3).is_ok());
    }

    #[test]
    fn matching_decisions_in_both_orders_are_no_flips() {
        let f = order_flips(&[(Decision::Yes, Decision::Yes), (Decision::No, Decision::No)]);
        assert_eq!((f.compared, f.yes_to_no, f.no_to_yes), (2, 0, 0));
        assert_eq!(f.flips(), 0);
        assert!(f.mcnemar().abs() < f64::EPSILON);
        assert_eq!(f.flip_rate(), Some(0.0));
    }

    #[test]
    fn flips_are_counted_by_direction_with_a_continuity_corrected_mcnemar() {
        let mut pairs = vec![(Decision::Yes, Decision::No); 3];
        pairs.push((Decision::No, Decision::Yes));
        pairs.extend(vec![(Decision::No, Decision::No); 4]);
        let f = order_flips(&pairs);
        assert_eq!((f.compared, f.yes_to_no, f.no_to_yes), (8, 3, 1));
        assert_eq!(f.flips(), 4);
        // (|3 - 1| - 1)^2 / (3 + 1)
        assert!((f.mcnemar() - 0.25).abs() < 1e-12);
        assert_eq!(f.flip_rate(), Some(0.5));
    }

    #[test]
    fn a_pair_with_an_undecided_side_is_not_compared() {
        let f = order_flips(&[
            (Decision::Undecided, Decision::Yes),
            (Decision::No, Decision::Undecided),
            (Decision::Yes, Decision::Yes),
        ]);
        assert_eq!(f.compared, 1);
        assert_eq!(f.flips(), 0);
    }

    #[test]
    fn no_compared_pairs_give_no_flip_rate() {
        let f = order_flips(&[]);
        assert_eq!(f.compared, 0);
        assert_eq!(f.flip_rate(), None);
        assert!(f.mcnemar().abs() < f64::EPSILON);
    }
}
