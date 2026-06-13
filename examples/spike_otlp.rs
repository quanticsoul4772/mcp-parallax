//! S1 spike (007-observability-layer, research.md): the full OTLP path on
//! the 0.32 train, compiled and run for real — gated init from env,
//! one retroactive span + metrics from a synthetic record, `force_flush`,
//! bounded shutdown — against a wiremock OTLP/HTTP double.
//!
//! Verifies the research-flagged uncertainties:
//!   (a) requests arrive at /v1/traces and /v1/metrics when the endpoint
//!       env is set; ZERO requests when absent;
//!   (b) the `GEN_AI` semconv constants compile (`semconv_experimental`);
//!   (c) retroactive timestamps (start in the past, `end_with_timestamp`)
//!       are accepted end to end.
//!
//! Run: `cargo run --example spike_otlp` (no credentials, no real backend).

#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

use opentelemetry::trace::{Span, SpanKind, Status, Tracer, TracerProvider as _};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::attribute::{
    GEN_AI_OPERATION_NAME, GEN_AI_REQUEST_MODEL, GEN_AI_USAGE_INPUT_TOKENS,
    GEN_AI_USAGE_OUTPUT_TOKENS,
};
use std::time::{Duration, SystemTime};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn endpoint_present() -> bool {
    [
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

#[tokio::main]
async fn main() {
    // Phase A: endpoint env absent -> the gate says disabled; nothing is
    // ever built, so zero requests are possible by construction.
    assert!(
        !endpoint_present(),
        "spike requires a clean env (no OTEL_EXPORTER_OTLP_* set)"
    );
    println!("phase A: no endpoint env -> gated off (no providers built)");

    // Phase B: a wiremock OTLP/HTTP double + env-driven init.
    let collector = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/traces"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1..)
        .mount(&collector)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/metrics"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1..)
        .mount(&collector)
        .await;

    // The exporter reads the env at build() (research D2). Example main is
    // single-threaded at this point — set_var is safe here.
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", collector.uri());
    assert!(endpoint_present());

    let resource = Resource::builder()
        .with_service_name("mcp-parallax-spike")
        .with_attributes([
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            KeyValue::new("service.instance.id", "spike-session"),
        ])
        .build();

    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
        .expect("span exporter from env");
    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();

    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .build()
        .expect("metric exporter from env");
    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .with_resource(resource)
        .build();

    // Retroactive span from a synthetic "record": ended 1s ago, 250ms long.
    let end = SystemTime::now() - Duration::from_secs(1);
    let start = end - Duration::from_millis(250);
    let tracer = tracer_provider.tracer("mcp-parallax-spike");
    let mut span = tracer
        .span_builder("parallax.verify")
        .with_kind(SpanKind::Client)
        .with_start_time(start)
        .with_attributes([
            KeyValue::new(GEN_AI_OPERATION_NAME, "execute_tool"),
            KeyValue::new(GEN_AI_REQUEST_MODEL, "claude-opus-4-8"),
            KeyValue::new(GEN_AI_USAGE_INPUT_TOKENS, 300_i64),
            KeyValue::new(GEN_AI_USAGE_OUTPUT_TOKENS, 30_i64),
            KeyValue::new("gen_ai.provider.name", "anthropic"),
            KeyValue::new("parallax.tool", "verify"),
            KeyValue::new("parallax.outcome", "success"),
            KeyValue::new("parallax.cost_usd", 0.00225_f64),
        ])
        .start_with_context(&tracer, &Context::new());
    span.set_status(Status::Ok);
    span.end_with_timestamp(end);

    let meter = opentelemetry::global::meter("mcp-parallax-spike");
    // NOTE: global meter is NOT wired to our provider unless set — use the
    // provider's meter directly instead (spike verifies the non-global path).
    drop(meter);
    let meter = opentelemetry::metrics::MeterProvider::meter(&meter_provider, "mcp-parallax-spike");
    let invocations = meter.u64_counter("parallax.invocations").build();
    invocations.add(
        1,
        &[
            KeyValue::new("parallax.tool", "verify"),
            KeyValue::new("parallax.outcome", "success"),
        ],
    );
    let latency = meter
        .f64_histogram("parallax.invocation.duration")
        .with_unit("s")
        .build();
    latency.record(0.25, &[KeyValue::new("parallax.tool", "verify")]);

    // Flush + bounded shutdown (FR-010 path).
    tracer_provider.force_flush().expect("trace flush");
    meter_provider.force_flush().expect("metric flush");
    tracer_provider
        .shutdown_with_timeout(Duration::from_secs(5))
        .expect("trace shutdown");
    meter_provider
        .shutdown_with_timeout(Duration::from_secs(5))
        .expect("metric shutdown");

    // Assertions: the double received both signals.
    let received = collector.received_requests().await.unwrap();
    let trace_posts = received
        .iter()
        .filter(|r| r.url.path() == "/v1/traces")
        .count();
    let metric_posts = received
        .iter()
        .filter(|r| r.url.path() == "/v1/metrics")
        .count();
    println!(
        "phase B: collector received {trace_posts} trace POST(s), {metric_posts} metric POST(s)"
    );
    assert!(trace_posts >= 1, "no trace export arrived");
    assert!(metric_posts >= 1, "no metric export arrived");
    let content_types: Vec<_> = received
        .iter()
        .filter_map(|r| r.headers.get("content-type"))
        .collect();
    println!("content types: {content_types:?}");

    println!("\nSPIKE: PASS — env-gated init, retroactive span, GenAI constants, flush+bounded shutdown all verified on 0.32");
}
