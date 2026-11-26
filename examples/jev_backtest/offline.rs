//! Offline subcommands: the database only, no calls.

use mcp_parallax::checkpoint::Boundary;
use mcp_parallax::jev::metrics::{skip_recall, split, Split};
use mcp_parallax::jev::report as tables;
use mcp_parallax::storage::jev::JevRecord;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::classifier::JevSite;
use std::collections::BTreeMap;

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
    let to_u32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let mut results = crate::rebuild::load();
    results.base_rate = Some(mcp_parallax::jev::measure::BaseRate {
        evaluations: to_u32(turns.len()),
        reviews: to_u32(reviewed),
        fired: fired
            .iter()
            .map(|(kind, count)| ((*kind).to_string(), to_u32(*count)))
            .collect(),
    });
    crate::rebuild::save(&results);
    print!("{}", mcp_parallax::jev::measure::tables(&results));
}

/// How one stored row becomes a §7 item, or why it can't.
type ItemFn = fn(&JevRecord) -> Result<tables::Item, tables::Skip>;

/// Row counts and outcomes, then the plan §7 tables on the tune split (064
/// SC-005). Labels are the big model's: agreement, not correctness, which
/// needs hand labels. The confirm split is counted, never read.
pub async fn report(storage: &SqliteStorage) {
    let rows = storage.load_jev_records().await.expect("jev records");
    // The §7 skip-gate sites, each with how one of its rows becomes an item;
    // the injection site has its own report (065).
    let sites: [(JevSite, ItemFn); 2] = [
        (JevSite::TurnReview, |row| {
            tables::turn_item(
                row.label_json.as_ref(),
                &row.questions_json,
                row.harness_session_id.as_deref(),
            )
        }),
        (JevSite::ResearchClaims, |row| {
            tables::claim_item(
                row.label_json.as_ref(),
                &row.questions_json,
                row.invocation_id.as_deref(),
            )
        }),
    ];
    for (site, to_item) in sites {
        let site_rows: Vec<&JevRecord> = rows.iter().filter(|r| r.site == site).collect();
        let mut outcomes: BTreeMap<&str, usize> = BTreeMap::new();
        for row in &site_rows {
            *outcomes.entry(row.outcome.as_str()).or_default() += 1;
        }
        let labeled = site_rows.iter().filter(|r| r.label_json.is_some()).count();
        let cost: f64 = site_rows.iter().filter_map(|r| r.cost_usd).sum();
        let confirm = site_rows
            .iter()
            .filter(|r| split(&r.id, tables::CONFIRM_PERCENT) == Split::Confirm)
            .count();
        println!("== {} ({} rows)", site.as_str(), site_rows.len());
        for (outcome, count) in &outcomes {
            println!("  {outcome}: {count}");
        }
        println!("  labeled: {labeled}; cost: ${cost:.6}");
        println!("  confirm set: {confirm} rows, sealed (not looked at)");
        // Every labeled tune row counts, whatever its outcome: a row Jev did
        // not answer (excluded, failed, rejected) goes to the big model.
        let (mut items, mut unlabeled, mut abandoned) = (Vec::new(), 0, 0);
        for row in site_rows
            .iter()
            .filter(|r| split(&r.id, tables::CONFIRM_PERCENT) == Split::Tune)
        {
            let item = to_item(row);
            match item {
                Ok(item) => {
                    if tables::is_abandoned(row.error_class.as_deref()) {
                        abandoned += 1;
                    }
                    items.push(item);
                }
                Err(tables::Skip::Unlabeled) => unlabeled += 1,
            }
        }
        let t = tables::site_tables(&items);
        println!(
            "  tune items: {} ({} scored, {} unscored and never skipped, of which {abandoned} \
             abandoned because verify finished first; {} without a cluster); \
             left out: {unlabeled} unlabeled",
            t.items,
            t.items - t.unscored,
            t.unscored,
            t.unclustered
        );
        println!(
            "  evidence for recall, its bound and the dry run: {} scored, clustered items \
             ({} unscored harmful items are not counted)",
            t.evidence, t.unscored_harmful
        );
        print_tables(&t);
    }
}

fn fmt(value: Option<f64>) -> String {
    value.map_or_else(|| "undefined".into(), |v| format!("{v:.4}"))
}

fn fmt_pair(value: Option<(f64, f64)>) -> String {
    value.map_or_else(
        || "undefined".into(),
        |(lo, hi)| format!("{lo:.4}..{hi:.4}"),
    )
}

fn print_tables(t: &tables::SiteTables) {
    println!(
        "  harmful (plan 7.1): {} of {}; needed for promotion: {}",
        t.harmful,
        t.items,
        tables::MIN_POSITIVES
    );
    match t.threshold {
        Some(threshold) => println!(
            "  skip threshold: P(safe) >= {threshold} (target recall {}, alpha {})",
            tables::TARGET_RECALL,
            tables::ALPHA
        ),
        None => println!("  skip threshold: none meets the target; nothing is skipped"),
    }
    let c = &t.confusion;
    println!(
        "  at the threshold: skipped {} (harmful {}), sent to the big model {} (harmful {})",
        c.skips(),
        c.fn_,
        c.tp + c.fp,
        c.tp
    );
    println!(
        "  skip recall: {}; one-sided lower bound {}; cluster bootstrap {}",
        fmt(t.skip_recall),
        fmt(t.recall_lower),
        fmt_pair(t.recall_bootstrap)
    );
    println!(
        "  skip share: {}; one-sided lower bound {}",
        fmt(t.skip_share),
        fmt(t.skip_share_lower)
    );
    println!(
        "  harmful among skipped: {}; one-sided upper bound {}",
        fmt(t.harmful_among_skipped),
        fmt(t.harmful_among_skipped_upper)
    );
    println!(
        "  baselines: always skip: recall {}, skipped {}; never skip: recall {}, skipped {}",
        fmt(skip_recall(&t.always_skip)),
        t.always_skip.skips(),
        fmt(skip_recall(&t.never_skip)),
        t.never_skip.skips()
    );
    println!("  clusters: {}", t.clusters);
    println!(
        "  promotion dry run on the tune set (not a promotion): {}",
        if t.dry_run.promote { "passes" } else { "fails" }
    );
    for reason in &t.dry_run.reasons {
        println!("    - {reason}");
    }
    println!("  not checked by the dry run:");
    for criterion in t.unchecked {
        println!("    - {criterion}");
    }
}
