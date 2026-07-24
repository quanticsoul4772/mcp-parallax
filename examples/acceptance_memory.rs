//! T016 — live acceptance pass for the memory layer (manual-run, real spend).
//!
//! SC-001: 12 saved memories, 10 paraphrased recall queries — intended memory
//! in the top 3 for ≥9/10 and top 1 for ≥7/10. SC-003: trust scenarios
//! (unverified external surfaces untrusted; refuted external save rejected
//! with findings). SC-004: recall < 5 s, unverified save < 10 s. Results
//! recorded in `specs/003-memory-layer/quickstart.md`.
//!
//! Run: `ANTHROPIC_API_KEY=... VOYAGE_API_KEY=... cargo run --example acceptance_memory`

// Acceptance tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::{AnthropicClient, VoyageClient};
use mcp_parallax::config::Config;
use mcp_parallax::memory::tools::{self, MemoryDeps, RecallParams, SaveParams};
use mcp_parallax::memory::Kind;
use mcp_parallax::modes::verify::VERIFY_ID;
use mcp_parallax::modes::{verify, ModeRegistry};
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::clock::SystemClock;
use std::sync::Arc;
use std::time::Instant;

// (content, kind) — the acceptance corpus, deliberately heterogeneous.
const MEMORIES: &[(&str, Kind)] = &[
    (
        "When CI passes locally but fails remotely, diff the toolchain versions first: \
         pin rust-toolchain.toml and check the runner image date before debugging the code.",
        Kind::Lesson,
    ),
    (
        "To keep stdout clean in an MCP stdio server, route all logging to stderr and \
         deny print_stdout at the lint level so violations fail the build.",
        Kind::Skill,
    ),
    (
        "sqlx SQLite pools default to multiple connections; for :memory: databases use \
         max_connections(1) or each connection sees its own empty database.",
        Kind::Fact,
    ),
    (
        "Brute-force cosine similarity over 5,000 1024-dim f32 vectors takes about 3 ms \
         in release mode, so small stores need no vector index.",
        Kind::Fact,
    ),
    (
        "Exponential backoff with a doubling base and a cap at 2^8 multiples avoids \
         thundering-herd retries while keeping worst-case wait bounded.",
        Kind::Skill,
    ),
    (
        "Never store an external claim as trusted without an independent verification \
         pass; poisoning enters through unverified web content.",
        Kind::Lesson,
    ),
    (
        "Windows file paths break URL-style SQLite connection strings; pass the path \
         through filename() which takes it verbatim.",
        Kind::Lesson,
    ),
    (
        "Voyage embeddings are asymmetric: documents must embed with input_type \
         document and queries with input_type query or retrieval quality degrades.",
        Kind::Fact,
    ),
    (
        "A derive macro that rebuilds state per call can silently undo construction-time \
         configuration; prefer wiring the instance field explicitly.",
        Kind::Lesson,
    ),
    (
        "For reproducible pseudo-random test vectors without a rand dependency, use a \
         64-bit LCG with the MMIX constants and map the top 24 bits to [-1, 1).",
        Kind::Skill,
    ),
    (
        "Anthropic structured outputs drop numeric range and length constraints from \
         schemas, so re-impose them with a local validator after the response.",
        Kind::Fact,
    ),
    (
        "Idempotent migrations (CREATE TABLE IF NOT EXISTS) let the same migration run \
         at every startup without version tracking for small schemas.",
        Kind::Skill,
    ),
];

// (paraphrased query, index of the intended memory above)
const QUERIES: &[(&str, usize)] = &[
    ("build green on my machine, red on the build server", 0),
    ("how do I stop log lines corrupting the JSON-RPC channel", 1),
    (
        "in-memory sqlite looks empty from a second pool connection",
        2,
    ),
    ("do I need an ANN index for a few thousand embeddings", 3),
    ("retry strategy that won't hammer a recovering service", 4),
    ("is it safe to remember things scraped from a website", 5),
    ("sqlite open fails on a path with backslashes", 6),
    (
        "does it matter which input_type I embed search text with",
        7,
    ),
    ("config set in the constructor seems ignored at runtime", 8),
    (
        "deterministic fake random numbers for tests without a crate",
        9,
    ),
];

async fn deps(config: &Config, db_path: &str) -> MemoryDeps {
    let mut registry = ModeRegistry::new();
    verify::register(&mut registry, config.verify_ensemble_k).expect("register");
    mcp_parallax::memory::consolidate::register(&mut registry).expect("register consolidation");
    let verify_mode = registry.get(VERIFY_ID).expect("mode").clone();
    let consolidation_mode = registry
        .get(mcp_parallax::memory::consolidate::CONSOLIDATION_MODE_ID)
        .expect("mode")
        .clone();
    let storage = SqliteStorage::connect(db_path).await.expect("store");
    MemoryDeps {
        embedder: Arc::new(VoyageClient::new(config).expect("voyage key present")),
        storage: Arc::new(storage),
        clock: Arc::new(SystemClock),
        model_client: Arc::new(AnthropicClient::new(config)),
        verify_mode,
        consolidation_mode,
        input_max_chars: config.input_max_chars,
        default_recall_limit: config.memory_recall_limit,
    }
}

