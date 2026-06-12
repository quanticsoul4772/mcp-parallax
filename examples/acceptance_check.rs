//! T011 — live acceptance pass for the deterministic layer (manual-run).
//!
//! Translation quality is the only live question — the engines are
//! deterministic. SC-001: ≥20 ground-truth claims, 100% verdict accuracy.
//! SC-002: ≥6 uncheckable claims, 100% declined. SC-003: every successful
//! response carries `formal_form` + `engine_result`. SC-007: a repeated
//! check yields an identical engine result. Results recorded in
//! `specs/005-deterministic-layer/quickstart.md`.
//!
//! Run: `ANTHROPIC_API_KEY=... cargo run --release --example acceptance_check`

// Acceptance tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::client::AnthropicClient;
use mcp_parallax::config::Config;
use mcp_parallax::deterministic::check::{run, CheckDeps};
use mcp_parallax::deterministic::contract::CheckParams;
use mcp_parallax::deterministic::translate::{register, TRANSLATE_MODE_ID};
use mcp_parallax::deterministic::Verdict;
use mcp_parallax::modes::ModeRegistry;
use std::sync::Arc;

/// (claim, expected verdict) — ground truth spans both engines.
const GROUND_TRUTH: &[(&str, Verdict)] = &[
    // arithmetic — true
    (
        "A 37% reduction from 1840 ms leaves 1159.2 ms.",
        Verdict::Supported,
    ),
    ("2 to the power of 32 is 4294967296.", Verdict::Supported),
    ("15% of 240 is 36.", Verdict::Supported),
    (
        "The sum of the integers 1 through 100 is 5050.",
        Verdict::Supported,
    ),
    (
        "If a build takes 299 seconds, that is just under 5 minutes.",
        Verdict::Supported,
    ),
    (
        "Tripling 17 and adding 9 gives an even number.",
        Verdict::Supported,
    ),
    ("A 4-hour window is 14400 seconds.", Verdict::Supported),
    (
        "0.1 plus 0.2 equals 0.3 to within one part in a million.",
        Verdict::Supported,
    ),
    // arithmetic — false
    (
        "A 37% reduction from 1840 ms leaves about 1400 ms.",
        Verdict::Refuted,
    ),
    ("2 to the power of 31 exceeds 3 billion.", Verdict::Refuted),
    ("25% of 80 is 25.", Verdict::Refuted),
    ("Doubling 750 gives more than 2000.", Verdict::Refuted),
    ("A week has exactly 10000 minutes.", Verdict::Refuted),
    ("Half of 7 is greater than 4.", Verdict::Refuted),
    // constraints — claims about satisfiability with known truth
    (
        "There exist two integers greater than 2 whose sum is 11.",
        Verdict::Supported,
    ),
    (
        "A boolean cannot be both true and false at the same time.",
        Verdict::Supported,
    ),
    (
        "There is an integer strictly between 100 and 102.",
        Verdict::Supported,
    ),
    (
        "Three numbers can each be strictly less than the next, cyclically.",
        Verdict::Refuted,
    ),
    (
        "There is an integer that is both less than 5 and greater than 10.",
        Verdict::Refuted,
    ),
    (
        "Two distinct integers can both lie strictly between 7 and 9.",
        Verdict::Refuted,
    ),
    (
        "You can pick three distinct integers from the range 1 to 3.",
        Verdict::Supported,
    ),
];

/// Clearly uncheckable claims — 100% must decline (SC-002), incl. one
/// too-vague-to-bound numeric claim (analysis C1).
const UNCHECKABLE: &[&str] = &[
    "Rust is more elegant than C++.",
    "The third-floor meeting room is the largest in the building.",
    "Next year's conference will have higher attendance.",
    "This design is cleaner than the alternative.",
    "Most people prefer dark mode.",
    "The new logo feels more trustworthy.",
    "Performance got noticeably better recently.", // numeric-ish but unboundable
];

