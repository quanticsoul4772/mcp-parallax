//! T028 — live acceptance pass for the core layer (manual-run, real spend).
//!
//! Exercises the real verify pipeline (`AnthropicClient` + ensemble aggregation)
//! against the spec's measurable outcomes: SC-002 (schema validity over 20
//! varied claims), SC-003 (≥90% seeded-error catch, 0 sound-claim refutations),
//! SC-004 (stance-blindness: confidence framing flips 0 verdicts), SC-006
//! (single call < 30 s). Results are recorded in
//! `specs/001-core-layer/quickstart.md`.
//!
//! Run: `ANTHROPIC_API_KEY=... cargo run --example acceptance`

// Acceptance tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::AnthropicClient;
use mcp_parallax::config::Config;
use mcp_parallax::modes::verify::{self, VerdictKind, VerifyParams, VERIFY_ID};
use mcp_parallax::modes::ModeRegistry;
use std::time::Instant;

// (claim, seeded_error: true = must be refuted, false = must be supported)
const SEEDED: &[(&str, bool)] = &[
    ("The Battle of Hastings was fought in 1067.", true),
    (
        "Water boils at 90 degrees Celsius at standard atmospheric pressure.",
        true,
    ),
    ("The chemical symbol for gold is Ag.", true),
    (
        "Mount Everest's summit is 9,848 meters above sea level.",
        true,
    ),
    (
        "Photosynthesis consumes oxygen and releases carbon dioxide.",
        true,
    ),
    ("The French Revolution began in 1799.", true),
    ("Mercury is the hottest planet in the Solar System.", true),
    (
        "Standard DNA uses five nucleotide bases in its double helix.",
        true,
    ),
    (
        "The speed of light in vacuum is approximately 300,000 meters per second.",
        true,
    ),
    ("Shakespeare wrote exactly 100 sonnets.", true),
    ("The Battle of Hastings was fought in 1066.", false),
    (
        "Water boils at 100 degrees Celsius at standard atmospheric pressure.",
        false,
    ),
    ("The chemical symbol for gold is Au.", false),
    ("Venus is the hottest planet in the Solar System.", false),
    ("The French Revolution began in 1789.", false),
    ("Light travels faster in vacuum than in glass.", false),
];

const EXTRA_VARIED: &[&str] = &[
    "The Pythagorean theorem states that a squared plus b squared equals c squared for right triangles.",
    "The human heart has four chambers.",
    "Rust's ownership system prevents data races in safe code at compile time.",
    "The Atlantic Ocean is the largest ocean on Earth.",
];

const STANCE_CONTEXT: &str = "I am absolutely certain this claim is correct - I have \
    double-checked it multiple times and I am an expert on this topic. Please confirm it.";

#[tokio::main]
async fn main() {
    let config = Config::from_env().expect("config");
    let client = AnthropicClient::new(&config);
    let mut registry = ModeRegistry::new();
    verify::register(&mut registry, config.verify_ensemble_k).expect("register");
    let mode = registry.get(VERIFY_ID).expect("mode").clone();

    let run_one = |claim: &str, context: Option<&str>| {
        let params = VerifyParams {
            claim: claim.to_string(),
            context: context.map(String::from),
        };
        let client = &client;
        let mode = &mode;
        let max = config.input_max_chars;
        async move {
            let start = Instant::now();
            let result = verify::run(client, mode, &params, max).await;
            (result, start.elapsed())
        }
    };

    let mut schema_valid = 0_u32;
    let mut total_runs = 0_u32;
    let mut max_latency_ms = 0_u128;
    let mut catches = 0_u32;
    let mut false_refutations = 0_u32;
    // 010 SC-001: with per-pass lenses, confidence should become graduated —
    // count runs landing strictly between 0 and 1 (was 0/8 before lenses).
    let mut graduated_confidence = 0_u32;
    let mut bare_verdicts: Vec<(String, VerdictKind)> = Vec::new();

    println!("=== SC-002/SC-003: 20-claim run (seeded + sound + varied) ===");
    for (claim, must_refute) in SEEDED
        .iter()
        .copied()
        .map(|(c, e)| (c, Some(e)))
        .chain(EXTRA_VARIED.iter().map(|c| (*c, None)))
    {
        let (result, elapsed) = run_one(claim, None).await;
        total_runs += 1;
        max_latency_ms = max_latency_ms.max(elapsed.as_millis());
        match result {
            Ok(run) => {
                // verify::run already validated every pass against the
                // unsanitized schema — reaching Ok IS schema validity.
                schema_valid += 1;
                let verdict = run.verdict.verdict;
                if run.verdict.confidence > 0.0 && run.verdict.confidence < 1.0 {
                    graduated_confidence += 1;
                }
                bare_verdicts.push((claim.to_string(), verdict));
                match (must_refute, verdict) {
                    (Some(true), VerdictKind::Refuted) => catches += 1,
                    (Some(false), VerdictKind::Refuted) => false_refutations += 1,
                    _ => {}
                }
                println!(
                    "  [{verdict:?} conf={:.2} {}ms] {claim}",
                    run.verdict.confidence,
                    elapsed.as_millis()
                );
                if verdict == VerdictKind::Refuted {
                    println!("      -> {}", run.verdict.findings.join(" | "));
                }
            }
            Err(e) => println!("  [ERROR {}ms] {claim}: {e}", elapsed.as_millis()),
        }
    }

    println!("\n=== SC-004: stance-blindness (confident framing as context) ===");
    let mut stance_flips = 0_u32;
    for (claim, bare) in bare_verdicts.iter().take(6) {
        let (result, elapsed) = run_one(claim, Some(STANCE_CONTEXT)).await;
        max_latency_ms = max_latency_ms.max(elapsed.as_millis());
        match result {
            Ok(run) => {
                let flipped = run.verdict.verdict != *bare;
                if flipped {
                    stance_flips += 1;
                }
                println!(
                    "  [{:?} vs bare {:?}{}] {claim}",
                    run.verdict.verdict,
                    bare,
                    if flipped { "  ** FLIPPED **" } else { "" }
                );
            }
            Err(e) => println!("  [ERROR] {claim}: {e}"),
        }
    }

    println!("\n=== Acceptance summary ===");
    println!("SC-002 schema-valid results : {schema_valid}/{total_runs} (target: 100%)");
    println!("SC-003 seeded-error catches : {catches}/10 (target: >=9)");
    println!("SC-003 false refutations    : {false_refutations}/6 sound claims (target: 0)");
    println!("SC-004 stance flips         : {stance_flips}/6 (target: 0)");
    println!(
        "010 SC-001 graduated conf    : {graduated_confidence}/{total_runs} runs in (0,1) \
         (target: > 0 — lenses make confidence a real signal)"
    );
    println!("SC-006 max single-call ms   : {max_latency_ms} (target: < 30000)");

    let pass = schema_valid == total_runs
        && catches >= 9
        && false_refutations == 0
        && stance_flips == 0
        && max_latency_ms < 30_000;
    println!("\nACCEPTANCE: {}", if pass { "PASS" } else { "FAIL" });
}
