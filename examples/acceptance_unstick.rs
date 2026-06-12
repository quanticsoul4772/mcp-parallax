//! T009 — live acceptance pass for unstick (manual-run, real spend).
//!
//! 10 varied stuck scenarios against the real pipeline: SC-002 (structural
//! validity), SC-003 (one-step shape, zero restatements of tried items),
//! SC-004 (per-call latency < 15 s). Results recorded in
//! `specs/002-unstick-mode/quickstart.md`.
//!
//! Run: `ANTHROPIC_API_KEY=... cargo run --example acceptance_unstick`

// Acceptance tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::AnthropicClient;
use mcp_parallax::config::Config;
use mcp_parallax::modes::unstick::{self, UnstickParams, UNSTICK_ID};
use mcp_parallax::modes::ModeRegistry;
use std::time::Instant;

// (goal, blocked, tried)
const SCENARIOS: &[(&str, &str, &[&str])] = &[
    (
        "Get the integration test suite passing on CI",
        "The same two tests fail on CI but pass locally; logs show no difference",
        &[
            "Re-running the CI job",
            "Pinning the toolchain version",
            "Adding debug logging",
        ],
    ),
    (
        "Reduce p99 latency of the search endpoint below 200ms",
        "Profiling shows no single hotspot; everything is uniformly a bit slow",
        &[
            "Adding a cache in front of the database",
            "Increasing instance size",
        ],
    ),
    (
        "Finish the introduction section of a research paper",
        "Rewritten the opening paragraph six times and none of them feel right",
        &[
            "Rewriting from scratch",
            "Reading other papers' introductions",
        ],
    ),
    (
        "Track down a memory leak in a long-running service",
        "Heap grows ~50MB/day in production but never under load tests",
        &[
            "Running the load test longer",
            "Reviewing recent diffs for leaks",
        ],
    ),
    (
        "Migrate the user table to the new schema without downtime",
        "Every migration plan I draft has a window where writes could be lost",
        &[],
    ),
    (
        "Get a flaky Bluetooth device pairing reliably with the app",
        "Pairing fails roughly 1 in 5 times with a generic timeout error",
        &["Increasing the timeout", "Retrying pairing automatically"],
    ),
    (
        "Decide on a name for the new open-source library",
        "Brainstormed 40 names; every candidate is taken on the package registry or feels wrong",
        &[
            "Brainstorming more names",
            "Asking teammates for suggestions",
        ],
    ),
    (
        "Reproduce a customer-reported crash that has no stack trace",
        "Only one customer hits it, on a device model we do not have",
        &[
            "Reading the crash report metadata",
            "Testing on similar devices",
        ],
    ),
    (
        "Bring the binary size of the release build under 5MB",
        "Stripped symbols and enabled LTO; still at 7.8MB and out of obvious ideas",
        &[
            "Stripping symbols",
            "Enabling LTO",
            "Removing unused dependencies",
        ],
    ),
    (
        "Write a regex that parses the legacy log format",
        "Every regex I write breaks on some line; the format seems inconsistent",
        &[
            "Writing increasingly complex regexes",
            "Looking for a format spec",
        ],
    ),
];

/// Soft one-step heuristics: markers that suggest an option menu or plan
/// leaked into the single string.
fn looks_like_menu_or_plan(step: &str) -> bool {
    let lower = step.to_lowercase();
    lower.contains(" or alternatively")
        || lower.contains("either ")
        || lower.contains("option 1")
        || lower.contains("option a")
        || lower.contains("1.") && lower.contains("2.")
        || lower.contains("first,") && lower.contains("then")
}

#[tokio::main]
async fn main() {
    let config = Config::from_env().expect("config");
    let client = AnthropicClient::new(&config);
    let mut registry = ModeRegistry::new();
    unstick::register(&mut registry).expect("register");
    let mode = registry.get(UNSTICK_ID).expect("mode").clone();

    let mut ok = 0_u32;
    let mut menus = 0_u32;
    let mut max_latency_ms = 0_u128;

    for (goal, blocked, tried) in SCENARIOS {
        let params = UnstickParams {
            goal: (*goal).to_string(),
            blocked: (*blocked).to_string(),
            tried: Some(tried.iter().map(|s| (*s).to_string()).collect()),
        };
        let start = Instant::now();
        let result = unstick::run(&client, &mode, &params, config.input_max_chars).await;
        let elapsed = start.elapsed();
        max_latency_ms = max_latency_ms.max(elapsed.as_millis());

        match result {
            Ok(run) => {
                // Reaching Ok already proves schema validity AND the
                // no-restatement rule (both are enforced in run()).
                ok += 1;
                let menu = looks_like_menu_or_plan(&run.step.next_step);
                if menu {
                    menus += 1;
                }
                println!(
                    "[{}ms{}] {goal}\n   -> {}\n",
                    elapsed.as_millis(),
                    if menu { " ** MENU? **" } else { "" },
                    run.step.next_step
                );
            }
            Err(e) => println!("[ERROR {}ms] {goal}: {e}\n", elapsed.as_millis()),
        }
    }

    println!("=== Acceptance summary ===");
    println!(
        "SC-002 valid results        : {ok}/{} (target: 100%)",
        SCENARIOS.len()
    );
    println!(
        "SC-003 menu/plan leakage    : {menus}/{} (target: 0)",
        SCENARIOS.len()
    );
    println!("SC-003 tried-restatements   : enforced in run() — any would have errored above");
    println!("SC-004 max single-call ms   : {max_latency_ms} (target: < 15000)");

    let pass = ok as usize == SCENARIOS.len() && menus == 0 && max_latency_ms < 15_000;
    println!("\nACCEPTANCE: {}", if pass { "PASS" } else { "FAIL" });
}
