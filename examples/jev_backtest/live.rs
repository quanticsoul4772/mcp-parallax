//! Live subcommands: these call Jev or the big model, and spend.
//!
//! Noise, option order and self-agreement read tune-split rows only and save
//! what they measured, failures included, to the results file, so
//! `reproduce` prints the same tables without calls (064 FR-011, SC-006).

use crate::rebuild::{self, asked, tune_success, Rebuilt};
use mcp_parallax::checkpoint::{review, SignalKind};
use mcp_parallax::client::pool::ClientPool;
use mcp_parallax::client::typesafe::TypesafeClient;
use mcp_parallax::config::Config;
use mcp_parallax::jev::injection::{injection_question, parse_dataset};
use mcp_parallax::jev::measure::{self, AgreementItem, InjectionResult, NoiseItem, OrderItem};
use mcp_parallax::jev::metrics::{Confusion, Decision};
use mcp_parallax::jev::questions::{
    research_claim_questions, turn_review_questions, RELATION_OPTIONS,
};
use mcp_parallax::jev::redact::{RedactedState, Redactor};
use mcp_parallax::modes::{verify, ModeRegistry};
use mcp_parallax::routing::CallSite;
use mcp_parallax::storage::jev::JevRecord;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::classifier::{
    Classifier, JevAnswer, JevQuestion, JevRequest, JevResponse, JevSite, QuestionKind,
};
use std::collections::BTreeMap;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);
const RESEARCH: &str = "research_claims";
const TURN: &str = "turn_review";

type Counts = BTreeMap<String, u32>;

fn bump(counts: &mut Counts, reason: &str) {
    *counts.entry(reason.to_string()).or_default() += 1;
}

fn jev(config: &Config) -> TypesafeClient {
    TypesafeClient::new(&config.jev).expect("TYPESAFE_API_KEY is required for live subcommands")
}

/// Tune-split research rows that kept their state, oldest first, with the
/// count of tune rows left out for lacking it.
async fn research_rows(storage: &SqliteStorage, limit: usize) -> (Vec<JevRecord>, Counts) {
    let mut skips = Counts::new();
    let mut rows = Vec::new();
    let all = storage.load_jev_records().await.expect("jev records");
    for row in all
        .into_iter()
        .filter(|r| tune_success(r, JevSite::ResearchClaims))
    {
        if rows.len() >= limit {
            break;
        }
        if row.state_json.is_some() && row.source_ids.is_some() {
            rows.push(row);
        } else {
            bump(&mut skips, "no stored state");
        }
    }
    (rows, skips)
}

/// Up to `limit` tune-split turns rebuilt from their transcripts that pass
/// `keep`, with the count of every reason a turn was left out.
async fn turn_rows(
    storage: &SqliteStorage,
    limit: usize,
    keep: impl Fn(&JevRecord) -> Result<(), &'static str>,
) -> (Vec<(JevRecord, Rebuilt)>, Counts) {
    let mut rebuilt = Vec::new();
    let mut skips = Counts::new();
    let rows = storage.load_jev_records().await.expect("jev records");
    for row in rows
        .into_iter()
        .filter(|r| tune_success(r, JevSite::TurnReview))
    {
        if rebuilt.len() >= limit {
            break;
        }
        if let Err(reason) = keep(&row) {
            bump(&mut skips, reason);
            continue;
        }
        match rebuild::rebuild(&row).await {
            Ok(turn) => rebuilt.push((row, turn)),
            Err(reason) => bump(&mut skips, reason),
        }
    }
    (rebuilt, skips)
}

fn request(config: &Config, state: RedactedState, questions: Vec<JevQuestion>) -> JevRequest {
    JevRequest {
        model: config.jev.model.clone(),
        state,
        questions,
        timeout: TIMEOUT,
    }
}

fn research_request(config: &Config, row: &JevRecord, questions: Vec<JevQuestion>) -> JevRequest {
    let state = row.state_json.as_ref().expect("filtered on state_json");
    let state = Redactor::new(vec![])
        .redact_json(state)
        .expect("stored state redacts");
    request(config, state, questions)
}

fn research_questions(row: &JevRecord) -> Vec<JevQuestion> {
    research_claim_questions(row.source_ids.as_ref().map_or(0, Vec::len))
}

fn turn_questions(row: &JevRecord) -> Vec<JevQuestion> {
    let ids = asked(row);
    turn_review_questions()
        .into_iter()
        .filter(|q| ids.contains(&q.id))
        .collect()
}

