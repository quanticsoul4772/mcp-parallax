//! Live subcommands: these call Jev or the big model, and spend.

use mcp_parallax::client::default_model_client;
use mcp_parallax::client::typesafe::TypesafeClient;
use mcp_parallax::config::Config;
use mcp_parallax::error::Outcome;
use mcp_parallax::jev::metrics::{wilson, Confusion, Decision};
use mcp_parallax::jev::questions::{research_claim_questions, RELATION_OPTIONS};
use mcp_parallax::jev::redact::Redactor;
use mcp_parallax::modes::{verify, ModeRegistry};
use mcp_parallax::storage::jev::{JevOutcome, JevRecord};
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::classifier::{
    Classifier, JevAnswer, JevQuestion, JevRequest, JevResponse, JevSite, QuestionKind,
};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

fn jev(config: &Config) -> TypesafeClient {
    TypesafeClient::new(&config.jev).expect("TYPESAFE_API_KEY is required for live subcommands")
}

/// Successful research rows that kept their state, newest last.
async fn research_rows(storage: &SqliteStorage, limit: usize) -> Vec<JevRecord> {
    storage
        .load_jev_records()
        .await
        .expect("jev records")
        .into_iter()
        .filter(|r| {
            r.site == JevSite::ResearchClaims
                && r.outcome == JevOutcome::Call(Outcome::Success)
                && r.state_json.is_some()
                && r.source_ids.is_some()
        })
        .take(limit)
        .collect()
}

fn request(config: &Config, row: &JevRecord, questions: Vec<JevQuestion>) -> JevRequest {
    let state = row.state_json.as_ref().expect("filtered on state_json");
    JevRequest {
        model: config.jev.model.clone(),
        state: Redactor::new(vec![])
            .redact_json(state)
            .expect("stored state redacts"),
        questions,
        timeout: TIMEOUT,
    }
}

fn base_questions(row: &JevRecord) -> Vec<JevQuestion> {
    research_claim_questions(row.source_ids.as_ref().map_or(0, Vec::len))
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

#[allow(clippy::cast_precision_loss)] // small sample counts
fn spread(samples: &[f64]) -> f64 {
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    (samples.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / n).sqrt()
}

/// Five calls per stored row: the spread of each answer and how often the
/// 0.5 decision flips.
pub async fn noise(config: &Config, storage: &SqliteStorage, limit: usize) {
    let client = jev(config);
    let (mut spreads, mut flips, mut pairs, mut failures) = (Vec::new(), 0_usize, 0_usize, 0_usize);
    for row in research_rows(storage, limit).await {
        let req = request(config, &row, base_questions(&row));
        let mut runs: Vec<Vec<(String, f64)>> = Vec::new();
        for _ in 0..5 {
            match client.classify(&req).await {
                Ok(response) => runs.push(values(&response)),
                Err(_) => failures += 1,
            }
        }
        let Some(first) = runs.first() else { continue };
        for (id, _) in first {
            let samples: Vec<f64> = runs
                .iter()
                .filter_map(|run| run.iter().find(|(q, _)| q == id).map(|(_, p)| *p))
                .collect();
            pairs += 1;
            spreads.push(spread(&samples));
            let yes = samples.iter().filter(|p| **p >= 0.5).count();
            if yes != 0 && yes != samples.len() {
                flips += 1;
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let mean_spread = spreads.iter().sum::<f64>() / spreads.len().max(1) as f64;
    println!("question-row pairs: {pairs}; failed calls: {failures}");
    println!("mean standard deviation: {mean_spread:.4}");
    println!("decisions that flipped across 5 calls: {flips}");
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

/// Each stored row under all six option orders: flips against the fixed
/// order, and the `McNemar` statistic on the paired `supports` decisions.
pub async fn order(config: &Config, storage: &SqliteStorage, limit: usize) {
    let client = jev(config);
    let rows = research_rows(storage, limit).await;
    let all = orders();
    let mut base: Vec<Vec<(String, bool)>> = Vec::new();
    for row in &rows {
        let response = client
            .classify(&request(config, row, base_questions(row)))
            .await;
        base.push(response.map(|r| supports_chosen(&r)).unwrap_or_default());
    }
    for order in all.iter().skip(1) {
        let (mut b, mut c, mut compared) = (0_u32, 0_u32, 0_u32);
        for (row, fixed) in rows.iter().zip(&base) {
            let questions = reordered(&base_questions(row), order);
            let Ok(response) = client.classify(&request(config, row, questions)).await else {
                continue;
            };
            for (id, now) in supports_chosen(&response) {
                if let Some((_, before)) = fixed.iter().find(|(q, _)| *q == id) {
                    compared += 1;
                    match (before, now) {
                        (true, false) => b += 1,
                        (false, true) => c += 1,
                        _ => {}
                    }
                }
            }
        }
        let statistic = if b + c == 0 {
            0.0
        } else {
            let diff = f64::from(b.abs_diff(c)) - 1.0;
            diff.max(0.0).powi(2) / f64::from(b + c)
        };
        println!(
            "order {order:?}: {compared} decisions, {} flips, McNemar {statistic:.3}",
            b + c
        );
    }
}

/// The big model's own repeatability: the research verify run twice on each
/// stored claim with its stored excerpts.
pub async fn self_agreement(config: &Config, storage: &SqliteStorage, limit: usize) {
    let mut registry = ModeRegistry::new();
    verify::register(&mut registry, config.verify_ensemble_k).expect("register verify");
    let base_mode = registry
        .get(verify::VERIFY_ID)
        .expect("verify mode")
        .clone();
    let mode = mcp_parallax::research::pipeline::research_verify_mode(&base_mode);
    let client = default_model_client(config);
    let (mut agree, mut compared) = (0_u32, 0_u32);
    for row in research_rows(storage, limit).await {
        let state = row.state_json.as_ref().expect("filtered on state_json");
        let claim = state["claim"].as_str().unwrap_or_default().to_string();
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
            claim,
            context: Some(format!("Source excerpts:\n\n{}", excerpts.join("\n\n"))),
            effort: None,
            passes: None,
        };
        let first = verify::run(client.as_ref(), &mode, &params, config.input_max_chars).await;
        let second = verify::run(client.as_ref(), &mode, &params, config.input_max_chars).await;
        if let (Ok(a), Ok(b)) = (first, second) {
            compared += 1;
            if a.verdict.verdict == b.verdict.verdict {
                agree += 1;
            }
        }
    }
    println!("claims compared: {compared}; same verdict twice: {agree}");
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
            let req = JevRequest {
                model: config.jev.model.clone(),
                state,
                questions: vec![question.clone()],
                timeout: TIMEOUT,
            };
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
