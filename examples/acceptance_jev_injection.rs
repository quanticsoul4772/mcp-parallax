//! 065 T024 — live check of the Jev injection warning (SC-005). Real calls:
//! Jev, Exa and the research models. Results go to
//! `specs/065-jev-injection-warning/quickstart.md`.
//!
//! ```text
//! cargo run --release --example acceptance_jev_injection
//! ```
//!
//! Runs one quick research question with the injection site `on` and the
//! real `TypeSafe` client, writes the rows and an invocation record the way
//! the server's record guard does, then asserts SC-005 from the result and
//! the stored rows and exits non-zero on any failure.

#![allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::typesafe::TypesafeClient;
use mcp_parallax::client::{default_model_client, ExaClient};
use mcp_parallax::config::{Config, DEFAULT_JEV_MODEL};
use mcp_parallax::error::Outcome;
use mcp_parallax::jev::shadow::InjectionJev;
use mcp_parallax::modes::verify::VERIFY_ID;
use mcp_parallax::modes::{verify, ModeRegistry};
use mcp_parallax::research::contract::ResearchParams;
use mcp_parallax::research::pipeline::{self, ResearchDeps};
use mcp_parallax::research::Depth;
use mcp_parallax::storage::jev::{JevOutcome, JevRecord};
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::telemetry::{InvocationRecord, JevShadow, NonModelUsage, PageCounts, SearchCost};
use mcp_parallax::traits::classifier::{Classifier, JevMode, JevSite};
use mcp_parallax::traits::clock::{SystemClock, TimeProvider};
use mcp_parallax::traits::storage::Storage as _;
use serde_json::Value;
use std::sync::Arc;

/// SC-005's checks on one run: the result as JSON and the run's stored
/// injection rows. Returns every failure found.
fn check(result: &Value, rows: &[JevRecord], invocation_id: &str) -> Vec<String> {
    let mut failures = Vec::new();
    let kept = result["stats"]["sources_kept"].as_u64().unwrap_or(0);
    let injection: Vec<&JevRecord> = rows
        .iter()
        .filter(|r| r.site == JevSite::ResearchInjection)
        .collect();
    if u64::try_from(injection.len()).unwrap_or(u64::MAX) != kept {
        failures.push(format!(
            "{} injection rows for {kept} kept pages",
            injection.len()
        ));
    }
    for row in &injection {
        if row.invocation_id.as_deref() != Some(invocation_id) {
            failures.push(format!("row {} not joined to the invocation", row.id));
        }
        if row.outcome == JevOutcome::Call(Outcome::Success)
            && row.model_answered.as_deref() != Some(DEFAULT_JEV_MODEL)
        {
            failures.push(format!(
                "row {} answered by {:?}",
                row.id, row.model_answered
            ));
        }
    }
    for field in ["jev_warned_pages", "jev_unscreened_pages"] {
        if !result["stats"][field].is_u64() {
            failures.push(format!("stats.{field} missing"));
        }
    }
    for source in result["sources"].as_array().into_iter().flatten() {
        let screen = &source["jev_injection"];
        let screened = screen["screened"].as_bool();
        let explained = screened == Some(true) || screen["reason"].is_string();
        if !explained {
            failures.push(format!(
                "source {} neither screened nor given a reason",
                source["id"]
            ));
        }
    }
    failures
}

