//! Live-measurement helpers for the back-test harness (064 FR-011, SC-006).
//!
//! The spread and flip statistics for noise, the transcript cut that lets the
//! harness rebuild a stored turn (turn rows keep only a fingerprint of their
//! state), and the saved results file with the tables printed from it, so a
//! live run can be reproduced without calling anything.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Probabilities are clamped this far from 0 and 1 before the logit.
const LOGIT_EPS: f64 = 1e-6;

/// The logit of `p`, clamped away from 0 and 1 so it stays finite.
#[must_use]
pub fn logit(p: f64) -> f64 {
    let p = p.clamp(LOGIT_EPS, 1.0 - LOGIT_EPS);
    (p / (1.0 - p)).ln()
}

/// Population standard deviation of the samples on the logit scale (research
/// D13); `None` without samples.
#[must_use]
pub fn logit_spread(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)] // a handful of samples
    let n = samples.len() as f64;
    let logits: Vec<f64> = samples.iter().map(|p| logit(*p)).collect();
    let mean = logits.iter().sum::<f64>() / n;
    Some((logits.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / n).sqrt())
}

/// Whether the samples fall on both sides of `cut` (at or above is yes).
#[must_use]
pub fn flipped(samples: &[f64], cut: f64) -> bool {
    let yes = samples.iter().filter(|p| **p >= cut).count();
    yes != 0 && yes != samples.len()
}

fn timestamp(line: &str) -> Option<DateTime<Utc>> {
    let value: Value = serde_json::from_str(line).ok()?;
    value["timestamp"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// The transcript as it stood at `until`: lines are kept up to the first one
/// whose timestamp is later.
#[must_use]
pub fn transcript_until(text: &str, until: DateTime<Utc>) -> String {
    let kept: Vec<&str> = text
        .lines()
        .take_while(|line| timestamp(line).is_none_or(|t| t <= until))
        .collect();
    kept.join("\n")
}

/// The text of the last assistant line that carries text.
#[must_use]
pub fn last_assistant_text(text: &str) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let value: Value = serde_json::from_str(line).ok()?;
        if value["type"] != "assistant" {
            return None;
        }
        let parts: Vec<&str> = value["message"]["content"]
            .as_array()?
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    })
}

/// One item asked five times.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoiseItem {
    /// The site, as recorded.
    pub site: String,
    /// The stored row.
    pub row_id: String,
    /// The question id.
    pub question: String,
    /// The answers: P(yes), or P(supports) for a relation question.
    pub samples: Vec<f64>,
}

/// One relation decision under a non-default option order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderItem {
    /// The stored row.
    pub row_id: String,
    /// The question id.
    pub question: String,
    /// The option order asked.
    pub order: Vec<String>,
    /// `supports` was chosen under the fixed order.
    pub base_supports: bool,
    /// `supports` was chosen under this order.
    pub supports: bool,
}

/// One item judged twice by the big model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreementItem {
    /// The site, as recorded.
    pub site: String,
    /// The stored row.
    pub row_id: String,
    /// The first verdict.
    pub first: String,
    /// The second verdict.
    pub second: String,
}

/// Everything a live measurement run produced, saved so the tables can be
/// printed again without calls (SC-006).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Results {
    /// Noise items.
    pub noise: Vec<NoiseItem>,
    /// Option-order items.
    pub order: Vec<OrderItem>,
    /// Self-agreement items.
    pub self_agreement: Vec<AgreementItem>,
    /// Rows left out: (site, reason, count).
    pub skipped: Vec<(String, String, u32)>,
}

fn noise_table(out: &mut String, noise: &[NoiseItem]) {
    let mut by_site: BTreeMap<&str, Vec<&NoiseItem>> = BTreeMap::new();
    for item in noise {
        by_site.entry(item.site.as_str()).or_default().push(item);
    }
    for (site, items) in by_site {
        let spreads: Vec<f64> = items
            .iter()
            .filter_map(|i| logit_spread(&i.samples))
            .collect();
        #[allow(clippy::cast_precision_loss)] // item counts
        let mean = spreads.iter().sum::<f64>() / spreads.len().max(1) as f64;
        let max = spreads.iter().copied().fold(0.0_f64, f64::max);
        let flips = items.iter().filter(|i| flipped(&i.samples, 0.5)).count();
        let _ = writeln!(
            out,
            "{site}: {} items asked 5 times; logit spread mean {mean:.4}, max {max:.4}; \
             flipped {flips}",
            items.len()
        );
    }
}

fn order_table(out: &mut String, order: &[OrderItem]) {
    let mut by_order: BTreeMap<String, (u32, u32, u32)> = BTreeMap::new();
    for item in order {
        let entry = by_order.entry(item.order.join(",")).or_default();
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
            "order [{order}]: {compared} decisions, {} flips, McNemar {statistic:.3}",
            b + c
        );
    }
}

fn agreement_table(out: &mut String, items: &[AgreementItem]) {
    let mut by_site: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    for item in items {
        let entry = by_site.entry(item.site.as_str()).or_default();
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

/// The tables for saved results: noise per site, every skip reason, option
/// order per order, and self-agreement per site.
#[must_use]
pub fn tables(results: &Results) -> String {
    let mut out = String::new();
    noise_table(&mut out, &results.noise);
    for (site, reason, count) in &results.skipped {
        let _ = writeln!(out, "skipped {site}, {reason}: {count}");
    }
    order_table(&mut out, &results.order);
    agreement_table(&mut out, &results.self_agreement);
    out
}

#[cfg(test)]
mod tests;