/// Per question: P(yes) for yes/no, P(supports) for relation, and the chosen
/// option for a choice question.
fn values(response: &JevResponse) -> Vec<(String, f64, Option<String>)> {
    response
        .answers
        .iter()
        .filter_map(|(id, answer)| match answer {
            JevAnswer::Noul { p } => Some((id.clone(), *p, None)),
            JevAnswer::Choice {
                choice,
                probabilities,
            } => probabilities
                .iter()
                .find(|(o, _)| o == "supports")
                .map(|(_, p)| (id.clone(), *p, Some(choice.clone()))),
        })
        .collect()
}

/// Ask one request five times; one noise item per question answered.
async fn five(
    client: &TypesafeClient,
    req: &JevRequest,
    site: &str,
    row_id: &str,
    losses: &mut Counts,
) -> Vec<NoiseItem> {
    let mut runs: Vec<Vec<(String, f64, Option<String>)>> = Vec::new();
    for _ in 0..5 {
        match client.classify(req).await {
            Ok(response) => runs.push(values(&response)),
            Err(_) => bump(losses, "failed calls"),
        }
    }
    let Some(first) = runs.first() else {
        return vec![];
    };
    first
        .iter()
        .map(|(id, _, _)| {
            let answers: Vec<&(String, f64, Option<String>)> = runs
                .iter()
                .filter_map(|run| run.iter().find(|(q, _, _)| q == id))
                .collect();
            NoiseItem {
                site: site.into(),
                row_id: row_id.into(),
                question: id.clone(),
                samples: answers.iter().map(|(_, p, _)| *p).collect(),
                choices: answers.iter().filter_map(|(_, _, c)| c.clone()).collect(),
            }
        })
        .collect()
}

/// Five calls per tune row on both sites: spread on the logit scale per
/// question, and decisions that change across the calls.
pub async fn noise(config: &Config, storage: &SqliteStorage, limit: usize) {
    let client = jev(config);
    let mut results = rebuild::load();
    // A site's saved items and counts are replaced only when this database
    // had usable rows for it; otherwise an earlier run's results stay.
    let (rows, mut research_counts) = research_rows(storage, limit).await;
    let research_items = if rows.is_empty() {
        None
    } else {
        let mut items = Vec::new();
        for row in rows {
            let req = research_request(config, &row, research_questions(&row));
            items.extend(five(&client, &req, RESEARCH, &row.id, &mut research_counts).await);
        }
        Some(items)
    };
    results.record_noise_run(RESEARCH, research_items, &research_counts);
    let (turns, mut turn_counts) = turn_rows(storage, limit, |_| Ok(())).await;
    let turn_items = if turns.is_empty() {
        None
    } else {
        let mut items = Vec::new();
        for (row, turn) in turns {
            let req = request(config, turn.state, turn_questions(&row));
            items.extend(five(&client, &req, TURN, &row.id, &mut turn_counts).await);
        }
        Some(items)
    };
    results.record_noise_run(TURN, turn_items, &turn_counts);
    results.drop_legacy_all();
    rebuild::save(&results);
    print!("{}", measure::tables(&results));
}

/// The six orders of the relation options.
fn orders() -> Vec<Vec<&'static str>> {
    let [a, b, c] = RELATION_OPTIONS;
    vec![
        vec![a, b, c],
        vec![a, c, b],
        vec![b, a, c],
        vec![b, c, a],
        vec![c, a, b],
        vec![c, b, a],
    ]
}

fn reordered(questions: &[JevQuestion], order: &[&str]) -> Vec<JevQuestion> {
    questions
        .iter()
        .map(|q| {
            let mut q = q.clone();
            if q.kind == QuestionKind::Choice {
                q.options = order
                    .iter()
                    .filter_map(|o| q.options.iter().find(|(name, _)| name == o).cloned())
                    .collect();
            }
            q
        })
        .collect()
}

fn supports_chosen(response: &JevResponse) -> Vec<(String, bool)> {
    response
        .answers
        .iter()
        .filter_map(|(id, answer)| match answer {
            JevAnswer::Choice { choice, .. } => Some((id.clone(), choice == "supports")),
            JevAnswer::Noul { .. } => None,
        })
        .collect()
}

