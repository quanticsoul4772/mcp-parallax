//! Spike S1 — rs-trafilatura extraction quality on bundled fixtures (T003).
//!
//! Validates research.md D2 before the pipeline depends on it: main text comes
//! out non-empty and boilerplate (nav, ads, cookie banners, footers) stays
//! out. Offline — no key, no network. Fallback crates are named in D2 if this
//! fails.
//!
//! Run: `cargo run --example spike_extract`

// Spikes are dev tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

const FIXTURES: &[(&str, &str, &[&str], &[&str])] = &[
    (
        "article",
        include_str!("../tests/fixtures/article.html"),
        // must appear in the extracted main text
        &["single-digit milliseconds", "release mode", "latency"],
        // boilerplate that must NOT appear
        &["Subscribe to our newsletter", "Privacy policy"],
    ),
    (
        "docs",
        include_str!("../tests/fixtures/docs.html"),
        &["WIDGET_TIMEOUT_MS", "silent fallback", "retryable"],
        &["Edit this page"],
    ),
    (
        "boilerplate",
        include_str!("../tests/fixtures/boilerplate.html"),
        &["evening visits rose forty percent", "operating levy"],
        &["Accept all cookies", "Tire Barn", "Cancel anytime"],
    ),
];

fn main() {
    let mut pass = true;

    for (name, html, must_have, must_not_have) in FIXTURES {
        let result = rs_trafilatura::extract(html);
        match result {
            Ok(extracted) => {
                let text = extracted.content_text;
                println!("== {name}: {} chars of main text", text.len());
                if text.trim().is_empty() {
                    println!("   FAIL: empty main text");
                    pass = false;
                }
                for needle in *must_have {
                    if !text.contains(needle) {
                        println!("   FAIL: main text missing {needle:?}");
                        pass = false;
                    }
                }
                for needle in *must_not_have {
                    if text.contains(needle) {
                        println!("   FAIL: boilerplate leaked: {needle:?}");
                        pass = false;
                    }
                }
            }
            Err(e) => {
                println!("== {name}: extraction error: {e}");
                pass = false;
            }
        }
    }

    println!(
        "\nSPIKE S1 (rs-trafilatura extraction): {}",
        if pass { "PASS" } else { "FAIL" }
    );
    assert!(pass, "extraction quality below the bar — see D2 fallbacks");
}
