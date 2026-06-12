//! S2 spike (006-checkpoint-layer, research.md D4): Voyage query-embed
//! latency from the dev machine — does the embedding lookup fit inside the
//! pre-action gate's budget (`GATE_BUDGET_MS` = 500 ms hard, SC-003 p95
//! < 150 ms)?
//!
//! Run: `VOYAGE_API_KEY=... cargo run --release --example spike_embed_latency`
//! Findings are recorded in specs/006-checkpoint-layer/research.md D4.

use mcp_parallax::client::VoyageClient;
use mcp_parallax::config::Config;
use mcp_parallax::traits::embedder::Embedder;
use std::time::Instant;

const SAMPLES: usize = 50;

#[allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let voyage_api_key = std::env::var("VOYAGE_API_KEY").expect("VOYAGE_API_KEY required");
    let config = Config {
        anthropic_api_key: "unused".into(),
        anthropic_model: "claude-opus-4-8".into(),
        verify_ensemble_k: 3,
        input_max_chars: 50_000,
        voyage_api_key: Some(voyage_api_key),
        voyage_model: "voyage-4".into(),
        memory_recall_limit: 5,
        brave_api_key: None,
        fetch_timeout_ms: 10_000,
        research_concurrency: 8,
        fetch_allow_private: false,
        checkpoint_gate_patterns: vec![],
        database_path: ":memory:".into(),
        log_level: "info".into(),
        request_timeout_ms: 30_000,
        max_retries: 1,
    };
    let client = VoyageClient::new(&config).expect("client");

    // Realistic gate queries: short action texts.
    let queries: Vec<String> = (0..SAMPLES)
        .map(|i| format!("bash git push origin main --force # sample {i}"))
        .collect();

    let mut latencies_ms: Vec<u128> = Vec::with_capacity(SAMPLES);
    for query in &queries {
        let started = Instant::now();
        let embedding = client.embed_query(query).await.expect("embed");
        latencies_ms.push(started.elapsed().as_millis());
        assert!(!embedding.vector.is_empty());
    }

    latencies_ms.sort_unstable();
    let p = |q: f64| latencies_ms[((latencies_ms.len() as f64 * q) as usize).min(SAMPLES - 1)];
    println!("samples : {SAMPLES} sequential query embeds (voyage-4)");
    println!("min     : {} ms", latencies_ms[0]);
    println!("p50     : {} ms", p(0.50));
    println!("p90     : {} ms", p(0.90));
    println!("p95     : {} ms", p(0.95));
    println!("max     : {} ms", latencies_ms[SAMPLES - 1]);
    let budget_ok = latencies_ms[SAMPLES - 1] < 500;
    let sc003_ok = p(0.95) < 150;
    println!("fits GATE_BUDGET_MS (500 ms hard): {budget_ok}");
    println!("fits SC-003 p95 (< 150 ms)       : {sc003_ok}");
}