fn deps(config: &Config) -> CheckDeps {
    let mut registry = ModeRegistry::new();
    register(&mut registry).expect("register translate mode");
    CheckDeps {
        model_client: Arc::new(AnthropicClient::new(config)),
        translate_mode: registry.get(TRANSLATE_MODE_ID).unwrap().clone(),
        input_max_chars: config.input_max_chars,
    }
}

fn params(claim: &str) -> CheckParams {
    CheckParams {
        claim: claim.to_string(),
        context: None,
    }
}

#[tokio::main(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // a linear acceptance script reads best unsplit
async fn main() {
    let config = Config::from_env().expect("config (ANTHROPIC_API_KEY)");
    let deps = deps(&config);

    let (mut correct, mut audited, mut completed) = (0_u32, 0_u32, 0_u32);
    for (claim, expected) in GROUND_TRUTH {
        match run(&deps, &params(claim)).await {
            Ok((result, _, _)) => {
                completed += 1;
                let ok = result.verdict == *expected;
                if ok {
                    correct += 1;
                }
                if result.formal_form.is_some() && result.engine_result.is_some() {
                    audited += 1;
                }
                println!(
                    "[{}]{} ({:?}, {} attempt(s)) {claim}\n   form: {}\n",
                    match result.verdict {
                        Verdict::Supported => "supported",
                        Verdict::Refuted => "refuted",
                        Verdict::NotCheckable => "NOT CHECKABLE?!",
                    },
                    if ok { "" } else { " ** WRONG **" },
                    result.engine,
                    result.translation_attempts,
                    result.formal_form.as_deref().unwrap_or("(none)")
                );
            }
            Err(e) => println!("[ERROR] {claim}: {e}\n"),
        }
    }

    let mut declined = 0_u32;
    for claim in UNCHECKABLE {
        match run(&deps, &params(claim)).await {
            Ok((result, _, _)) => {
                let ok = result.verdict == Verdict::NotCheckable;
                if ok {
                    declined += 1;
                }
                println!(
                    "[{}]{} {claim} — {}",
                    if ok { "declined" } else { "VERDICT?!" },
                    if ok { "" } else { " ** FALSE PRECISION **" },
                    result.reason.as_deref().unwrap_or("(no reason)")
                );
            }
            Err(e) => println!("[ERROR] {claim}: {e}"),
        }
    }

    // SC-007: a repeated check with the same formalization → identical
    // engine result. (Engine-level determinism is unit-pinned; this repeats
    // one live claim and compares engine results when the forms agree.)
    let claim = "15% of 240 is 36.";
    let (first, _, _) = run(&deps, &params(claim)).await.expect("repeat 1");
    let (second, _, _) = run(&deps, &params(claim)).await.expect("repeat 2");
    let deterministic =
        first.formal_form != second.formal_form || first.engine_result == second.engine_result;
    println!(
        "\ndeterminism: forms {} -> results {}",
        if first.formal_form == second.formal_form {
            "identical"
        } else {
            "differ (translation variance)"
        },
        if first.engine_result == second.engine_result {
            "identical"
        } else {
            "DIFFER"
        },
    );

    println!("\n=== Acceptance summary ===");
    println!(
        "SC-001 verdict accuracy : {correct}/{} on {completed} completed (target: 100%)",
        GROUND_TRUTH.len()
    );
    println!(
        "SC-002 honest declines  : {declined}/{} (target: 100%)",
        UNCHECKABLE.len()
    );
    println!("SC-003 auditability     : {audited}/{completed} responses carry form+result");
    println!("SC-007 determinism      : {deterministic}");

    let pass = correct as usize == GROUND_TRUTH.len()
        && completed as usize == GROUND_TRUTH.len()
        && declined as usize == UNCHECKABLE.len()
        && audited == completed
        && deterministic;
    println!("\nACCEPTANCE: {}", if pass { "PASS" } else { "FAIL" });
}
