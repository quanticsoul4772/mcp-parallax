//! T019 — live acceptance pass for the research layer (manual-run, real spend).
//!
//! SC-001/SC-002: ≥6 live questions — every cited source id resolves, zero
//! fabricated citations, every response conforms to the typed structs.
//! SC-003: quick < 90 s, standard < 4 min. SC-004: a tiny-budget run returns
//! well-formed with `stopped_early`. SC-007: a false-premise question is not
//! confirmed. Results recorded in `specs/004-research-layer/quickstart.md`.
//!
//! Run: `ANTHROPIC_API_KEY=... BRAVE_API_KEY=... cargo run --release --example acceptance_research`

// Acceptance tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::{AnthropicClient, BraveClient};
use mcp_parallax::config::Config;
use mcp_parallax::modes::verify::VERIFY_ID;
use mcp_parallax::modes::{verify, ModeRegistry};
use mcp_parallax::research::contract::{Constraints, ResearchParams, ResearchResult};
use mcp_parallax::research::fetch::{FetchPolicy, HygieneFetcher, DOMAIN_SPACING_MS};
use mcp_parallax::research::pipeline::{self, ResearchDeps};
use mcp_parallax::research::Depth;
use mcp_parallax::traits::clock::SystemClock;
use std::sync::Arc;
use std::time::Instant;

// (question, depth) — the acceptance set. The last one carries a false
// premise (SC-007): Rust 1.0 shipped in 2015, not 2018.
const QUESTIONS: &[(&str, Depth)] = &[
    (
        "What is the default branch protection behavior for new GitHub repositories?",
        Depth::Quick,
    ),
    (
        "Which embedding model families does Voyage AI currently offer and how do they differ?",
        Depth::Quick,
    ),
    (
        "What are the documented rate limits of the Brave Search API free tier?",
        Depth::Quick,
    ),
    (
        "How does SQLite's WAL journaling mode differ from rollback journaling?",
        Depth::Standard,
    ),
    (
        "What is the Model Context Protocol and which transports does it define?",
        Depth::Standard,
    ),
    (
        "Why did Rust 1.0, released in 2018, remove the garbage collector it shipped with?",
        Depth::Quick,
    ),
];

fn deps(config: &Config) -> ResearchDeps {
    let mut registry = ModeRegistry::new();
    verify::register(&mut registry, config.verify_ensemble_k).expect("register verify");
    pipeline::register(&mut registry).expect("register research modes");
    let verify_mode = registry.get(VERIFY_ID).expect("verify mode").clone();
    ResearchDeps {
        model_client: Arc::new(AnthropicClient::new(config)),
        search: Arc::new(BraveClient::new(config).expect("brave key present")),
        clock: Arc::new(SystemClock),
        scope_mode: registry.get(pipeline::SCOPE_MODE_ID).unwrap().clone(),
        extract_mode: registry.get(pipeline::EXTRACT_MODE_ID).unwrap().clone(),
        synth_mode: registry.get(pipeline::SYNTH_MODE_ID).unwrap().clone(),
        verify_mode: pipeline::research_verify_mode(&verify_mode),
        input_max_chars: config.input_max_chars,
        concurrency: usize::from(config.research_concurrency),
    }
}

fn fetcher(config: &Config) -> HygieneFetcher {
    HygieneFetcher::new(FetchPolicy {
        timeout_ms: config.fetch_timeout_ms,
        domains_allow: vec![],
        domains_deny: vec![],
        domain_spacing_ms: DOMAIN_SPACING_MS,
    })
    .expect("fetcher")
}

/// SC-001: every finding source id and every `[sN]` in the answer resolves.
fn citations_resolve(result: &ResearchResult) -> bool {
    let listed: Vec<&str> = result.sources.iter().map(|s| s.id.as_str()).collect();
    let findings_ok = result
        .key_findings
        .iter()
        .all(|f| !f.sources.is_empty() && f.sources.iter().all(|id| listed.contains(&id.as_str())));
    let answer_ok = mcp_parallax::research::grounding::citation_tokens(&result.answer)
        .iter()
        .all(|id| listed.contains(&id.as_str()));
    findings_ok && answer_ok
}

