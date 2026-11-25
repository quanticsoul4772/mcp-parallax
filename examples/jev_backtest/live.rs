//! Live subcommands: these call Jev or the big model, and spend.
//!
//! Noise, option order and self-agreement read tune-split rows only and save
//! what they measured to the results file, so `reproduce` prints the same
//! tables without calls (064 FR-011, SC-006).

use crate::rebuild::{self, asked, tune_success, Rebuilt};
use mcp_parallax::checkpoint::{review, SignalKind};
use mcp_parallax::client::default_model_client;
use mcp_parallax::client::typesafe::TypesafeClient;
use mcp_parallax::config::Config;
use mcp_parallax::jev::measure::{self, AgreementItem, NoiseItem, OrderItem};
use mcp_parallax::jev::metrics::{wilson, Confusion, Decision};
use mcp_parallax::jev::questions::{
    research_claim_questions, turn_review_questions, RELATION_OPTIONS,
};
use mcp_parallax::jev::redact::{RedactedState, Redactor};
use mcp_parallax::modes::{verify, ModeRegistry};
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

fn jev(config: &Config) -> TypesafeClient {
    TypesafeClient::new(&config.jev).expect("TYPESAFE_API_KEY is required for live subcommands")
}

/// Tune-split research rows that kept their state, oldest first.
async fn research_rows(storage: &SqliteStorage, limit: usize) -> Vec<JevRecord> {
    storage
        .load_jev_records()
        .await
        .expect("jev records")
        .into_iter()
        .filter(|r| {
            tune_success(r, JevSite::ResearchClaims)
                && r.state_json.is_some()
                && r.source_ids.is_some()
        })
        .take(limit)
        .collect()
}

