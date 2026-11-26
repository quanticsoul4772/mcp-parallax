//! The injection site's report from stored records (065 FR-009).
//!
//! How often Jev warned, why calls failed, and how Jev's answer compares
//! with 063's pattern screen on the same pages. Agreement is not
//! correctness: neither screen is ground truth.

use crate::error::Outcome;
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::JevSite;
use std::collections::BTreeMap;

/// Jev's warning against 063's flag, over screened pages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Agreement {
    /// Both flagged the page.
    pub both: u32,
    /// Jev warned; 063 found no pattern.
    pub jev_only: u32,
    /// 063 found a pattern; Jev did not warn.
    pub only_063: u32,
    /// Neither flagged it.
    pub neither: u32,
}

/// The injection site's table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InjectionTable {
    /// Injection rows given.
    pub rows: u32,
    /// Rows with a decision (band `warned` or `clear`).
    pub screened: u32,
    /// Rows Jev warned on.
    pub warned: u32,
    /// Rows that did not end in success, by outcome, in name order.
    pub failures: Vec<(String, u32)>,
    /// Over screened rows.
    pub agreement: Agreement,
}

/// Whether an injection row has a usable answer: the call succeeded and
/// Jev returned a probability. The same rule `screen_page` uses, so every
/// surface counts screened pages alike.
#[must_use]
pub fn is_screened(row: &JevRecord) -> bool {
    row.outcome == JevOutcome::Call(Outcome::Success) && row.questions_json[0]["answer"].is_f64()
}

/// The table for `rows`; rows from other sites are ignored.
#[must_use]
pub fn injection_table(rows: &[&JevRecord]) -> InjectionTable {
    let mut table = InjectionTable::default();
    let mut failures: BTreeMap<String, u32> = BTreeMap::new();
    for row in rows.iter().filter(|r| r.site == JevSite::ResearchInjection) {
        table.rows += 1;
        if row.outcome != JevOutcome::Call(Outcome::Success) {
            *failures
                .entry(row.outcome.as_str().to_string())
                .or_default() += 1;
        }
        if !is_screened(row) {
            continue;
        }
        table.screened += 1;
        // Warned versus clear needs a pinned threshold; without one there is
        // a probability but no decision to compare.
        let warned = match row.questions_json[0]["band"].as_str() {
            Some("warned") => true,
            Some("clear") => false,
            _ => continue,
        };
        table.warned += u32::from(warned);
        let flagged_063 = row
            .label_json
            .as_ref()
            .and_then(|l| l["patterns063"].as_array())
            .is_some_and(|p| !p.is_empty());
        let a = &mut table.agreement;
        match (warned, flagged_063) {
            (true, true) => a.both += 1,
            (true, false) => a.jev_only += 1,
            (false, true) => a.only_063 += 1,
            (false, false) => a.neither += 1,
        }
    }
    table.failures = failures.into_iter().collect();
    table
}

#[cfg(test)]
mod tests;
