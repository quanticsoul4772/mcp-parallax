//! Spike 2 — one real structured-outputs call (T007, research.md).
//!
//! **Manual-run, live API, real spend.** Validates the exact request shape the
//! thin client will use: `output_config.format` with a sanitized schema, the
//! constrained JSON arriving as a string in `content[0].text`, and `stop_reason`
//! behaving per the documented table.
//!
//! Run: `ANTHROPIC_API_KEY=... cargo run --example spike_client`

// Spikes are dev tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mcp_parallax::schema::{sanitize, validate};
use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY required");
    let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-opus-4-8".to_string());

    // Tiny schema with a value constraint the grammar drops (confidence range).
    let unsanitized = json!({
        "type": "object",
        "properties": {
            "verdict": { "type": "string", "enum": ["supported", "refuted"] },
            "findings": { "type": "array", "items": { "type": "string" } },
            "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
        },
        "required": ["verdict", "findings", "confidence"],
        "additionalProperties": false
    });
    let schema = sanitize(&unsanitized);

    let body = json!({
        "model": model,
        "max_tokens": 512,
        "messages": [{
            "role": "user",
            "content": "Verify this claim: 'The Battle of Hastings was fought in 1067.' \
                        Respond with a verdict, specific findings, and a confidence in [0,1]."
        }],
        "output_config": { "format": { "type": "json_schema", "schema": schema } }
    });

    let resp = reqwest::Client::new()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .expect("request sent");

    let status = resp.status();
    let payload: Value = resp.json().await.expect("json body");
    println!("HTTP {status}");
    println!("{}", serde_json::to_string_pretty(&payload).unwrap());

    assert!(status.is_success(), "non-2xx: {payload}");

    // Exit criteria (research.md spike 2):
    let stop_reason = payload["stop_reason"]
        .as_str()
        .expect("stop_reason present");
    println!("\nstop_reason = {stop_reason}");
    assert_eq!(stop_reason, "end_turn", "expected a normal completion");

    let text = payload["content"][0]["text"]
        .as_str()
        .expect("content[0].text is a string");
    let parsed: Value = serde_json::from_str(text).expect("constrained body parses as JSON");
    validate(&unsanitized, &parsed).expect("parsed value passes the UNSANITIZED schema");
    println!(
        "usage: in={} out={}",
        payload["usage"]["input_tokens"], payload["usage"]["output_tokens"]
    );
    println!(
        "\nSPIKE 2 PASS: content[0].text parsed and validated against the full schema:\n{parsed}"
    );
}
