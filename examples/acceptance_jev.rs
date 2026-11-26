//! 064 T034 — live shadow check (SC-004). Real calls: Jev, the review model,
//! Exa and the research models. Results go to
//! `specs/064-jev-first-pass/quickstart.md` §6.
//!
//! ```text
//! PARALLAX_JEV_MODE_TURN_REVIEW=shadow PARALLAX_JEV_MODE_RESEARCH_CLAIMS=shadow \
//!   cargo run --release --example acceptance_jev
//! ```
//!
//! Drives six real end-of-turn reviews on transcripts it writes to a temp
//! folder (one carries a planted secret) and one standard research question,
//! with the real `TypeSafe` client wrapped in a recorder that keeps every state
//! sent. Then asserts SC-004 from the stored records and exits non-zero on any
//! failure. The research rows are linked to an invocation record written here
//! the way the server's record guard does it.

#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

use mcp_parallax::checkpoint::contract::CheckpointTurnParams;
use mcp_parallax::checkpoint::review;
use mcp_parallax::checkpoint::run::{run_turn, CheckpointDeps};
use mcp_parallax::client::typesafe::{TypesafeClient, JEV_CLAIM_DEADLINE_MS, JEV_TURN_DEADLINE_MS};
use mcp_parallax::client::{default_model_client, ExaClient};
use mcp_parallax::config::Config;
use mcp_parallax::error::Outcome;
use mcp_parallax::jev::redact::Redactor;
use mcp_parallax::jev::shadow::{ClaimJev, TurnJev};
use mcp_parallax::modes::verify::VERIFY_ID;
use mcp_parallax::modes::{verify, ModeRegistry};
use mcp_parallax::research::contract::ResearchParams;
use mcp_parallax::research::pipeline::{self, ResearchDeps};
use mcp_parallax::research::Depth;
use mcp_parallax::storage::jev::JevOutcome;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::telemetry::{InvocationRecord, JevShadow, NonModelUsage, PageCounts, SearchCost};
use mcp_parallax::traits::classifier::{Classifier, JevMode, JevRequest, JevResponse, JevSite};
use mcp_parallax::traits::clock::{SystemClock, TimeProvider};
use mcp_parallax::traits::storage::Storage as _;
use mcp_parallax::traits::trajectory::FsTrajectoryReader;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The real client, keeping a copy of every state it sends.
struct Recorder {
    inner: TypesafeClient,
    sent: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Classifier for Recorder {
    async fn classify(
        &self,
        request: &JevRequest,
    ) -> Result<JevResponse, mcp_parallax::error::AppError> {
        self.sent
            .lock()
            .unwrap()
            .push(request.state.as_str().to_string());
        self.inner.classify(request).await
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
}

/// An earlier committed statement and a final message that reverses it.
const REVERSALS: [(&str, &str); 6] = [
    (
        "The database migration is fully reversible and safe to run.",
        "After checking, the database migration is not reversible after all.",
    ),
    (
        "The cache layer is enabled for write operations.",
        "On review, the cache layer is not enabled for write operations.",
    ),
    (
        "The new parser is thread safe for concurrent use.",
        "Testing showed the new parser is not thread safe for concurrent use.",
    ),
    (
        "The export job is idempotent and safe to retry.",
        "The export job is not idempotent, so it is not safe to retry.",
    ),
    (
        "The config loader validates every field at startup.",
        "Looking again, the config loader does not validate every field at startup.",
    ),
    (
        "The staging rollout is complete and the deploy token SECRET works.",
        "The staging rollout is not complete yet.",
    ),
];

fn write_transcript(dir: &std::path::Path, session: &str, earlier: &str) -> String {
    let path = dir.join(format!("{session}.jsonl"));
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(
        file,
        "{}",
        json!({ "type": "assistant", "sessionId": session,
                "message": { "role": "assistant", "content": [{ "type": "text", "text": earlier }] } })
    )
    .unwrap();
    path.to_string_lossy().to_string()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let config = Config::from_env().expect("configuration");
    assert_eq!(
        (config.jev.turn_review, config.jev.research_claims),
        (JevMode::Shadow, JevMode::Shadow),
        "set PARALLAX_JEV_MODE_TURN_REVIEW=shadow and PARALLAX_JEV_MODE_RESEARCH_CLAIMS=shadow"
    );
    assert!(config.exa_api_key.is_some(), "EXA_API_KEY is required");
    let pinned = config.jev.model.clone();
    // A file, not memory: the harness report (T039) reads these rows later.
    // The file keeps earlier runs' rows, so SC-004 checks only this run's.
    let run_started = SystemClock.now();
    std::fs::create_dir_all("data").unwrap();
    let storage = Arc::new(
        SqliteStorage::connect("./data/jev-acceptance.db")
            .await
            .unwrap(),
    );
    let recorder = Arc::new(Recorder {
        inner: TypesafeClient::new(&config.jev).expect("TYPESAFE_API_KEY"),
        sent: Mutex::new(Vec::new()),
    });
    let secret = ["sk-", "acceptance", "Planted", "Secret", "77"].concat();
    let redactor = Redactor::new(vec![secret.clone()]);

    // ---- end-of-turn reviews ----------------------------------------------
    let mut registry = ModeRegistry::new();
    review::register(&mut registry).unwrap();
    let turn_deps = CheckpointDeps {
        reader: Arc::new(FsTrajectoryReader),
        storage: Arc::clone(&storage) as Arc<dyn mcp_parallax::traits::storage::Storage>,
        clock: Arc::new(SystemClock),
        model_client: default_model_client(&config),
        review_mode: registry.get(review::REVIEW_MODE_ID).unwrap().clone(),
        model: config.default_model().to_string(),
        embedder: None,
        gate_extra_patterns: vec![],
        jev: Some(TurnJev {
            classifier: Arc::clone(&recorder) as Arc<dyn Classifier>,
            redactor: redactor.clone(),
            model: pinned.clone(),
            server_session_id: "acceptance".into(),
            deadline: Duration::from_millis(JEV_TURN_DEADLINE_MS),
        }),
        turn_signals: None,
        skip_thresholds: None,
        fixed_eval_id: None,
    };
    let dir = tempfile::tempdir().unwrap();
    for (i, (earlier, final_message)) in REVERSALS.iter().enumerate() {
        let session = format!("acceptance-{i}");
        let earlier = earlier.replace("SECRET", &secret);
        let path = write_transcript(dir.path(), &session, &earlier);
        let params = CheckpointTurnParams {
            session_id: session,
            transcript_path: path,
            final_message: (*final_message).to_string(),
            continuation: false,
        };
        let (result, _, _) = run_turn(&turn_deps, &params, None).await.unwrap();
        println!("turn {i}: verdict {:?}", result.verdict);
    }

    // ---- one standard research question -----------------------------------
    let mut registry = ModeRegistry::new();
    verify::register(&mut registry, config.verify_ensemble_k).unwrap();
    pipeline::register(&mut registry).unwrap();
    let verify_mode = registry.get(VERIFY_ID).unwrap().clone();
    let research_deps = ResearchDeps {
        scope_client: default_model_client(&config),
        extract_client: default_model_client(&config),
        verify_client: default_model_client(&config),
        synth_client: default_model_client(&config),
        search: Arc::new(ExaClient::new(&config).expect("exa key present")),
        reranker: config.voyage_api_key.as_ref().map(|_| {
            Arc::new(mcp_parallax::client::VoyageClient::new(&config).expect("voyage key"))
                as Arc<dyn mcp_parallax::traits::rerank::Reranker>
        }),
        clock: Arc::new(SystemClock),
        scope_mode: registry.get(pipeline::SCOPE_MODE_ID).unwrap().clone(),
        extract_mode: registry.get(pipeline::EXTRACT_MODE_ID).unwrap().clone(),
        synth_mode: registry.get(pipeline::SYNTH_MODE_ID).unwrap().clone(),
        verify_mode: pipeline::research_verify_mode(&verify_mode),
        input_max_chars: config.input_max_chars,
        concurrency: usize::from(config.research_concurrency),
        max_age_hours: config.exa_max_age_hours,
        routing: config.routing.clone(),
        backend: config.backend,
        jev: Some(ClaimJev {
            classifier: Arc::clone(&recorder) as Arc<dyn Classifier>,
            redactor: redactor.clone(),
            model: pinned.clone(),
            server_session_id: "acceptance".into(),
            deadline: Duration::from_millis(JEV_CLAIM_DEADLINE_MS),
        }),
        // 064's acceptance covers the claim site only; 065 has its own example.
        jev_injection: None,
    };
    let rows = JevShadow::default();
    let started = SystemClock.now();
    let (result, usage) = pipeline::run_at(
        &research_deps,
        &ResearchParams {
            question: "How does Rust's borrow checker prevent data races between threads?".into(),
            depth: Some(Depth::Standard),
            focus: None,
            constraints: None,
        },
        research_deps.concurrency,
        &SearchCost::default(),
        &PageCounts::default(),
        &NonModelUsage::default(),
        &rows,
    )
    .await
    .expect("research run");
    println!(
        "research: {} findings, {} claims verified",
        result.key_findings.len(),
        result.stats.claims_verified
    );
    // Link the rows the way the server's record guard does.
    let mut research_rows = rows.take();
    let invocation_id = uuid::Uuid::new_v4().to_string();
    for row in &mut research_rows {
        row.invocation_id = Some(invocation_id.clone());
    }
    let jev_cost: f64 = research_rows.iter().filter_map(|r| r.cost_usd).sum();
    let record = InvocationRecord::create(
        &SystemClock,
        "acceptance",
        "research",
        config.default_model(),
        &usage,
        Outcome::Success,
        started,
    )
    .with_id(invocation_id)
    .with_jev_cost(Some(jev_cost));
    storage.record_invocation(&record).await.unwrap();
    storage.record_jev(&research_rows).await.unwrap();

    // ---- SC-004 -------------------------------------------------------------
    let all = this_run(
        storage.load_jev_records().await.unwrap(),
        run_started,
        |r| r.created_at,
    );
    let checkpoint_ids: Vec<String> = storage
        .list_checkpoints()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    let invocation_ids: Vec<String> = storage
        .list_invocations()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    let turn: Vec<_> = all
        .iter()
        .filter(|r| r.site == JevSite::TurnReview)
        .collect();
    let claims: Vec<_> = all
        .iter()
        .filter(|r| r.site == JevSite::ResearchClaims)
        .collect();

    let mut outcomes: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for row in &all {
        *outcomes
            .entry((row.site.as_str(), row.outcome.as_str()))
            .or_default() += 1;
    }
    let excluded = claims
        .iter()
        .filter(|r| r.outcome == JevOutcome::Excluded)
        .count();
    println!(
        "rows: turn_review {}, research_claims {}",
        turn.len(),
        claims.len()
    );
    for ((site, outcome), count) in &outcomes {
        println!("  {site} {outcome}: {count}");
    }
    #[allow(clippy::cast_precision_loss)]
    let share = excluded as f64 / claims.len().max(1) as f64;
    println!("excluded share of research claims: {share:.2}");
    for (name, rows) in [("turn_review", &turn), ("research_claims", &claims)] {
        let latencies: Vec<u64> = rows.iter().filter_map(|r| r.latency_ms).collect();
        let max = latencies.iter().max().copied().unwrap_or(0);
        println!(
            "{name}: Jev latency max {max} ms over {} calls; critical path {}",
            latencies.len(),
            if name == "research_claims" {
                "0 ms by design (never longer than verify)"
            } else {
                "bounded by the 1000 ms turn deadline"
            }
        );
    }

    assert!(turn.len() >= 5, "only {} turn-review rows", turn.len());
    assert!(
        claims.len() >= 10,
        "only {} research-claim rows",
        claims.len()
    );
    for row in &turn {
        let id = row.eval_id.as_deref().expect("turn row has an eval_id");
        assert!(
            checkpoint_ids.iter().any(|c| c == id),
            "turn row joins no checkpoint record"
        );
    }
    for row in &claims {
        let id = row
            .invocation_id
            .as_deref()
            .expect("claim row has an invocation_id");
        assert!(
            invocation_ids.iter().any(|i| i == id),
            "claim row joins no invocation record"
        );
    }
    for row in &all {
        assert_eq!(
            JevOutcome::parse(row.outcome.as_str()),
            Some(row.outcome),
            "unbound outcome"
        );
        if let Some(answered) = &row.model_answered {
            assert_eq!(answered, &pinned, "answered by another model");
        }
        if row.outcome == JevOutcome::Call(Outcome::Success) {
            assert!(
                row.model_answered.is_some(),
                "a successful row has no answering model"
            );
        }
        if row.outcome == JevOutcome::Excluded {
            assert!(
                row.request_id.is_none()
                    && row.latency_ms.is_none()
                    && row.model_answered.is_none(),
                "an excluded claim has request details"
            );
        }
    }
    let sent: Vec<String> = recorder.sent.lock().unwrap().clone();
    assert!(
        sent.iter().all(|state| !state.contains(&secret)),
        "the planted secret reached Jev"
    );
    // `applied` is written as 0 by every insert (storage::jev); there is no
    // path that sets it in 064.
    println!(
        "SC-004: passed ({} states sent, none with the planted secret)",
        sent.len()
    );
}

/// The rows this run wrote: the database file keeps earlier runs' rows, and
/// SC-004 must not pass on them.
fn this_run<T>(
    rows: Vec<T>,
    since: chrono::DateTime<chrono::Utc>,
    created_at: impl Fn(&T) -> chrono::DateTime<chrono::Utc>,
) -> Vec<T> {
    rows.into_iter()
        .filter(|r| created_at(r) >= since)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::this_run;
    use chrono::{Duration, Utc};

    #[test]
    fn rows_from_earlier_runs_are_left_out() {
        let since = Utc::now();
        let rows = vec![
            ("old", since - Duration::hours(1)),
            ("new", since + Duration::seconds(1)),
        ];
        let kept = this_run(rows, since, |r| r.1);
        assert_eq!(kept.iter().map(|r| r.0).collect::<Vec<_>>(), ["new"]);
    }

    #[test]
    fn a_run_that_wrote_nothing_keeps_nothing() {
        let since = Utc::now();
        let rows = vec![("old", since - Duration::hours(1))];
        assert!(this_run(rows, since, |r| r.1).is_empty());
    }
}
