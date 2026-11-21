//! 058: a tool request's `_meta` trace context parents the exported span.
//!
//! Its own process (each `tests/*.rs` file is one), so this file owns the
//! single telemetry init the in-memory exporter needs. Parenting rides the
//! recording path every exit takes, so these tests assert the span, not the
//! tool's outcome: the generic mock reply makes most calls fail, and a failed
//! call is still recorded and exported.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use mcp_parallax::observability;
use opentelemetry_sdk::metrics::in_memory_exporter::InMemoryMetricExporter;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use rmcp::model::{CallToolRequestParams, RequestMetaObject};
use serde_json::{json, Value};
use std::sync::OnceLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The caller's span every traced request names as its parent.
const PARENT_SPAN: &str = "00f067aa0ba902b7";

struct Telemetry {
    spans: InMemorySpanExporter,
    guard: observability::Guard,
}

fn telemetry() -> &'static Telemetry {
    static TELEMETRY: OnceLock<Telemetry> = OnceLock::new();
    TELEMETRY.get_or_init(|| {
        let spans = InMemorySpanExporter::default();
        let guard = observability::init_with_exporters(
            spans.clone(),
            InMemoryMetricExporter::default(),
            "trace-context-tests",
        );
        Telemetry { spans, guard }
    })
}

/// A tool call carrying `traceparent` for `trace_id` in its request `_meta`.
fn traced(name: &str, arguments: &Value, trace_id: &str) -> CallToolRequestParams {
    let mut params = tool_call(name, arguments);
    let mut meta = RequestMetaObject::new();
    meta.set_traceparent(format!("00-{trace_id}-{PARENT_SPAN}-01"));
    params.meta = Some(meta);
    params
}

/// A distinct, valid trace id per call.
fn trace_id(n: usize) -> String {
    format!("{:032x}", 0x0580_0000_usize + n)
}

/// Every model reply is the same empty object: enough for each tool to run
/// and be recorded, whatever its outcome.
async fn generic_model(mock: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({}))))
        .mount(mock)
        .await;
}

fn exported() -> Vec<SpanData> {
    let telemetry = telemetry();
    telemetry.guard.flush();
    telemetry.spans.get_finished_spans().unwrap()
}

/// One call per tool, with arguments that deserialize; the outcome is not
/// under test, so most of these fail against the generic reply.
fn tool_calls(transcript: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("verify", json!({ "claim": "traced" })),
        (
            "unstick",
            json!({ "goal": "g", "blocked": "b", "tried": ["t"] }),
        ),
        ("diverge", json!({ "problem": "p" })),
        ("decide", json!({ "decision": "d", "options": ["a", "b"] })),
        ("elicit", json!({ "task": "t" })),
        ("check", json!({ "claim": "2 + 2 == 4" })),
        (
            "checkpoint_action",
            json!({
                "session_id": "tc-action",
                "transcript_path": "unused.jsonl",
                "tool_name": "Read",
                "tool_input": "src/main.rs"
            }),
        ),
        (
            "checkpoint_batch",
            json!({ "session_id": "tc-batch", "transcript_path": transcript }),
        ),
        (
            "checkpoint_turn",
            json!({
                "session_id": "tc-turn",
                "transcript_path": "unused.jsonl",
                "final_message": "done",
                "continuation": true
            }),
        ),
        (
            "save",
            json!({
                "content": "traced memory",
                "kind": "lesson",
                "origin": "this session",
                "external": false
            }),
        ),
        ("recall", json!({ "query": "traced" })),
        ("forget", json!({ "id": "no-such-memory" })),
        (
            "surface",
            json!({ "session_id": "tc-surface", "prompt": "traced prompt" }),
        ),
        (
            "research",
            json!({ "question": "traced?", "depth": "quick" }),
        ),
        (
            "grounded_verify",
            json!({ "claim": "a exists", "locators": [{ "path": "a.rs" }] }),
        ),
    ]
}

/// Every call whose span is missing from its trace or has another parent,
/// plus `checkpoint_batch`'s evaluation span.
fn unparented(spans: &[SpanData], calls: &[(&str, Value)], ids: &[String]) -> Vec<String> {
    let mut wrong = Vec::new();
    for ((name, _), id) in calls.iter().zip(ids) {
        let expected = format!("parallax.{name}");
        match spans
            .iter()
            .find(|s| s.name == expected && s.span_context.trace_id().to_string() == *id)
        {
            Some(span) if span.parent_span_id.to_string() == PARENT_SPAN => {}
            Some(span) => wrong.push(format!("{name}: parent {}", span.parent_span_id)),
            None => wrong.push(format!("{name}: no span in trace {id}")),
        }
    }
    let batch = calls
        .iter()
        .position(|(n, _)| *n == "checkpoint_batch")
        .unwrap();
    if !spans.iter().any(|s| {
        s.name.starts_with("parallax.checkpoint.")
            && s.span_context.trace_id().to_string() == ids[batch]
            && s.parent_span_id.to_string() == PARENT_SPAN
    }) {
        wrong.push("checkpoint_batch: no checkpoint span under the caller".into());
    }
    wrong
}

/// T005: every tool's span joins the trace its request carried.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_tool_span_joins_the_callers_trace() {
    telemetry();
    let dir = tempfile::tempdir().unwrap();
    let transcript = write_transcript(dir.path(), "tc-batch", &[("cargo test", true); 4]);
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    let root = dir.path().to_string_lossy().to_string();
    let calls = tool_calls(&transcript);
    let ids: Vec<String> = (0..calls.len()).map(trace_id).collect();

    let base_mock = MockServer::start().await;
    generic_model(&base_mock).await;
    let (base, _s1, _r1) = serve(&base_mock, 2_000).await;

    let memory_mock = MockServer::start().await;
    generic_model(&memory_mock).await;
    mount_embeddings(&memory_mock).await;
    let (memory, _s2, _r2) = serve_with_memory(&memory_mock, ":memory:").await;

    let research_mock = MockServer::start().await;
    generic_model(&research_mock).await;
    let (research, _s3, _r3) = serve_with_research(&research_mock).await;

    let grounded_mock = MockServer::start().await;
    generic_model(&grounded_mock).await;
    let (grounded, _s4, _r4) = serve_with_grounded(&grounded_mock, &root).await;

    for ((name, arguments), id) in calls.iter().zip(&ids) {
        let client = match *name {
            "save" | "recall" | "forget" | "surface" => &memory,
            "research" => &research,
            "grounded_verify" => &grounded,
            _ => &base,
        };
        // The outcome is not under test; the span is.
        let _ = client.call_tool(traced(name, arguments, id)).await;
    }

    let wrong = unparented(&exported(), &calls, &ids);
    assert!(
        wrong.is_empty(),
        "spans not parented:\n  {}",
        wrong.join("\n  ")
    );
}