#[tokio::main(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // a linear acceptance script reads best unsplit
async fn main() {
    let config = Config::from_env().expect("config (both keys required)");
    assert!(
        config.brave_api_key.is_some(),
        "BRAVE_API_KEY required for the acceptance run"
    );
    let deps = deps(&config);

    let (mut grounded, mut total) = (0_u32, 0_u32);
    let mut max_quick_ms = 0_u128;
    let mut max_standard_ms = 0_u128;
    let mut false_premise_confirmed = false;

    for (i, (question, depth)) in QUESTIONS.iter().enumerate() {
        let start = Instant::now();
        let outcome = pipeline::run(
            &deps,
            &fetcher(&config),
            &ResearchParams {
                question: (*question).to_string(),
                depth: Some(*depth),
                focus: None,
                constraints: None,
            },
        )
        .await;
        let elapsed = start.elapsed().as_millis();
        match *depth {
            Depth::Quick => max_quick_ms = max_quick_ms.max(elapsed),
            _ => max_standard_ms = max_standard_ms.max(elapsed),
        }

        match outcome {
            Ok((result, _, _)) => {
                total += 1;
                let ok = citations_resolve(&result);
                if ok {
                    grounded += 1;
                }
                // SC-007: the last question's premise (Rust 1.0 in 2018, with
                // a GC) must not be confirmed.
                if i == QUESTIONS.len() - 1 {
                    false_premise_confirmed = result.key_findings.iter().any(|f| {
                        f.support == mcp_parallax::research::Support::Confirmed
                            && f.claim.contains("2018")
                    }) && result.answer.contains("released in 2018");
                }
                println!(
                    "[{elapsed}ms]{} {question}\n   confidence {:.2}; {} findings, {} sources, \
                     {} gaps; stopped_early={} ({:?})\n   -> {}\n",
                    if ok { "" } else { " ** UNGROUNDED **" },
                    result.confidence,
                    result.key_findings.len(),
                    result.sources.len(),
                    result.gaps.len(),
                    result.stats.stopped_early,
                    result.stats.stop_reason,
                    result.answer.chars().take(220).collect::<String>()
                );
            }
            Err(e) => println!("[ERROR {elapsed}ms] {question}: {e}\n"),
        }
    }

    // SC-004: a deliberately tiny budget returns well-formed, stopped early.
    let (tiny, _, _) = pipeline::run(
        &deps,
        &fetcher(&config),
        &ResearchParams {
            question: "What is the capital of France?".to_string(),
            depth: Some(Depth::Quick),
            focus: None,
            constraints: Some(Constraints {
                budget_tokens: Some(1_000),
                ..Constraints::default()
            }),
        },
    )
    .await
    .expect("tiny-budget run must not error");
    let tiny_ok = tiny.stats.stopped_early && tiny.stats.stop_reason.is_some();
    println!(
        "tiny budget: stopped_early={} reason={:?}",
        tiny.stats.stopped_early, tiny.stats.stop_reason
    );

    println!("\n=== Acceptance summary ===");
    println!(
        "SC-001/002 grounded responses: {grounded}/{total} (target: all; typed structs end to end)"
    );
    println!("SC-003 max quick: {max_quick_ms} ms (target < 90000)   max standard: {max_standard_ms} ms (target < 240000)");
    println!("SC-004 tiny-budget honesty: {tiny_ok}");
    println!("SC-007 false premise confirmed: {false_premise_confirmed} (target: false)");

    let pass = grounded == total
        && usize::try_from(total).unwrap() == QUESTIONS.len()
        && max_quick_ms < 90_000
        && max_standard_ms < 240_000
        && tiny_ok
        && !false_premise_confirmed;
    println!("\nACCEPTANCE: {}", if pass { "PASS" } else { "FAIL" });
}