#[tokio::main]
async fn main() {
    let config = Config::from_env().expect("config");
    let storage = SqliteStorage::connect(&config.database_path)
        .await
        .expect("database");
    let classifier: Arc<dyn Classifier> =
        Arc::new(TypesafeClient::new(&config.jev).expect("TYPESAFE_API_KEY present"));
    let mut registry = ModeRegistry::new();
    verify::register(&mut registry, config.verify_ensemble_k).unwrap();
    pipeline::register(&mut registry).unwrap();
    let verify_mode = registry.get(VERIFY_ID).unwrap().clone();
    let deps = ResearchDeps {
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
        jev: None,
        jev_injection: Some(InjectionJev::new(
            classifier,
            DEFAULT_JEV_MODEL.into(),
            "acceptance".into(),
            JevMode::On,
        )),
    };
    let rows = JevShadow::default();
    let started = SystemClock.now();
    let (result, usage) = pipeline::run_at(
        &deps,
        &ResearchParams {
            question: "What is indirect prompt injection in LLM applications?".into(),
            depth: Some(Depth::Quick),
            focus: None,
            constraints: None,
        },
        deps.concurrency,
        &SearchCost::default(),
        &PageCounts::default(),
        &NonModelUsage::default(),
        &rows,
    )
    .await
    .expect("research run");

    // Link and store the rows the way the server's record guard does.
    let mut run_rows = rows.take();
    let invocation_id = uuid::Uuid::new_v4().to_string();
    for row in &mut run_rows {
        row.invocation_id = Some(invocation_id.clone());
    }
    let jev_cost: f64 = run_rows.iter().filter_map(|r| r.cost_usd).sum();
    let record = InvocationRecord::create(
        &SystemClock,
        "acceptance",
        "research",
        config.default_model(),
        &usage,
        Outcome::Success,
        started,
    )
    .with_id(invocation_id.clone())
    .with_jev_cost(Some(jev_cost));
    storage.record_invocation(&record).await.unwrap();
    storage.record_jev(&run_rows).await.unwrap();
    let stored: Vec<JevRecord> = storage
        .load_jev_records()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.invocation_id.as_deref() == Some(invocation_id.as_str()))
        .collect();

    let value = serde_json::to_value(&result).unwrap();
    println!(
        "kept {}, warned {}, unscreened {}, Jev cost ${jev_cost:.6}",
        value["stats"]["sources_kept"],
        value["stats"]["jev_warned_pages"],
        value["stats"]["jev_unscreened_pages"]
    );
    let failures = check(&value, &stored, &invocation_id);
    if failures.is_empty() {
        println!("SC-005: pass");
    } else {
        for failure in &failures {
            println!("FAIL: {failure}");
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::check;
    use mcp_parallax::error::Outcome;
    use mcp_parallax::storage::jev::{JevOutcome, JevRecord};
    use mcp_parallax::traits::classifier::{JevMode, JevSite};
    use serde_json::json;

    fn row(id: &str, invocation: &str, answered: Option<&str>) -> JevRecord {
        JevRecord {
            id: id.into(),
            created_at: chrono::Utc::now(),
            server_session_id: "s".into(),
            harness_session_id: None,
            site: JevSite::ResearchInjection,
            mode: JevMode::On,
            questions_json: json!([]),
            model_requested: "jev-1.14.0".into(),
            model_answered: answered.map(String::from),
            outcome: JevOutcome::Call(Outcome::Success),
            error_class: None,
            request_id: None,
            latency_ms: None,
            input_tokens: None,
            cost_usd: None,
            eval_id: None,
            invocation_id: Some(invocation.into()),
            claim_key: None,
            source_ids: Some(vec![id.into()]),
            label_json: None,
            label_source: None,
            state_json: None,
            state_hash: None,
            transcript_ref: None,
            redaction_count: 0,
            truncated: false,
            applied: false,
        }
    }

    #[test]
    fn a_complete_run_passes_and_each_gap_is_named() {
        let result = json!({
            "stats": { "sources_kept": 2, "jev_warned_pages": 1, "jev_unscreened_pages": 0 },
            "sources": [
                { "id": "s1", "jev_injection": { "screened": true, "reason": null } },
                { "id": "s2", "jev_injection": { "screened": false, "reason": "timeout" } }
            ]
        });
        let rows = [
            row("s1", "inv", Some("jev-1.14.0")),
            row("s2", "inv", Some("jev-1.14.0")),
        ];
        assert!(check(&result, &rows, "inv").is_empty());

        let bad = json!({
            "stats": { "sources_kept": 3 },
            "sources": [{ "id": "s1", "jev_injection": { "screened": false, "reason": null } }]
        });
        let bad_rows = [row("s1", "other", Some("jev-1.13.0"))];
        let failures = check(&bad, &bad_rows, "inv");
        assert_eq!(failures.len(), 6, "{failures:?}");
    }
}