#[tokio::main(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // a linear acceptance script reads best unsplit
async fn main() {
    let config = Config::from_env().expect("config (both keys required)");
    assert!(
        config.voyage_api_key.is_some(),
        "VOYAGE_API_KEY required for the acceptance run"
    );
    let db = std::env::temp_dir().join(format!("parallax-accept-{}.db", uuid::Uuid::new_v4()));
    let db_path = db.to_string_lossy().to_string();
    let deps = deps(&config, &db_path).await;

    // ---- SC-001 setup + SC-004 save latency --------------------------------
    let mut ids = Vec::new();
    let mut max_save_ms = 0_u128;
    for (content, kind) in MEMORIES {
        let start = Instant::now();
        let (saved, _, _) = tools::save(
            &deps,
            &SaveParams {
                content: (*content).to_string(),
                kind: *kind,
                origin: "acceptance run".into(),
                external: false,
                tags: None,
                verify: None,
            },
        )
        .await
        .expect("save");
        max_save_ms = max_save_ms.max(start.elapsed().as_millis());
        ids.push(saved.id);
    }
    println!(
        "saved {} memories (max save {} ms)\n",
        ids.len(),
        max_save_ms
    );

    // ---- SC-001: paraphrased recall ----------------------------------------
    let (mut top1, mut top3) = (0_u32, 0_u32);
    let mut max_recall_ms = 0_u128;
    for (query, intended) in QUERIES {
        let start = Instant::now();
        let (result, _, _) = tools::recall(
            &deps,
            &RecallParams {
                query: (*query).to_string(),
                kind: None,
                limit: Some(3),
            },
        )
        .await
        .expect("recall");
        max_recall_ms = max_recall_ms.max(start.elapsed().as_millis());

        let rank = result.memories.iter().position(|m| m.id == ids[*intended]);
        match rank {
            Some(0) => {
                top1 += 1;
                top3 += 1;
                println!("[top-1] {query}");
            }
            Some(r) => {
                top3 += 1;
                println!("[top-{}] {query}", r + 1);
            }
            None => println!("[MISS ] {query}"),
        }
    }

    // ---- SC-003: trust scenarios --------------------------------------------
    // Unverified external save surfaces untrusted at recall.
    let (ext, _, _) = tools::save(
        &deps,
        &SaveParams {
            content: "External tip: cargo build --jobs 1 always halves compile time".into(),
            kind: Kind::Fact,
            origin: "random forum post".into(),
            external: true,
            tags: None,
            verify: None,
        },
    )
    .await
    .expect("external save");
    let (recalled, _, _) = tools::recall(
        &deps,
        &RecallParams {
            query: "compile time cargo jobs tip".into(),
            kind: None,
            limit: Some(3),
        },
    )
    .await
    .expect("recall external");
    let untrusted_labeled = recalled
        .memories
        .iter()
        .find(|m| m.id == ext.id)
        .is_some_and(|m| m.trust == mcp_parallax::memory::Trust::Untrusted);

    // Refuted external save is rejected with findings.
    let refuted = tools::save(
        &deps,
        &SaveParams {
            content: "The Apollo 11 mission landed on the Moon in 1972.".into(),
            kind: Kind::Fact,
            origin: "a blog".into(),
            external: true,
            tags: None,
            verify: Some(true),
        },
    )
    .await;
    let refuted_rejected = matches!(
        &refuted,
        Err(e) if e.to_string().contains("verification refuted")
    );
    if let Err(e) = &refuted {
        println!("\nrefuted save rejected as expected:\n  {e}");
    }

    // ---- Summary -------------------------------------------------------------
    println!("\n=== Acceptance summary ===");
    println!("SC-001 top-3: {top3}/10 (target ≥9)   top-1: {top1}/10 (target ≥7)");
    println!("SC-002 structure: all results deserialized through the typed structs");
    println!(
        "SC-003 untrusted labeled: {untrusted_labeled}   refuted rejected: {refuted_rejected}"
    );
    println!("SC-004 max recall: {max_recall_ms} ms (target < 5000)   max save: {max_save_ms} ms (target < 10000)");

    let pass = top3 >= 9
        && top1 >= 7
        && untrusted_labeled
        && refuted_rejected
        && max_recall_ms < 5_000
        && max_save_ms < 10_000;
    println!("\nACCEPTANCE: {}", if pass { "PASS" } else { "FAIL" });

    let _ = std::fs::remove_file(&db);
}
