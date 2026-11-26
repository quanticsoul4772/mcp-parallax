//! 065 T028a: does the `directs_ai` decision depend on the order its two
//! criteria are declared in?
//!
//! Every dataset line is asked three times: criteria `true` first, `false`
//! first, and `true` first again. Order flips (first against second) print
//! beside repeat flips (first against third), so a flip count can be read
//! against Jev's own run-to-run noise. Nothing is saved; the printed lines go
//! into the measurements doc.

use crate::live::{
    jev, read_dataset, request, turn_questions, turn_rows, turn_version, values, TURN,
};
use crate::rebuild::{self, asked};
use mcp_parallax::client::typesafe::TypesafeClient;
use mcp_parallax::config::Config;
use mcp_parallax::jev::injection::{order_flips, OrderFlips};
use mcp_parallax::jev::measure::{self, YesNoOrderItem};
use mcp_parallax::jev::metrics::Decision;
use mcp_parallax::jev::questions::{
    injection_question, injection_question_swapped, turn_review_questions_swapped,
};
use mcp_parallax::jev::redact::{RedactedState, Redactor};
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::classifier::{Classifier, JevQuestion};

/// One call's decision at `threshold`; a failed call or a missing answer is
/// undecided, so the pair it belongs to is not compared.
async fn decide(
    client: &TypesafeClient,
    config: &Config,
    state: &RedactedState,
    question: &JevQuestion,
    threshold: f64,
) -> (Decision, bool) {
    let req = request(config, state.clone(), vec![question.clone()]);
    client
        .classify(&req)
        .await
        .map_or((Decision::Undecided, true), |response| {
            let decision = match values(&response).first().map(|(_, p, _)| *p) {
                Some(p) if p >= threshold => Decision::Yes,
                Some(_) => Decision::No,
                None => Decision::Undecided,
            };
            (decision, false)
        })
}

fn line(label: &str, f: &OrderFlips) -> String {
    let rate = f
        .flip_rate()
        .map_or_else(|| "undefined".to_string(), |r| format!("{r:.4}"));
    format!(
        "  {label}: {} compared, {} flips ({} yes to no, {} no to yes), flip rate {rate}, McNemar {:.3}",
        f.compared,
        f.flips(),
        f.yes_to_no,
        f.no_to_yes,
        f.mcnemar()
    )
}

/// Ask every line in both criteria orders and once more in the first.
pub async fn injection_order(config: &Config, folder: &str, threshold: f64) {
    let dataset = read_dataset(folder);
    let client = jev(config);
    let first = injection_question();
    let swapped = injection_question_swapped();
    let redactor = Redactor::new(vec![]);
    let mut order_pairs = Vec::new();
    let mut repeat_pairs = Vec::new();
    let mut failed = 0_u32;
    let mut unredacted = 0_u32;
    for item in &dataset.items {
        let page = serde_json::json!({ "page_url": item.url, "page_text": item.text });
        let Ok(state) = redactor.redact_json(&page) else {
            unredacted += 1;
            continue;
        };
        let (a, fa) = decide(&client, config, &state, &first, threshold).await;
        let (b, fb) = decide(&client, config, &state, &swapped, threshold).await;
        let (c, fc) = decide(&client, config, &state, &first, threshold).await;
        failed += u32::from(fa) + u32::from(fb) + u32::from(fc);
        order_pairs.push((a, b));
        repeat_pairs.push((a, c));
    }
    println!(
        "injection order at threshold {threshold}: {} items, {failed} failed calls, {unredacted} not redacted, {} dataset lines skipped",
        dataset.items.len(),
        dataset.skipped
    );
    println!(
        "{}",
        line(
            "order (true first vs false first)",
            &order_flips(&order_pairs)
        )
    );
    println!(
        "{}",
        line("repeat (true first twice)", &order_flips(&repeat_pairs))
    );
}

/// 066 T019c: ask each stored turn row's turn-review questions with their
/// criteria `true` first and `false` first, and save both P(yes) per
/// question, so the order check (FR-010) can be read at any cut.
pub async fn turn_order(config: &Config, storage: &SqliteStorage, limit: usize) {
    let client = jev(config);
    let mut results = rebuild::load();
    let (turns, mut counts) = turn_rows(storage, limit, |_| Ok(())).await;
    let version = turn_version();
    let mut items = Vec::new();
    for (row, turn) in turns {
        let ids = asked(&row);
        let swapped: Vec<JevQuestion> = turn_review_questions_swapped()
            .into_iter()
            .filter(|q| ids.contains(&q.id))
            .collect();
        let first = client
            .classify(&request(config, turn.state.clone(), turn_questions(&row)))
            .await;
        let second = client.classify(&request(config, turn.state, swapped)).await;
        let (Ok(first), Ok(second)) = (first, second) else {
            *counts.entry("failed calls".to_string()).or_default() += 1;
            continue;
        };
        let later = values(&second);
        for (id, p_true_first, _) in values(&first) {
            if let Some((_, p_false_first, _)) = later.iter().find(|(q, _, _)| *q == id) {
                items.push(YesNoOrderItem {
                    site: TURN.into(),
                    row_id: row.id.clone(),
                    question: id,
                    version,
                    true_first: p_true_first,
                    false_first: *p_false_first,
                });
            }
        }
    }
    results.replace_yes_no_order(TURN, version, items);
    results.replace_skips(&format!("yes/no order v{version}"), TURN, &counts);
    rebuild::save(&results);
    print!("{}", measure::tables(&results));
}
