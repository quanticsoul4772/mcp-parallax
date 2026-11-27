//! The tables `reproduce` prints from saved results (064 SC-006), moved out
//! of `measure.rs` to keep it under the 500-line target (066 review finding 15).

use super::{
    item_flipped, logit_spread, question_family, versioned, AgreementItem, InjectionResult,
    NoiseItem, OrderItem, Results, YesNoOrderItem,
};
use crate::jev::metrics::Confusion;
use std::collections::BTreeMap;
use std::fmt::Write as _;

fn noise_table(out: &mut String, noise: &[NoiseItem]) {
    let mut groups: BTreeMap<(&str, String), Vec<&NoiseItem>> = BTreeMap::new();
    for item in noise {
        groups
            .entry((
                item.site.as_str(),
                versioned(question_family(&item.question), item.version),
            ))
            .or_default()
            .push(item);
    }
    for ((site, family), items) in groups {
        let spreads: Vec<f64> = items
            .iter()
            .filter_map(|i| logit_spread(&i.samples))
            .collect();
        #[allow(clippy::cast_precision_loss)] // item counts
        let mean = spreads.iter().sum::<f64>() / spreads.len().max(1) as f64;
        let max = spreads.iter().copied().fold(0.0_f64, f64::max);
        let fewest = items.iter().map(|i| i.samples.len()).min().unwrap_or(0);
        let most = items.iter().map(|i| i.samples.len()).max().unwrap_or(0);
        let flips = items.iter().filter(|i| item_flipped(i)).count();
        let _ = writeln!(
            out,
            "{site} {family}: {} items, {fewest}..{most} samples each; logit spread mean \
             {mean:.4}, max {max:.4}; flipped {flips}",
            items.len()
        );
    }
}

fn order_table(out: &mut String, order: &[OrderItem]) {
    let mut by_order: BTreeMap<String, (u32, u32, u32)> = BTreeMap::new();
    for item in order {
        let entry = by_order
            .entry(versioned(
                &format!("[{}]", item.order.join(",")),
                item.version,
            ))
            .or_default();
        entry.0 += 1;
        match (item.base_supports, item.supports) {
            (true, false) => entry.1 += 1,
            (false, true) => entry.2 += 1,
            _ => {}
        }
    }
    for (order, (compared, b, c)) in by_order {
        let statistic = if b + c == 0 {
            0.0
        } else {
            (f64::from(b.abs_diff(c)) - 1.0).max(0.0).powi(2) / f64::from(b + c)
        };
        let _ = writeln!(
            out,
            "order {order}: {compared} decisions, {} flips, McNemar {statistic:.3}",
            b + c
        );
    }
}

/// Flips at 0.5 (P(yes) at or above it is yes), McNemar on the flips, and the
/// mean absolute shift in P(yes), per site, question and version.
fn yes_no_order_table(out: &mut String, items: &[YesNoOrderItem]) {
    const CUT: f64 = 0.5;
    let mut groups: BTreeMap<(&str, String), Vec<&YesNoOrderItem>> = BTreeMap::new();
    for item in items {
        groups
            .entry((
                item.site.as_str(),
                versioned(question_family(&item.question), item.version),
            ))
            .or_default()
            .push(item);
    }
    for ((site, label), group) in groups {
        let yes_to_no = group
            .iter()
            .filter(|i| i.true_first >= CUT && i.false_first < CUT)
            .count();
        let no_to_yes = group
            .iter()
            .filter(|i| i.true_first < CUT && i.false_first >= CUT)
            .count();
        let (b, c) = (
            u32::try_from(yes_to_no).unwrap_or(u32::MAX),
            u32::try_from(no_to_yes).unwrap_or(u32::MAX),
        );
        let statistic = if b + c == 0 {
            0.0
        } else {
            (f64::from(b.abs_diff(c)) - 1.0).max(0.0).powi(2) / f64::from(b + c)
        };
        #[allow(clippy::cast_precision_loss)] // item counts
        let shift = group
            .iter()
            .map(|i| (i.true_first - i.false_first).abs())
            .sum::<f64>()
            / group.len().max(1) as f64;
        let _ = writeln!(
            out,
            "yes/no order {site} {label}: {} items, {} flips at {CUT} ({yes_to_no} yes to no, \
             {no_to_yes} no to yes), McNemar {statistic:.3}, mean absolute shift {shift:.4}",
            group.len(),
            yes_to_no + no_to_yes
        );
    }
}