/// Each tune research row under all six option orders (the turn site asks
/// no choice questions).
pub async fn order(config: &Config, storage: &SqliteStorage, limit: usize) {
    let client = jev(config);
    let (rows, skips) = research_rows(storage, limit).await;
    let all = orders();
    let mut results = rebuild::load();
    results.order.clear();
    let mut losses = Counts::new();
    let mut base: Vec<Option<Vec<(String, bool)>>> = Vec::new();
    for row in &rows {
        let response = client
            .classify(&research_request(config, row, research_questions(row)))
            .await;
        if let Ok(response) = response {
            base.push(Some(supports_chosen(&response)));
        } else {
            bump(&mut losses, "base call failed");
            base.push(None);
        }
    }
    for order in all.iter().skip(1) {
        for (row, fixed) in rows.iter().zip(&base) {
            let Some(fixed) = fixed else { continue };
            let questions = reordered(&research_questions(row), order);
            let Ok(response) = client
                .classify(&research_request(config, row, questions))
                .await
            else {
                bump(&mut losses, "reordered call failed");
                continue;
            };
            for (id, now) in supports_chosen(&response) {
                if let Some((_, before)) = fixed.iter().find(|(q, _)| *q == id) {
                    results.order.push(OrderItem {
                        row_id: row.id.clone(),
                        question: id,
                        order: order.iter().map(|o| (*o).to_string()).collect(),
                        base_supports: *before,
                        supports: now,
                    });
                }
            }
        }
    }
    let mut counts = skips;
    counts.extend(losses);
    rebuild::record_skips(&mut results, "order", RESEARCH, &counts);
    rebuild::save(&results);
    print!("{}", measure::tables(&results));
}

/// A turn whose review saw no memory-derived candidates: its label records a
/// contradiction outcome (null with memory candidates, absent on rows
/// written before the outcome fields existed).
fn no_memory_candidates(row: &JevRecord) -> Result<(), &'static str> {
    match row.label_json.as_ref().map(|l| &l["contradiction_flagged"]) {
        Some(serde_json::Value::Bool(_)) => Ok(()),
        Some(serde_json::Value::Null) => Err("memory candidates present"),
        _ => Err("no review outcome in the label"),
    }
}

