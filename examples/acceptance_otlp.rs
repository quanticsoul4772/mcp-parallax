//! Acceptance for 007-observability-layer (T009): real invocations and
//! checkpoint evaluations exported over the real OTLP/HTTP wire to an
//! in-process collector double, with the protobuf payloads decoded and
//! audited.
//!
//! Asserts SC-001 (one span per record; attribute values equal the stored
//! record), SC-002 (rates from telemetry == rates from records), SC-003
//! (disabled ⇒ zero telemetry requests + sub-microsecond emission
//! fast-path), SC-005 (attribute audit: no content/credentials anywhere in
//! the payloads), SC-006 (`GenAI` attribute names present). SC-004's
//! session-level behavior (unreachable collector, bounded exit) is asserted
//! by `stdio_smoke_with_unreachable_collector_stays_clean_and_exits_bounded`
//! in tests/integration.rs.
//!
//! No credentials needed: the Anthropic API is a wiremock double too.

#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::panic,
    clippy::type_complexity
)]

use mcp_parallax::client::AnthropicClient;
use mcp_parallax::config::Config;
use mcp_parallax::server::Parallax;
use mcp_parallax::storage::SqliteStorage;
use mcp_parallax::traits::clock::SystemClock;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message as _;
use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Instant;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLAIM: &str = "the acceptance claim text that must never appear in telemetry";

fn config() -> Config {
    Config {
        anthropic_api_key: "dummy-acceptance-key".into(),
        anthropic_model: "claude-opus-4-8".into(),
        verify_ensemble_k: 3,
        input_max_chars: 50_000,
        voyage_api_key: None,
        voyage_model: "voyage-4".into(),
        memory_recall_limit: 5,
        brave_api_key: None,
        fetch_timeout_ms: 10_000,
        research_concurrency: 8,
        fetch_allow_private: false,
        checkpoint_gate_patterns: vec![],
        database_path: ":memory:".into(),
        log_level: "info".into(),
        request_timeout_ms: 5_000,
        max_retries: 1,
    }
}

fn end_turn(value: &Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 100, "output_tokens": 10 }
    })
}

fn tool_call(name: &str, arguments: &Value) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new(name.to_string());
    params.arguments = arguments.as_object().cloned();
    params
}

fn write_transcript(dir: &std::path::Path, session: &str, commands: &[(&str, bool)]) -> String {
    let file_path = dir.join(format!("{session}.jsonl"));
    let mut file = std::fs::File::create(&file_path).unwrap();
    for (i, (command, failed)) in commands.iter().enumerate() {
        writeln!(
            file,
            "{}",
            json!({ "type": "assistant", "sessionId": session,
                    "message": { "role": "assistant", "content": [
                        { "type": "tool_use", "id": format!("t{i}"), "name": "Bash",
                          "input": { "command": command } } ]}})
        )
        .unwrap();
        if *failed {
            writeln!(
                file,
                "{}",
                json!({ "type": "user", "sessionId": session,
                        "message": { "role": "user", "content": [
                            { "type": "tool_result", "tool_use_id": format!("t{i}"), "is_error": true } ]}})
            )
            .unwrap();
        }
    }
    file_path.to_string_lossy().to_string()
}

