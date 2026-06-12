//! Spike S2 — Brave Search response shape (T004; live, one request).
//!
//! Pins the deserializer shape the wiremock tests mirror (research.md D1):
//! `web.results[].{url, title, description}`. Manual-run, real (tiny) spend.
//!
//! Run: `BRAVE_API_KEY=... cargo run --example spike_brave`

// Spikes are dev tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct SearchResponse {
    web: Option<WebResults>,
}

#[derive(Debug, Deserialize)]
struct WebResults {
    #[serde(default)]
    results: Vec<WebResult>,
}

#[derive(Debug, Deserialize)]
struct WebResult {
    url: String,
    title: String,
    #[serde(default)]
    description: String,
}

#[tokio::main]
async fn main() {
    let key = std::env::var("BRAVE_API_KEY").expect("BRAVE_API_KEY required for this spike");

    let response = reqwest::Client::new()
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", &key)
        .header("Accept", "application/json")
        .query(&[("q", "rust mcp server stdio"), ("count", "5")])
        .send()
        .await
        .expect("request");

    let status = response.status();
    println!("HTTP {status}");
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        panic!("non-2xx from Brave: {body}");
    }

    let payload: SearchResponse = response.json().await.expect("shape mismatch");
    let results = payload.web.expect("web key present").results;
    println!("{} results", results.len());
    assert!(!results.is_empty(), "no results for a common query");

    for (i, r) in results.iter().enumerate() {
        assert!(r.url.starts_with("http"), "url shape: {}", r.url);
        assert!(!r.title.is_empty(), "empty title");
        println!(
            "  [{i}] {} — {} ({} desc chars)",
            r.title,
            r.url,
            r.description.len()
        );
    }

    println!("\nSPIKE S2 (Brave response shape): PASS");
}
