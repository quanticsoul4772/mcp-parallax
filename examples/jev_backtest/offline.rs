//! Offline subcommands: the database only, no calls.

use mcp_parallax::checkpoint::Boundary;
use mcp_parallax::jev::metrics::{split, Confusion, Decision, Split};
use mcp_parallax::storage::jev::{JevOutcome, JevRecord};
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::classifier::JevSite;
use std::collections::BTreeMap;

/// Share of the rows that open the sealed confirm set.
const CONFIRM_PERCENT: u64 = 30;

/// Past turn reviews and the signals they fired (sizes the shadow period).
pub async fn base_rate(storage: &SqliteStorage) {
    let records = storage
        .list_checkpoints()
        .await
        .expect("checkpoint records");
    let turns: Vec<_> = records
        .iter()
        .filter(|r| r.boundary == Boundary::Turn)
        .collect();
    let reviewed = turns.iter().filter(|r| r.review_ran).count();
    let mut fired: BTreeMap<&str, usize> = BTreeMap::new();
    for record in &turns {
        for signal in &record.signals_fired {
            *fired.entry(signal.kind.as_str()).or_default() += 1;
        }
    }
    println!("turn evaluations: {}", turns.len());
    println!("reviews run:      {reviewed}");
    for (kind, count) in &fired {
        println!("fired {kind}: {count}");
    }
}

fn noul_answer(row: &JevRecord, question: &str) -> Option<f64> {
    row.questions_json
        .as_array()?
        .iter()
        .find(|q| q["id"] == question)?["answer"]
        .as_f64()
}

/// Row counts, outcomes and agreement with the big model on the tune split.
/// Agreement is not correctness: correctness needs hand labels.
pub async fn report(storage: &SqliteStorage) {
    let rows = storage.load_jev_records().await.expect("jev records");
    for site in JevSite::ALL {
        let site_rows: Vec<&JevRecord> = rows.iter().filter(|r| r.site == site).collect();
        let mut outcomes: BTreeMap<&str, usize> = BTreeMap::new();
        for row in &site_rows {
            *outcomes.entry(row.outcome.as_str()).or_default() += 1;
        }
        let labeled = site_rows.iter().filter(|r| r.label_json.is_some()).count();
        let cost: f64 = site_rows.iter().filter_map(|r| r.cost_usd).sum();
        let confirm = site_rows
            .iter()
            .filter(|r| split(&r.id, CONFIRM_PERCENT) == Split::Confirm)
            .count();
        println!("== {} ({} rows)", site.as_str(), site_rows.len());
        for (outcome, count) in &outcomes {
            println!("  {outcome}: {count}");
        }
        println!("  labeled: {labeled}; cost: ${cost:.6}");
        println!("  confirm set: {confirm} rows, sealed (not looked at)");
        let pairs: Vec<(Decision, bool)> = site_rows
            .iter()
            .filter(|r| split(&r.id, CONFIRM_PERCENT) == Split::Tune)
            .filter(|r| r.outcome == JevOutcome::Call(mcp_parallax::error::Outcome::Success))
            .filter_map(|r| agreement_pair(site, r))
            .collect();
        let c = Confusion::from_pairs(&pairs);
        println!(
            "  agreement with the big model on {} tune rows (not correctness): \
             precision {:?}, recall {:?}",
            pairs.len(),
            c.precision(),
            c.recall()
        );
    }
}

/// Jev's answer (at 0.5) against the big model's label, per site.
fn agreement_pair(site: JevSite, row: &JevRecord) -> Option<(Decision, bool)> {
    let label = row.label_json.as_ref()?;
    match site {
        JevSite::TurnReview => {
            let positive = label["contradicts"].as_bool()?;
            let p = noul_answer(row, "contradiction")?;
            Some((
                if p >= 0.5 {
                    Decision::Yes
                } else {
                    Decision::No
                },
                positive,
            ))
        }
        JevSite::ResearchClaims => {
            let supported = matches!(label["support"].as_str()?, "confirmed" | "unverified");
            let first = row.questions_json.as_array()?.first()?;
            let says_supports = first["answer"].as_str()? == "supports";
            Some((
                if says_supports {
                    Decision::Yes
                } else {
                    Decision::No
                },
                supported,
            ))
        }
    }
}