/// The research verify run twice on one stored claim with its stored
/// excerpts; `None` when either run failed (the second is skipped then).
async fn verify_twice(
    config: &Config,
    client: &dyn mcp_parallax::traits::client::ModelClient,
    mode: &mcp_parallax::modes::CorrectiveMode,
    row: &JevRecord,
) -> Option<AgreementItem> {
    let state = row.state_json.as_ref().expect("filtered on state_json");
    let excerpts: Vec<String> = state["sources"]
        .as_object()
        .map(|sources| {
            sources
                .values()
                .filter_map(|s| s["excerpts"][0].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let params = verify::VerifyParams {
        claim: state["claim"].as_str().unwrap_or_default().to_string(),
        context: Some(format!("Source excerpts:\n\n{}", excerpts.join("\n\n"))),
        effort: None,
        passes: None,
    };
    let max = config.input_max_chars;
    let first = verify::run(client, mode, &params, max).await.ok()?;
    let second = verify::run(client, mode, &params, max).await.ok()?;
    Some(AgreementItem {
        site: RESEARCH.into(),
        row_id: row.id.clone(),
        first: format!("{:?}", first.verdict.verdict),
        second: format!("{:?}", second.verdict.verdict),
    })
}

/// The end-of-turn review run twice on one rebuilt turn; `None` when either
/// run failed. Both runs always happen.
async fn review_twice(
    client: &dyn mcp_parallax::traits::client::ModelClient,
    mode: &mcp_parallax::modes::CorrectiveMode,
    row: &JevRecord,
    turn: &Rebuilt,
) -> Option<AgreementItem> {
    let capture_enabled = asked(row).iter().any(|q| q == "capture_worthy");
    let candidates = review::mine_candidates(&turn.window, &turn.final_message, &[]);
    let mut verdicts = Vec::new();
    for _ in 0..2 {
        let outcome = review::review_once(
            client,
            mode,
            &candidates,
            &[],
            &turn.final_message,
            &turn.activity,
            capture_enabled,
        )
        .await;
        if let Ok((flags, proposal, _, _, _)) = outcome {
            let has = |kind: SignalKind| flags.iter().any(|(s, _)| s.kind == kind);
            verdicts.push(format!(
                "contradiction={} capture={}",
                has(SignalKind::SelfContradiction),
                proposal.is_some()
            ));
        }
    }
    let [first, second] = verdicts.as_slice() else {
        return None;
    };
    Some(AgreementItem {
        site: TURN.into(),
        row_id: row.id.clone(),
        first: first.clone(),
        second: second.clone(),
    })
}

/// The big model's repeatability on tune rows, through the server's own
/// per-site routing: the research verify run twice on each stored claim,
/// and the end-of-turn review run twice on each rebuilt turn whose review
/// saw no memory candidates.
pub async fn self_agreement(config: &Config, storage: &SqliteStorage, limit: usize) {
    let http = reqwest::Client::new();
    let pool = ClientPool::from_factory(&config.routing, |model, effort| {
        mcp_parallax::client::build_model_client(config, &http, model, effort)
    });
    let verify_client = pool.for_site(CallSite::ResearchVerify);
    let review_client = pool.for_site(CallSite::CheckpointReview);
    let research_note = format!(
        "self-agreement {RESEARCH}: research verify model {} (effort {:?})",
        config.routing.model_for(CallSite::ResearchVerify),
        config.routing.effort_for(CallSite::ResearchVerify),
    );
    let turn_note = format!(
        "self-agreement {TURN}: turn review model {} (effort {:?}); run without stored \
         preferences, which aren't recorded",
        config.routing.model_for(CallSite::CheckpointReview),
        config.routing.effort_for(CallSite::CheckpointReview),
    );
    let mut results = rebuild::load();

    let mut registry = ModeRegistry::new();
    verify::register(&mut registry, config.verify_ensemble_k).expect("register verify");
    review::register(&mut registry).expect("register review");
    let verify_mode = mcp_parallax::research::pipeline::research_verify_mode(
        registry.get(verify::VERIFY_ID).expect("verify mode"),
    );
    // As in noise: a site's items, and its model note, are replaced only when
    // this database had usable rows for it.
    let (rows, mut research_counts) = research_rows(storage, limit).await;
    let mut research_items = None;
    if !rows.is_empty() {
        let mut items = Vec::new();
        for row in rows {
            match verify_twice(config, verify_client.as_ref(), &verify_mode, &row).await {
                Some(item) => items.push(item),
                None => bump(&mut research_counts, "pair dropped: a run failed"),
            }
        }
        research_items = Some(items);
        results.replace_note(&format!("self-agreement {RESEARCH}"), research_note);
    }
    results.record_agreement_run(RESEARCH, research_items, &research_counts);

    let review_mode = registry.get(review::REVIEW_MODE_ID).expect("review mode");
    let (turns, mut turn_counts) = turn_rows(storage, limit, no_memory_candidates).await;
    let mut turn_items = None;
    if !turns.is_empty() {
        let mut items = Vec::new();
        for (row, turn) in turns {
            match review_twice(review_client.as_ref(), review_mode, &row, &turn).await {
                Some(item) => items.push(item),
                None => bump(&mut turn_counts, "pair dropped: a run failed"),
            }
        }
        turn_items = Some(items);
        results.replace_note(&format!("self-agreement {TURN}"), turn_note);
    }
    results.record_agreement_run(TURN, turn_items, &turn_counts);
    results.drop_legacy_notes();
    rebuild::save(&results);
    print!("{}", measure::tables(&results));
}

/// A labeled set from your folder, asked with the question in your file;
/// saved with the other results and printed as a table.
pub async fn injection(config: &Config, folder: &str, question_file: &str, threshold: f64) {
    // Checked before any call is paid for: a broken file stops the run here.
    let mut results = rebuild::load();
    let client = jev(config);
    let question =
        injection_question(&std::fs::read_to_string(question_file).expect("question file"))
            .unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1)
            });
    let redactor = Redactor::new(vec![]);
    let mut pairs: Vec<(Decision, bool)> = Vec::new();
    let mut failed = 0_u32;
    let mut paths: Vec<_> = std::fs::read_dir(folder)
        .expect("dataset folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    paths.sort();
    let files: Vec<String> = paths
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("dataset file"))
        .collect();
    // Every line is checked before any call: a dataset with nothing usable
    // stops here, and the saved result is left as it was.
    let dataset = parse_dataset(files.iter().flat_map(|f| f.lines())).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    for item in dataset.items {
        let page = serde_json::json!({ "page_url": item.url, "page_text": item.text });
        let Ok(state) = redactor.redact_json(&page) else {
            failed += 1;
            continue;
        };
        let req = request(config, state, vec![question.clone()]);
        match client.classify(&req).await {
            Ok(response) => {
                let p = values(&response).first().map(|(_, p, _)| *p);
                let decision = match p {
                    Some(p) if p >= threshold => Decision::Yes,
                    Some(_) => Decision::No,
                    None => Decision::Undecided,
                };
                pairs.push((decision, item.label));
            }
            Err(_) => failed += 1,
        }
    }
    let c = Confusion::from_pairs(&pairs);
    results.injection = Some(InjectionResult {
        threshold,
        items: u32::try_from(pairs.len()).unwrap_or(u32::MAX),
        failed,
        tp: c.tp,
        fp: c.fp,
        tn: c.tn,
        fn_: c.fn_,
        undecided_positive: c.undecided_positive,
        undecided_negative: c.undecided_negative,
        skipped: dataset.skipped,
    });
    rebuild::save(&results);
    print!("{}", measure::tables(&results));
}
