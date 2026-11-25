//! Live-measurement helpers for the back-test harness (064 FR-011, SC-006).
//!
//! The spread and flip statistics for noise, the transcript cut that lets the
//! harness rebuild a stored turn (turn rows keep only a fingerprint of their
//! state), and the saved results file with the tables printed from it, so a
//! live run can be reproduced without calling anything.

use crate::jev::metrics::Confusion;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Counts the harness records for calls lost from items it did save, as
/// opposed to rows it skipped.
pub const LOSS_REASONS: [&str; 2] = ["failed calls", "pair dropped: a run failed"];

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

/// The question a per-source id belongs to: `relation_3` is `relation`.
#[must_use]
pub fn question_family(id: &str) -> &str {
    match id.rsplit_once('_') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => id,
    }
}

/// Whether an item's decision changed across its calls: the chosen option
/// for a choice question, the 0.5 cut for a yes/no question.
#[must_use]
pub fn item_flipped(item: &NoiseItem) -> bool {
    if item.choices.is_empty() {
        flipped(&item.samples, 0.5)
    } else {
        item.choices.iter().any(|c| *c != item.choices[0])
    }
}

fn timestamp(line: &str) -> Option<DateTime<Utc>> {
    let value: Value = serde_json::from_str(line).ok()?;
    value["timestamp"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// The transcript as it stood at `until`: every line up to the last
/// timestamped line at or before `until`, stopping at the first later one,
/// with the file's own line endings kept.
#[must_use]
pub fn transcript_until(text: &str, until: DateTime<Utc>) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut end = 0;
    for (i, line) in lines.iter().enumerate() {
        match timestamp(line) {
            Some(t) if t > until => break,
            Some(_) => end = i + 1,
            None => {}
        }
    }
    lines[..end].concat()
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

/// One item asked several times.
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
    /// The option chosen on each call, for a choice question; empty for a
    /// yes/no question.
    #[serde(default)]
    pub choices: Vec<String>,
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

/// The FR-011(d) base rate from the checkpoint records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseRate {
    /// Turn evaluations.
    pub evaluations: u32,
    /// Turn reviews that ran.
    pub reviews: u32,
    /// Signals fired, by kind.
    pub fired: Vec<(String, u32)>,
}

/// The FR-011(e) injection back-test at one threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InjectionResult {
    /// The threshold used.
    pub threshold: f64,
    /// Items answered.
    pub items: u32,
    /// Items that failed or could not be redacted.
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

/// Everything the measurement runs produced, saved so the tables can be
/// printed again without calls (SC-006).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Results {
    /// Noise items.
    pub noise: Vec<NoiseItem>,
    /// Option-order items.
    pub order: Vec<OrderItem>,
    /// Self-agreement items.
    pub self_agreement: Vec<AgreementItem>,
    /// Rows left out and calls lost: (site, reason, count).
    pub skipped: Vec<(String, String, u32)>,
    /// The base rate.
    pub base_rate: Option<BaseRate>,
    /// The injection back-test.
    pub injection: Option<InjectionResult>,
    /// What the runs printed about how they measured (the models used, what
    /// was not replayed), so `reproduce` shows it too.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl Results {
    /// Replace the noise items of `site` only; other sites' items stay.
    pub fn replace_noise(&mut self, site: &str, items: Vec<NoiseItem>) {
        self.noise.retain(|i| i.site != site);
        self.noise.extend(items);
    }

    /// Replace the self-agreement items of `site` only; other sites' items
    /// stay.
    pub fn replace_agreement(&mut self, site: &str, items: Vec<AgreementItem>) {
        self.self_agreement.retain(|i| i.site != site);
        self.self_agreement.extend(items);
    }

    /// Replace the skip and loss counts a measurement recorded for `site`
    /// under `prefix`; zero counts are not kept.
    pub fn replace_skips(&mut self, prefix: &str, site: &str, counts: &BTreeMap<String, u32>) {
        let tag = format!("{prefix}: ");
        self.skipped
            .retain(|(s, reason, _)| !(s == site && reason.starts_with(&tag)));
        for (reason, count) in counts {
            if *count > 0 {
                self.skipped
                    .push((site.to_string(), format!("{tag}{reason}"), *count));
            }
        }
    }

    /// Replace only the skip reasons of `site` under `prefix`, keeping its
    /// loss counts (calls that failed, pairs dropped), which describe the
    /// items an earlier run saved.
    fn replace_skip_reasons(&mut self, prefix: &str, site: &str, counts: &BTreeMap<String, u32>) {
        let tag = format!("{prefix}: ");
        let is_loss = |reason: &str| LOSS_REASONS.iter().any(|l| reason == format!("{tag}{l}"));
        self.skipped
            .retain(|(s, reason, _)| !(s == site && reason.starts_with(&tag)) || is_loss(reason));
        for (reason, count) in counts {
            if *count > 0 && !LOSS_REASONS.contains(&reason.as_str()) {
                self.skipped
                    .push((site.to_string(), format!("{tag}{reason}"), *count));
            }
        }
    }

    /// Save one site's noise run. `items` is `None` when the run had no
    /// usable row for the site: the saved items stay, and so do the loss
    /// counts behind them; only the skip reasons are replaced. With items,
    /// everything for the site is replaced.
    pub fn record_noise_run(
        &mut self,
        site: &str,
        items: Option<Vec<NoiseItem>>,
        counts: &BTreeMap<String, u32>,
    ) {
        match items {
            Some(items) => {
                self.replace_noise(site, items);
                self.replace_skips("noise", site, counts);
            }
            None if counts.values().any(|c| *c > 0) => {
                self.replace_skip_reasons("noise", site, counts);
            }
            None => {}
        }
    }

    /// Save one site's self-agreement run, by the same rule as
    /// [`Self::record_noise_run`].
    pub fn record_agreement_run(
        &mut self,
        site: &str,
        items: Option<Vec<AgreementItem>>,
        counts: &BTreeMap<String, u32>,
    ) {
        match items {
            Some(items) => {
                self.replace_agreement(site, items);
                self.replace_skips("self-agreement", site, counts);
            }
            None if counts.values().any(|c| *c > 0) => {
                self.replace_skip_reasons("self-agreement", site, counts);
            }
            None => {}
        }
    }

    /// Replace the notes starting with `prefix` with `note`; other notes
    /// stay.
    pub fn replace_note(&mut self, prefix: &str, note: String) {
        self.notes.retain(|n| !n.starts_with(prefix));
        self.notes.push(note);
    }

    /// Drop skip entries under the old `all` site, which earlier harness
    /// versions wrote and no run replaces any more.
    pub fn drop_legacy_all(&mut self) {
        self.skipped.retain(|(site, _, _)| site != "all");
    }

    /// Drop self-agreement notes in the old single-note format, which the
    /// per-site notes no longer replace.
    pub fn drop_legacy_notes(&mut self) {
        self.notes.retain(|n| {
            !n.starts_with("self-agreement models:") && !n.starts_with("self-agreement: ")
        });
    }
}

/// Read a results file. A missing file is empty results unless `must_exist`;
/// an unreadable or unparseable file is always an error, never empty results.
///
/// # Errors
///
/// The file is missing while `must_exist`, unreadable, or not valid results.
pub fn read_results(path: &Path, must_exist: bool) -> Result<Results, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{} is not a valid results file: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !must_exist => Ok(Results::default()),
        Err(e) => Err(format!("{} can't be read: {e}", path.display())),
    }
}

fn noise_table(out: &mut String, noise: &[NoiseItem]) {
    let mut groups: BTreeMap<(&str, &str), Vec<&NoiseItem>> = BTreeMap::new();
    for item in noise {
        groups
            .entry((item.site.as_str(), question_family(&item.question)))
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
         injections, {} clean",
        interval(c.tp, c.tp + c.fn_),
        interval(c.fp, negatives),
        c.undecided_positive,
        c.undecided_negative
    );
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

#[cfg(test)]
mod tests;
