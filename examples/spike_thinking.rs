//! Spike 4 — thinking + `output_config` composability (T008, research.md).
//!
//! **Manual-run, live API, real spend.** The docs don't explicitly confirm that
//! extended thinking composes with structured outputs; this spike answers it
//! empirically. Core does NOT depend on the answer — the finding is recorded in
//! `docs/design/SDK_USAGE_CORE.md` either way.
//!
//! Run: `ANTHROPIC_API_KEY=... cargo run --example spike_thinking`

// Spikes are dev tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY required");
    let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-opus-4-8".to_string());

    let schema = json!({
        "type": "object",
        "properties": {
            "answer": { "type": "string", "enum": ["yes", "no"] },
            "reason": { "type": "string" }
        },
        "required": ["answer", "reason"],
        "additionalProperties": false
    });

    // Opus 4.8 rejects the legacy `thinking.type: enabled` shape outright
    // ("use thinking.type.adaptive and output_config.effort"), so the real
    // composability question is adaptive thinking + format.
    let body = json!({
        "model": model,
        "max_tokens": 2048,
        "thinking": { "type": "adaptive" },
        "messages": [{
            "role": "user",
            "content": "Is 1067 the year of the Battle of Hastings? Think it through, then answer."
        }],
        "output_config": {
            "effort": "medium",
            "format": { "type": "json_schema", "schema": schema }
        }
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

    if !status.is_success() {
        println!(
            "\nSPIKE 4 FINDING: thinking + output_config DO NOT compose (HTTP {status}). \
             Record in SDK_USAGE_CORE.md; core proceeds without thinking."
        );
        return;
    }

    // With thinking, content[] holds thinking block(s) followed by the text block.
    let text_block = payload["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
        .expect("a text block exists");
    let parsed: Value =
        serde_json::from_str(text_block["text"].as_str().expect("text is a string"))
            .expect("constrained body parses");
    println!(
        "\nSPIKE 4 FINDING: thinking + output_config COMPOSE. stop_reason={}, parsed={parsed}",
        payload["stop_reason"]
    );
}