/// Up to `limit` tune-split turns rebuilt from their transcripts, with the
/// count of every reason a turn could not be.
async fn turn_rows(
    storage: &SqliteStorage,
    limit: usize,
) -> (Vec<(JevRecord, Rebuilt)>, BTreeMap<&'static str, u32>) {
    let mut rebuilt = Vec::new();
    let mut skips: BTreeMap<&'static str, u32> = BTreeMap::new();
    let rows = storage.load_jev_records().await.expect("jev records");
    for row in rows
        .into_iter()
        .filter(|r| tune_success(r, JevSite::TurnReview))
    {
        if rebuilt.len() >= limit {
            break;
        }
        match rebuild::rebuild(&row).await {
            Ok(turn) => rebuilt.push((row, turn)),
            Err(reason) => *skips.entry(reason).or_default() += 1,
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

/// A number per question: P(yes) for yes/no, P(supports) for relation.
fn values(response: &JevResponse) -> Vec<(String, f64)> {
    response
        .answers
        .iter()
        .filter_map(|(id, answer)| match answer {
            JevAnswer::Noul { p } => Some((id.clone(), *p)),
            JevAnswer::Choice { probabilities, .. } => probabilities
                .iter()
                .find(|(o, _)| o == "supports")
                .map(|(_, p)| (id.clone(), *p)),
        })
        .collect()
}

/// Ask one request five times; one noise item per question answered.
async fn five(
    client: &TypesafeClient,
    req: &JevRequest,
    site: &str,
    row_id: &str,
    failures: &mut u32,
) -> Vec<NoiseItem> {
    let mut runs: Vec<Vec<(String, f64)>> = Vec::new();
    for _ in 0..5 {
        match client.classify(req).await {
            Ok(response) => runs.push(values(&response)),
            Err(_) => *failures += 1,
        }
    }
    let Some(first) = runs.first() else {
        return vec![];
    };
    first
        .iter()
        .map(|(id, _)| NoiseItem {
            site: site.into(),
            row_id: row_id.into(),
            question: id.clone(),
            samples: runs
                .iter()
                .filter_map(|run| run.iter().find(|(q, _)| q == id).map(|(_, p)| *p))
                .collect(),
        })
        .collect()
}

/// Five calls per tune row on both sites: spread on the logit scale and
/// decisions that flip at 0.5.
pub async fn noise(config: &Config, storage: &SqliteStorage, limit: usize) {
    let client = jev(config);
    let mut results = rebuild::load(rebuild::RESULTS_FILE);
    results.noise.clear();
    let mut failures = 0_u32;
    for row in research_rows(storage, limit).await {
        let req = research_request(config, &row, research_questions(&row));
        let items = five(&client, &req, RESEARCH, &row.id, &mut failures).await;
        results.noise.extend(items);
    }
    let (turns, skips) = turn_rows(storage, limit).await;
    for (row, turn) in turns {
        let req = request(config, turn.state, turn_questions(&row));
        let items = five(&client, &req, TURN, &row.id, &mut failures).await;
        results.noise.extend(items);
    }
    rebuild::record_skips(&mut results, "noise", TURN, &skips);
    rebuild::save(&results);
    println!("failed calls: {failures}");
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
    let rows = research_rows(storage, limit).await;
    let all = orders();
    let mut results = rebuild::load(rebuild::RESULTS_FILE);
    results.order.clear();
    let mut base: Vec<Vec<(String, bool)>> = Vec::new();
    for row in &rows {
        let response = client
            .classify(&research_request(config, row, research_questions(row)))
            .await;
        base.push(response.map(|r| supports_chosen(&r)).unwrap_or_default());
    }
    for order in all.iter().skip(1) {
        for (row, fixed) in rows.iter().zip(&base) {
            let questions = reordered(&research_questions(row), order);
            let Ok(response) = client
                .classify(&research_request(config, row, questions))
                .await
            else {
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
    rebuild::save(&results);
    print!("{}", measure::tables(&results));
}

/// The big model's repeatability on tune rows: the research verify run twice
/// on each stored claim, and the end-of-turn review run twice on each
/// rebuilt turn.
pub async fn self_agreement(config: &Config, storage: &SqliteStorage, limit: usize) {
    let client = default_model_client(config);
    let mut results = rebuild::load(rebuild::RESULTS_FILE);
    results.self_agreement.clear();

    let mut registry = ModeRegistry::new();
    verify::register(&mut registry, config.verify_ensemble_k).expect("register verify");
    review::register(&mut registry).expect("register review");
    let verify_mode = mcp_parallax::research::pipeline::research_verify_mode(
        registry.get(verify::VERIFY_ID).expect("verify mode"),
    );
    for row in research_rows(storage, limit).await {
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
        let first = verify::run(
            client.as_ref(),
            &verify_mode,
            &params,
            config.input_max_chars,
        )
        .await;
        let second = verify::run(
            client.as_ref(),
            &verify_mode,
            &params,
            config.input_max_chars,
        )
        .await;
        if let (Ok(a), Ok(b)) = (first, second) {
            results.self_agreement.push(AgreementItem {
                site: RESEARCH.into(),
                row_id: row.id.clone(),
                first: format!("{:?}", a.verdict.verdict),
                second: format!("{:?}", b.verdict.verdict),
            });
        }
    }

    let review_mode = registry.get(review::REVIEW_MODE_ID).expect("review mode");
    let (turns, skips) = turn_rows(storage, limit).await;
    for (row, turn) in turns {
        let capture_enabled = asked(&row).iter().any(|q| q == "capture_worthy");
        let candidates = review::mine_candidates(&turn.window, &turn.final_message, &[]);
        let mut verdicts = Vec::new();
        for _ in 0..2 {
            let outcome = review::review_once(
                client.as_ref(),
                review_mode,
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
        if let [first, second] = verdicts.as_slice() {
            results.self_agreement.push(AgreementItem {
                site: TURN.into(),
                row_id: row.id.clone(),
                first: first.clone(),
                second: second.clone(),
            });
        }
    }
    rebuild::record_skips(&mut results, "self-agreement", TURN, &skips);
    rebuild::save(&results);
    print!("{}", measure::tables(&results));
}

/// A labeled set from your folder, asked with the question in your file;
/// precision, recall and false-positive rate at `threshold`.
pub async fn injection(config: &Config, folder: &str, question_file: &str, threshold: f64) {
    let client = jev(config);
    let instructions = std::fs::read_to_string(question_file)
        .expect("question file")
        .trim()
        .to_string();
    let question = JevQuestion {
        id: "directs_ai".into(),
        version: 1,
        kind: QuestionKind::Noul,
        instructions,
        options: vec![],
    };
    let redactor = Redactor::new(vec![]);
    let mut pairs: Vec<(Decision, bool)> = Vec::new();
    let mut failures = 0_u32;
    let mut paths: Vec<_> = std::fs::read_dir(folder)
        .expect("dataset folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    paths.sort();
    for path in paths {
        for line in std::fs::read_to_string(&path)
            .expect("dataset file")
            .lines()
        {
            let Ok(item) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let (Some(text), Some(label)) = (item["text"].as_str(), item["label"].as_bool()) else {
                continue;
            };
            let Ok(state) = redactor.redact(text) else {
                failures += 1;
                continue;
            };
            let req = request(config, state, vec![question.clone()]);
            match client.classify(&req).await {
                Ok(response) => {
                    let p = values(&response).first().map(|(_, p)| *p);
                    let decision = match p {
                        Some(p) if p >= threshold => Decision::Yes,
                        Some(_) => Decision::No,
                        None => Decision::Undecided,
                    };
                    pairs.push((decision, label));
                }
                Err(_) => failures += 1,
            }
        }
    }
    let c = Confusion::from_pairs(&pairs);
    let negatives = c.fp + c.tn + c.undecided_negative;
    println!(
        "items: {}; failed: {failures}; threshold {threshold}",
        pairs.len()
    );
    println!("precision {:?}; recall {:?}", c.precision(), c.recall());
    println!(
        "recall 95% interval {:?}; false-positive rate 95% interval {:?}",
        wilson(c.tp, c.tp + c.fn_, 1.959_964),
        wilson(c.fp, negatives, 1.959_964)
    );
}