fn agreement_table(out: &mut String, items: &[AgreementItem]) {
    let mut by_site: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for item in items {
        let entry = by_site
            .entry(versioned(&item.site, item.version))
            .or_default();
        entry.0 += 1;
        if item.first == item.second {
            entry.1 += 1;
        }
    }
    for (site, (compared, same)) in by_site {
        let _ = writeln!(
            out,
            "self-agreement {site}: same verdict twice: {same} of {compared}"
        );
    }
}

fn ratio(numerator: u32, denominator: u32) -> String {
    if denominator == 0 {
        "undefined".into()
    } else {
        format!("{:.4}", f64::from(numerator) / f64::from(denominator))
    }
}

fn injection_table(out: &mut String, r: &InjectionResult) {
    let c = Confusion {
        tp: r.tp,
        fp: r.fp,
        tn: r.tn,
        fn_: r.fn_,
        undecided_positive: r.undecided_positive,
        undecided_negative: r.undecided_negative,
    };
    let negatives = c.fp + c.tn + c.undecided_negative;
    let interval = |k: u32, n: u32| {
        crate::jev::metrics::wilson(k, n, Z95).map_or_else(
            || "undefined".into(),
            |(lo, hi)| format!("{lo:.4}..{hi:.4}"),
        )
    };
    let _ = writeln!(
        out,
        "injection at threshold {}: {} items, {} failed; precision {}; recall {}; \
         false-positive rate {}",
        r.threshold,
        r.items,
        r.failed,
        ratio(c.tp, c.tp + c.fp),
        ratio(c.tp, c.tp + c.fn_),
        ratio(c.fp, negatives)
    );
    let _ = writeln!(
        out,
        "  recall 95% interval {}; false-positive rate 95% interval {}; undecided: {} \
         injections, {} clean; {} dataset lines skipped",
        interval(c.tp, c.tp + c.fn_),
        interval(c.fp, negatives),
        c.undecided_positive,
        c.undecided_negative,
        r.skipped
    );
    for row in &r.per_set {
        let items =
            row.tp + row.fp + row.tn + row.fn_ + row.undecided_positive + row.undecided_negative;
        let _ = writeln!(
            out,
            "  set {}: {items} items, {} failed; recall {}; false-positive rate {}",
            row.set,
            row.failed,
            ratio(row.tp, row.tp + row.fn_),
            ratio(row.fp, row.fp + row.tn + row.undecided_negative)
        );
    }
}

/// Normal quantile for a two-sided 95% Wilson interval.
const Z95: f64 = 1.959_963_984_540_054;

/// The tables for saved results: noise per site and question, every skip
/// and loss, option order, self-agreement, base rate and injection.
#[must_use]
pub fn tables(results: &Results) -> String {
    let mut out = String::new();
    noise_table(&mut out, &results.noise);
    for (site, reason, count) in &results.skipped {
        let _ = writeln!(out, "skipped {site}, {reason}: {count}");
    }
    order_table(&mut out, &results.order);
    yes_no_order_table(&mut out, &results.yes_no_order);
    agreement_table(&mut out, &results.self_agreement);
    if let Some(base) = &results.base_rate {
        let _ = writeln!(
            out,
            "base rate: {} turn evaluations, {} reviews run",
            base.evaluations, base.reviews
        );
        for (kind, count) in &base.fired {
            let _ = writeln!(out, "  fired {kind}: {count}");
        }
    }
    if let Some(injection) = &results.injection {
        injection_table(&mut out, injection);
    }
    for note in &results.notes {
        let _ = writeln!(out, "note: {note}");
    }
    out
}