#[tokio::main]
async fn main() {
    // ---- SC-003 (overhead half): disabled emission is the fast path ------
    let probe = mcp_parallax::telemetry::InvocationRecord {
        id: "overhead".into(),
        session_id: "overhead".into(),
        tool: "verify".into(),
        model: "claude-opus-4-8".into(),
        input_tokens: 1,
        output_tokens: 1,
        cost_usd: 0.0,
        latency_ms: 1,
        outcome: mcp_parallax::error::Outcome::Success,
        created_at: chrono::Utc::now(),
    };
    let started = Instant::now();
    for _ in 0..100_000 {
        mcp_parallax::observability::emit_invocation(&probe);
    }
    let per_call_ns = started.elapsed().as_nanos() / 100_000;
    println!("SC-003 disabled fast path: {per_call_ns} ns/call (bound: 1000 ns)");
    assert!(
        per_call_ns < 1_000,
        "disabled emission must be the atomic fast path"
    );

    // ---- SC-003 (egress half): no endpoint env ⇒ zero telemetry requests --
    assert!(
        std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT").is_none(),
        "acceptance requires a clean OTEL env"
    );
    let collector = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/traces"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&collector)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/metrics"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&collector)
        .await;
    assert!(
        mcp_parallax::observability::init("acceptance")
            .unwrap()
            .is_none(),
        "no endpoint env must mean disabled"
    );

    // ---- Enable for real (env-driven OTLP) and drive the server ----------
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", collector.uri());
    let guard = mcp_parallax::observability::init("acceptance-instance")
        .unwrap()
        .expect("endpoint set means enabled");

    let anthropic = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "verdict": "supported", "findings": []
        }))))
        .mount(&anthropic)
        .await;

    let cfg = config();
    let storage = Arc::new(SqliteStorage::connect(":memory:").await.unwrap());
    let anthropic_client =
        Arc::new(AnthropicClient::with_base_url(&cfg, &anthropic.uri()).with_backoff_base_ms(1));
    let server = Parallax::new(
        anthropic_client,
        storage.clone(),
        Arc::new(SystemClock),
        &cfg,
    )
    .unwrap();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move { server.serve(server_io).await });
    let client = ().serve(client_io).await.expect("client init");
    let _running = server_task.await.expect("join").expect("server init");

    // Invocations across classes: 3 successes + 1 invalid_input.
    for _ in 0..3 {
        client
            .call_tool(tool_call("verify", &json!({ "claim": CLAIM })))
            .await
            .unwrap();
    }
    let _ = client
        .call_tool(tool_call("verify", &json!({ "claim": "   " })))
        .await
        .unwrap_err();

    // Checkpoint evaluations: a flag, a silence, a fail-open.
    let dir = tempfile::tempdir().unwrap();
    let loop_path = write_transcript(
        dir.path(),
        "acc-loop",
        &[
            ("cargo test", true),
            ("cargo test", true),
            ("cargo test", true),
            ("cargo test", true),
        ],
    );
    client
        .call_tool(tool_call(
            "checkpoint_batch",
            &json!({ "session_id": "acc-loop", "transcript_path": loop_path }),
        ))
        .await
        .unwrap();
    let benign_path = write_transcript(
        dir.path(),
        "acc-benign",
        &[("ls", false), ("cargo fmt", false)],
    );
    client
        .call_tool(tool_call(
            "checkpoint_batch",
            &json!({ "session_id": "acc-benign", "transcript_path": benign_path }),
        ))
        .await
        .unwrap();
    client
        .call_tool(tool_call(
            "checkpoint_batch",
            &json!({ "session_id": "acc-gone", "transcript_path": "missing/never.jsonl" }),
        ))
        .await
        .unwrap();

    guard.flush();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // ---- Decode the OTLP payloads ----------------------------------------
    let received = collector.received_requests().await.unwrap();
    let mut spans = Vec::new();
    let mut metric_points: HashMap<String, Vec<(Vec<(String, String)>, f64)>> = HashMap::new();
    for request in &received {
        match request.url.path() {
            "/v1/traces" => {
                let decoded = ExportTraceServiceRequest::decode(request.body.as_slice()).unwrap();
                for rs in decoded.resource_spans {
                    for ss in rs.scope_spans {
                        spans.extend(ss.spans);
                    }
                }
            }
            "/v1/metrics" => {
                let decoded = ExportMetricsServiceRequest::decode(request.body.as_slice()).unwrap();
                for rm in decoded.resource_metrics {
                    for sm in rm.scope_metrics {
                        for metric in sm.metrics {
                            use opentelemetry_proto::tonic::metrics::v1::metric::Data;
                            let entry = metric_points.entry(metric.name.clone()).or_default();
                            match metric.data {
                                Some(Data::Sum(sum)) => {
                                    for dp in sum.data_points {
                                        entry.push((attrs_of(&dp.attributes), number_of(dp.value)));
                                    }
                                }
                                Some(Data::Histogram(hist)) => {
                                    for dp in hist.data_points {
                                        entry.push((attrs_of(&dp.attributes), dp.count as f64));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    println!(
        "collector: {} requests, {} spans, {} metric names",
        received.len(),
        spans.len(),
        metric_points.len()
    );

    // ---- SC-001: one span per record, attribute values equal the record --
    let invocations = storage.list_invocations().await.unwrap();
    let checkpoints = storage.list_checkpoints().await.unwrap();
    let expected_spans = invocations.len() + checkpoints.len();
    assert_eq!(
        spans.len(),
        expected_spans,
        "one span per record ({} invocations + {} checkpoints)",
        invocations.len(),
        checkpoints.len()
    );
    let span_attr =
        |span: &opentelemetry_proto::tonic::trace::v1::Span, key: &str| -> Option<String> {
            span.attributes
                .iter()
                .find(|kv| kv.key == key)
                .and_then(|kv| kv.value.as_ref())
                .map(any_value_string)
        };
    for record in &invocations {
        let span = spans
            .iter()
            .filter(|s| s.name == format!("parallax.{}", record.tool))
            .find(|s| span_attr(s, "parallax.outcome").as_deref() == Some(record.outcome.as_str()))
            .unwrap_or_else(|| panic!("span for {} {:?} missing", record.tool, record.outcome));
        assert_eq!(
            span_attr(span, "gen_ai.request.model").as_deref(),
            Some(record.model.as_str())
        );
        assert_eq!(
            span_attr(span, "gen_ai.usage.input_tokens").as_deref(),
            Some(record.input_tokens.to_string().as_str())
        );
    }
    for record in &checkpoints {
        let span = spans
            .iter()
            .find(|s| {
                s.name == format!("parallax.checkpoint.{}", record.boundary.as_str())
                    && span_attr(s, "parallax.session_id").as_deref()
                        == Some(record.session_id.as_str())
            })
            .unwrap_or_else(|| panic!("checkpoint span for {} missing", record.session_id));
        assert_eq!(
            span_attr(span, "parallax.checkpoint.verdict").as_deref(),
            Some(record.verdict.as_str())
        );
        assert_eq!(
            span_attr(span, "parallax.checkpoint.fail_open").as_deref(),
            Some(if record.fail_open { "true" } else { "false" })
        );
    }
    println!("SC-001: {} spans matched their records", spans.len());

    // ---- SC-002: rates from telemetry == rates from records --------------
    let counter_total = |name: &str, filter: &[(&str, &str)]| -> f64 {
        metric_points.get(name).map_or(0.0, |points| {
            points
                .iter()
                .filter(|(attrs, _)| {
                    filter
                        .iter()
                        .all(|(k, v)| attrs.iter().any(|(ak, av)| ak == k && av == v))
                })
                .map(|(_, value)| *value)
                .sum()
        })
    };
    let success_invocations = counter_total(
        "parallax.invocations",
        &[("parallax.tool", "verify"), ("parallax.outcome", "success")],
    );
    let record_successes = invocations
        .iter()
        .filter(|r| r.tool == "verify" && r.outcome.as_str() == "success")
        .count() as f64;
    assert!(
        (success_invocations - record_successes).abs() < f64::EPSILON,
        "telemetry success count {success_invocations} != record count {record_successes}"
    );
    let flags = counter_total(
        "parallax.checkpoint.evaluations",
        &[("parallax.checkpoint.verdict", "flag")],
    );
    let record_flags = checkpoints
        .iter()
        .filter(|c| c.verdict.as_str() == "flag")
        .count() as f64;
    assert!(
        (flags - record_flags).abs() < f64::EPSILON,
        "flag rate mismatch"
    );
    let fail_opens = counter_total(
        "parallax.checkpoint.evaluations",
        &[("parallax.checkpoint.fail_open", "true")],
    );
    let record_fail_opens = checkpoints.iter().filter(|c| c.fail_open).count() as f64;
    assert!(
        (fail_opens - record_fail_opens).abs() < f64::EPSILON,
        "fail-open rate mismatch"
    );
    println!("SC-002: telemetry-computed counts equal record-computed counts");

    // ---- SC-005: attribute audit — no content, no credentials ------------
    let mut all_values: Vec<String> = Vec::new();
    for span in &spans {
        for kv in &span.attributes {
            if let Some(v) = kv.value.as_ref() {
                all_values.push(any_value_string(v));
            }
        }
    }
    for points in metric_points.values() {
        for (attrs, _) in points {
            for (_, v) in attrs {
                all_values.push(v.clone());
            }
        }
    }
    for value in &all_values {
        assert!(
            !value.contains("acceptance claim text"),
            "claim text leaked: {value}"
        );
        assert!(
            !value.contains("dummy-acceptance-key"),
            "credential leaked: {value}"
        );
        assert!(
            !value.contains("cargo test"),
            "checkpoint evidence leaked: {value}"
        );
    }
    println!(
        "SC-005: {} attribute values audited — record fields only",
        all_values.len()
    );

    // ---- SC-006: GenAI names present --------------------------------------
    assert!(spans
        .iter()
        .any(|s| span_attr(s, "gen_ai.request.model").is_some()));
    assert!(spans
        .iter()
        .any(|s| span_attr(s, "gen_ai.usage.output_tokens").is_some()));
    assert!(metric_points.contains_key("gen_ai.client.token.usage"));
    println!("SC-006: GenAI semantic-convention names resolve");

    guard.shutdown();
    client.cancel().await.unwrap();
    println!("\nACCEPTANCE: PASS");
}

fn attrs_of(
    attributes: &[opentelemetry_proto::tonic::common::v1::KeyValue],
) -> Vec<(String, String)> {
    attributes
        .iter()
        .map(|kv| {
            (
                kv.key.clone(),
                kv.value.as_ref().map(any_value_string).unwrap_or_default(),
            )
        })
        .collect()
}

const fn number_of(
    value: Option<opentelemetry_proto::tonic::metrics::v1::number_data_point::Value>,
) -> f64 {
    use opentelemetry_proto::tonic::metrics::v1::number_data_point::Value as NumberValue;
    match value {
        Some(NumberValue::AsDouble(d)) => d,
        #[allow(clippy::cast_precision_loss)]
        Some(NumberValue::AsInt(i)) => i as f64,
        None => 0.0,
    }
}

fn any_value_string(value: &opentelemetry_proto::tonic::common::v1::AnyValue) -> String {
    use opentelemetry_proto::tonic::common::v1::any_value::Value as Av;
    match &value.value {
        Some(Av::StringValue(s)) => s.clone(),
        Some(Av::BoolValue(b)) => b.to_string(),
        Some(Av::IntValue(i)) => i.to_string(),
        Some(Av::DoubleValue(d)) => d.to_string(),
        Some(Av::ArrayValue(array)) => array
            .values
            .iter()
            .map(any_value_string)
            .collect::<Vec<_>>()
            .join(","),
        _ => String::new(),
    }
}
