//! OTLP export (007): telemetry as a pure function of the records.
//!
//! One span + metric set per tool invocation and per checkpoint evaluation,
//! derived from the record structs at the exact points the records are
//! written — one measurement, two sinks (FR-009). Off by default: providers
//! exist only when a standard OTLP endpoint variable is present and
//! `OTEL_SDK_DISABLED` is not true (honored app-side — the Rust SDK does
//! not implement it, upstream #1936). Telemetry failures never propagate
//! (FR-006); diagnostics ride the existing stderr `tracing` subscriber.

use crate::checkpoint::CheckpointRecord;
use crate::error::{ConfigError, Outcome};
use crate::telemetry::InvocationRecord;
use opentelemetry::metrics::{Counter, Histogram, MeterProvider as _};
use opentelemetry::trace::{Span, SpanKind, Status, Tracer, TracerProvider as _};
use opentelemetry::{Array, Context, KeyValue, StringValue, Value};
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider, SpanExporter};
use opentelemetry_sdk::Resource;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// Bound on the per-provider shutdown flush (FR-010).
pub const FLUSH_TIMEOUT_MS: u64 = 5_000;

/// `service.name` fallback when the environment doesn't set one.
pub const SERVICE_NAME: &str = "mcp-parallax";

/// The conventions' renamed provider key — the semconv crate (1.36) predates
/// the `gen_ai.system` → `gen_ai.provider.name` rename (research.md D5).
const GEN_AI_PROVIDER_NAME: &str = "gen_ai.provider.name";

/// GenAI semconv attribute keys — declared locally because the upstream
/// `opentelemetry_semantic_conventions` crate deprecated these in the 0.32
/// train (CI `-D warnings` makes the deprecation a hard error). The literals
/// below are the canonical identifiers OTel collectors receive; the test
/// assertions in this module verify against the same strings.
const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
const GEN_AI_TOKEN_TYPE: &str = "gen_ai.token.type";
const GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
const GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";

/// GenAI-standard token-usage histogram buckets (research.md D4).
const TOKEN_BUCKETS: [f64; 14] = [
    1.0,
    4.0,
    16.0,
    64.0,
    256.0,
    1_024.0,
    4_096.0,
    16_384.0,
    65_536.0,
    262_144.0,
    1_048_576.0,
    4_194_304.0,
    16_777_216.0,
    67_108_864.0,
];

/// Fast-path switch: emission points pay one atomic load when disabled
/// (FR-005).
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Process-global handles — set once at init. The global OTel provider
/// registry is deliberately NOT used: instruments are held here, created
/// from our own providers (S1 finding: `global::meter` is unwired unless a
/// global provider is installed).
static HANDLES: OnceLock<Handles> = OnceLock::new();

struct Handles {
    tracer: SdkTracer,
    invocations: Counter<u64>,
    invocation_duration: Histogram<f64>,
    cost: Counter<f64>,
    token_usage: Histogram<u64>,
    checkpoint_evaluations: Counter<u64>,
    checkpoint_duration: Histogram<f64>,
    push_evaluations: Counter<u64>,
}

/// Owns the providers; dropped/shut down by `main` on exit.
pub struct Guard {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
}

impl Guard {
    /// Flush buffered telemetry without shutting down (test harnesses read
    /// in-memory exporters between emissions).
    ///
    /// # Errors-as-logs
    ///
    /// Failures are warn-logged, never propagated (FR-006).
    pub fn flush(&self) {
        if let Err(e) = self.tracer_provider.force_flush() {
            tracing::warn!("telemetry trace flush failed: {e}");
        }
        if let Err(e) = self.meter_provider.force_flush() {
            tracing::warn!("telemetry metric flush failed: {e}");
        }
    }

    /// Shut both providers down within the bounded window (FR-010) —
    /// shutdown drains and exports the queue itself, so no separate flush
    /// (a pre-flush would wait on the SDK's own internal timeout FIRST,
    /// pushing the worst case past the bound — review finding 2). Failures
    /// are warn-logged, never propagated — a dead collector must not
    /// affect exit. Worst case: 2 × `FLUSH_TIMEOUT_MS`.
    pub fn shutdown(&self) {
        let timeout = Duration::from_millis(FLUSH_TIMEOUT_MS);
        if let Err(e) = self.tracer_provider.shutdown_with_timeout(timeout) {
            tracing::warn!("telemetry trace shutdown failed: {e}");
        }
        if let Err(e) = self.meter_provider.shutdown_with_timeout(timeout) {
            tracing::warn!("telemetry metric shutdown failed: {e}");
        }
    }
}

/// The endpoint variables that constitute enablement (any present —
/// generic or signal-specific).
const ENDPOINT_VARS: [&str; 3] = [
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
];

/// The pure enablement gate (research.md D2): endpoint present and not
/// disabled. Pure over a lookup so the truth table tests with maps — no
/// process-env mutation (analysis finding U1).
///
/// `OTEL_SDK_DISABLED` follows the OTel spec's lenient semantics
/// (case-insensitive `"true"` disables, anything else does not) — a named
/// exception to the loud-malformed-config convention: the variable's
/// contract is OTel's, not ours.
///
/// # Errors
///
/// A present endpoint that does not parse as a URL is a [`ConfigError`]
/// naming the variable — never a silent fallback.
fn gate(lookup: &dyn Fn(&str) -> Option<String>) -> Result<bool, ConfigError> {
    if lookup("OTEL_SDK_DISABLED").is_some_and(|v| v.trim().eq_ignore_ascii_case("true")) {
        return Ok(false);
    }
    let mut any = false;
    for var in ENDPOINT_VARS {
        if let Some(value) = lookup(var).filter(|v| !v.trim().is_empty()) {
            // 0.32 treats schemeless endpoints as https; a value that can't
            // parse even with a scheme assumed is a configuration error.
            let candidate = if value.contains("://") {
                value
            } else {
                format!("https://{value}")
            };
            if candidate.parse::<reqwest::Url>().is_err() {
                return Err(ConfigError::Invalid(match var {
                    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT" => "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT" => "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
                    _ => "OTEL_EXPORTER_OTLP_ENDPOINT",
                }));
            }
            any = true;
        }
    }
    Ok(any)
}

/// Initialize telemetry from the environment (FR-004). Returns `None` when
/// disabled — no providers, no exporter, no egress.
///
/// # Errors
///
/// [`ConfigError`] for a present-but-malformed endpoint variable.
pub fn init(instance_id: &str) -> Result<Option<Guard>, ConfigError> {
    if !gate(&|key| std::env::var(key).ok())? {
        return Ok(None);
    }
    let span_exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => {
            tracing::error!("OTLP span exporter construction failed: {e}");
            return Err(ConfigError::Invalid(
                "OTEL_EXPORTER_OTLP_* (exporter construction - see stderr)",
            ));
        }
    };
    let metric_exporter = match opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => {
            tracing::error!("OTLP metric exporter construction failed: {e}");
            return Err(ConfigError::Invalid(
                "OTEL_EXPORTER_OTLP_* (exporter construction - see stderr)",
            ));
        }
    };
    Ok(Some(init_with_exporters(
        span_exporter,
        metric_exporter,
        instance_id,
    )))
}

/// Build providers/instruments from explicit exporters and enable emission.
///
/// `init` uses this with the OTLP exporters; integration tests inject the
/// SDK's in-memory exporters — the export boundary is the SDK's own
/// abstraction, deliberately not wrapped in a bespoke seam (plan,
/// Constitution IV note).
pub fn init_with_exporters<S, M>(span_exporter: S, metric_exporter: M, instance_id: &str) -> Guard
where
    S: SpanExporter + 'static,
    M: PushMetricExporter + 'static,
{
    let mut resource = Resource::builder().with_attributes([
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        KeyValue::new("service.instance.id", instance_id.to_string()),
    ]);
    // Resource::builder honors OTEL_SERVICE_NAME via its env detector; the
    // fallback applies only when the operator didn't choose a name.
    if std::env::var("OTEL_SERVICE_NAME").is_err() {
        resource = resource.with_service_name(SERVICE_NAME);
    }
    let resource = resource.build();

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();
    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .with_resource(resource)
        .build();

    let meter = meter_provider.meter(SERVICE_NAME);
    let handles = Handles {
        tracer: tracer_provider.tracer(SERVICE_NAME),
        invocations: meter
            .u64_counter("parallax.invocations")
            .with_unit("{invocation}")
            .with_description("Completed tool invocations by tool/model/outcome")
            .build(),
        invocation_duration: meter
            .f64_histogram("parallax.invocation.duration")
            .with_unit("s")
            .with_description("Tool invocation latency")
            .build(),
        cost: meter
            .f64_counter("parallax.cost")
            .with_unit("USD")
            .with_description("Computed invocation cost")
            .build(),
        token_usage: meter
            .u64_histogram("gen_ai.client.token.usage")
            .with_unit("{token}")
            .with_boundaries(TOKEN_BUCKETS.to_vec())
            .with_description("Model token usage per invocation")
            .build(),
        checkpoint_evaluations: meter
            .u64_counter("parallax.checkpoint.evaluations")
            .with_unit("{evaluation}")
            .with_description("Checkpoint evaluations by boundary/verdict")
            .build(),
        checkpoint_duration: meter
            .f64_histogram("parallax.checkpoint.duration")
            .with_unit("s")
            .with_description("Checkpoint evaluation latency")
            .build(),
        push_evaluations: meter
            .u64_counter("parallax.push.evaluations")
            .with_unit("{evaluation}")
            .with_description("Push evaluations by outcome (surfaced/silent/fail-open)")
            .build(),
    };
    // First init wins; a second init (tests) keeps the existing handles —
    // emission still flows to the first exporters, which is what the
    // process-global test harness expects. The warn makes the disconnect
    // visible if production ever inits twice (review finding 6).
    if HANDLES.set(handles).is_err() {
        tracing::warn!(
            "telemetry handles already initialized - emission flows to the first init's exporters"
        );
    }
    ENABLED.store(true, Ordering::Release);

    Guard {
        tracer_provider,
        meter_provider,
    }
}

/// `gen_ai.provider.name` from the attributed model id.
fn provider_of(model: &str) -> &'static str {
    if model.starts_with("voyage") {
        "voyageai"
    } else {
        "anthropic"
    }
}

/// Span timing from a record: end = `created_at`, start = end − latency.
fn record_window(
    created_at: chrono::DateTime<chrono::Utc>,
    latency_ms: u64,
) -> (SystemTime, SystemTime) {
    let end: SystemTime = created_at.into();
    let start = end
        .checked_sub(Duration::from_millis(latency_ms))
        .unwrap_or(end);
    (start, end)
}

/// Export one invocation record (data-model §3). Fire-and-forget; a single
/// atomic load when telemetry is disabled.
pub fn emit_invocation(record: &InvocationRecord) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    let Some(handles) = HANDLES.get() else {
        return;
    };

    let (start, end) = record_window(record.created_at, record.latency_ms);
    let outcome = record.outcome.as_str();
    let provider = provider_of(&record.model);

    let mut attributes = vec![
        KeyValue::new(GEN_AI_OPERATION_NAME, "execute_tool"),
        KeyValue::new(GEN_AI_REQUEST_MODEL, record.model.clone()),
        #[allow(clippy::cast_possible_wrap)] // token counts far below i64::MAX
        KeyValue::new(GEN_AI_USAGE_INPUT_TOKENS, record.input_tokens as i64),
        #[allow(clippy::cast_possible_wrap)]
        KeyValue::new(GEN_AI_USAGE_OUTPUT_TOKENS, record.output_tokens as i64),
        KeyValue::new(GEN_AI_PROVIDER_NAME, provider),
        KeyValue::new("parallax.tool", record.tool.clone()),
        KeyValue::new("parallax.outcome", outcome),
        KeyValue::new("parallax.cost_usd", record.cost_usd),
        KeyValue::new("parallax.session_id", record.session_id.clone()),
    ];
    if record.outcome != Outcome::Success {
        attributes.push(KeyValue::new("error.type", outcome));
    }

    let mut span = handles
        .tracer
        .span_builder(format!("parallax.{}", record.tool))
        .with_kind(SpanKind::Client)
        .with_start_time(start)
        .with_attributes(attributes)
        .start_with_context(&handles.tracer, &Context::new());
    span.set_status(if record.outcome == Outcome::Success {
        Status::Ok
    } else {
        Status::error(outcome.to_string())
    });
    span.end_with_timestamp(end);

    let base = [
        KeyValue::new("parallax.tool", record.tool.clone()),
        KeyValue::new(GEN_AI_REQUEST_MODEL, record.model.clone()),
        KeyValue::new("parallax.outcome", outcome),
    ];
    handles.invocations.add(1, &base);
    #[allow(clippy::cast_precision_loss)] // latency in seconds, ms precision is ample
    handles.invocation_duration.record(
        record.latency_ms as f64 / 1000.0,
        &[
            KeyValue::new("parallax.tool", record.tool.clone()),
            KeyValue::new("parallax.outcome", outcome),
        ],
    );
    handles.cost.add(
        record.cost_usd,
        &[
            KeyValue::new("parallax.tool", record.tool.clone()),
            KeyValue::new(GEN_AI_REQUEST_MODEL, record.model.clone()),
        ],
    );
    for (kind, count) in [
        ("input", record.input_tokens),
        ("output", record.output_tokens),
    ] {
        handles.token_usage.record(
            count,
            &[
                KeyValue::new(GEN_AI_TOKEN_TYPE, kind),
                KeyValue::new(GEN_AI_REQUEST_MODEL, record.model.clone()),
                KeyValue::new(GEN_AI_PROVIDER_NAME, provider),
                KeyValue::new("parallax.tool", record.tool.clone()),
            ],
        );
    }
}

/// Export one checkpoint record (data-model §4). Signal KINDS only — never
/// evidence strings (FR-008).
pub fn emit_checkpoint(record: &CheckpointRecord) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    let Some(handles) = HANDLES.get() else {
        return;
    };

    let (start, end) = record_window(record.created_at, record.latency_ms);
    let boundary = record.boundary.as_str();
    let verdict = record.verdict.as_str();
    let signal_kinds: Vec<StringValue> = record
        .signals_fired
        .iter()
        .map(|s| StringValue::from(s.kind.as_str().to_string()))
        .collect();

    let attributes = vec![
        KeyValue::new("parallax.checkpoint.boundary", boundary),
        KeyValue::new("parallax.checkpoint.verdict", verdict),
        KeyValue::new(
            "parallax.checkpoint.signals",
            Value::Array(Array::String(signal_kinds)),
        ),
        KeyValue::new("parallax.checkpoint.suppressed", record.suppressed),
        KeyValue::new("parallax.checkpoint.fail_open", record.fail_open),
        KeyValue::new("parallax.checkpoint.review_ran", record.review_ran),
        KeyValue::new("parallax.checkpoint.cost_usd", record.cost_usd),
        KeyValue::new("parallax.session_id", record.session_id.clone()),
    ];

    let mut span = handles
        .tracer
        .span_builder(format!("parallax.checkpoint.{boundary}"))
        .with_kind(SpanKind::Internal)
        .with_start_time(start)
        .with_attributes(attributes)
        .start_with_context(&handles.tracer, &Context::new());
    // Fail-open is data, not an error: the evaluation completed (data-model §4).
    span.set_status(Status::Ok);
    span.end_with_timestamp(end);

    handles.checkpoint_evaluations.add(
        1,
        &[
            KeyValue::new("parallax.checkpoint.boundary", boundary),
            KeyValue::new("parallax.checkpoint.verdict", verdict),
            KeyValue::new("parallax.checkpoint.suppressed", record.suppressed),
            KeyValue::new("parallax.checkpoint.fail_open", record.fail_open),
        ],
    );
    #[allow(clippy::cast_precision_loss)]
    handles.checkpoint_duration.record(
        record.latency_ms as f64 / 1000.0,
        &[
            KeyValue::new("parallax.checkpoint.boundary", boundary),
            KeyValue::new("parallax.checkpoint.verdict", verdict),
        ],
    );
}

/// Mirror one push evaluation record (016) as a span + counter.
///
/// Emitted at the same exit point as the store write (007 FR-009 — the
/// surfaces cannot disagree). Attributes carry counts and outcomes only —
/// never memory content or ids (the checkpoint no-evidence rule).
pub fn emit_push(record: &crate::memory::push::PushRecord) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    let Some(handles) = HANDLES.get() else {
        return;
    };

    let (start, end) = record_window(record.created_at, record.latency_ms);
    let surfaced_count = record.surfaced_ids.len();
    let attributes = vec![
        KeyValue::new(
            "parallax.push.surfaced_count",
            i64::try_from(surfaced_count).unwrap_or(i64::MAX),
        ),
        KeyValue::new("parallax.push.fail_open", record.fail_open),
        #[allow(clippy::cast_possible_wrap)] // token counts far below i64::MAX
        KeyValue::new("gen_ai.usage.input_tokens", record.input_tokens as i64),
        KeyValue::new("parallax.session_id", record.session_id.clone()),
    ];

    let mut span = handles
        .tracer
        .span_builder("parallax.push")
        .with_kind(SpanKind::Internal)
        .with_start_time(start)
        .with_attributes(attributes)
        .start_with_context(&handles.tracer, &Context::new());
    // Fail-open is data, not an error: the evaluation completed.
    span.set_status(Status::Ok);
    span.end_with_timestamp(end);

    let outcome = if record.fail_open {
        "fail_open"
    } else if surfaced_count > 0 {
        "surfaced"
    } else {
        "silent"
    };
    handles
        .push_evaluations
        .add(1, &[KeyValue::new("parallax.push.outcome", outcome)]);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::checkpoint::{Boundary, Signal, SignalKind, Verdict};
    use chrono::{DateTime, Utc};
    use opentelemetry_sdk::metrics::in_memory_exporter::InMemoryMetricExporter;
    use opentelemetry_sdk::trace::InMemorySpanExporter;

    fn lookup<'a>(map: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            map.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_string())
        }
    }

    // D2 truth table over injected lookups — no process-env mutation (U1).
    #[test]
    fn gate_truth_table() {
        // No env -> disabled.
        assert!(!gate(&lookup(&[])).unwrap());
        // Any endpoint -> enabled.
        assert!(gate(&lookup(&[(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://localhost:4318"
        )]))
        .unwrap());
        assert!(gate(&lookup(&[(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            "https://collector.example:4318"
        )]))
        .unwrap());
        // Schemeless parses with the assumed https scheme.
        assert!(gate(&lookup(&[(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "collector.example:4318"
        )]))
        .unwrap());
        // Empty value = absent.
        assert!(!gate(&lookup(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "  ")])).unwrap());
        // OTEL_SDK_DISABLED: OTel-spec semantics (case-insensitive true; garbage = not disabled).
        for disabled in ["true", "TRUE", "True"] {
            assert!(!gate(&lookup(&[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318"),
                ("OTEL_SDK_DISABLED", disabled),
            ]))
            .unwrap());
        }
        for not_disabled in ["false", "banana", "1"] {
            assert!(gate(&lookup(&[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318"),
                ("OTEL_SDK_DISABLED", not_disabled),
            ]))
            .unwrap());
        }
        // Malformed endpoint = loud ConfigError naming the variable.
        let err = gate(&lookup(&[(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://exa mple:bad port",
        )]))
        .unwrap_err();
        assert!(err.to_string().contains("OTEL_EXPORTER_OTLP_ENDPOINT"));
    }

    fn sample_invocation(outcome: Outcome) -> InvocationRecord {
        InvocationRecord {
            id: "i1".into(),
            session_id: "s1".into(),
            tool: "verify".into(),
            model: "claude-opus-4-8".into(),
            input_tokens: 300,
            output_tokens: 30,
            cost_usd: 0.00225,
            latency_ms: 1200,
            outcome,
            created_at: DateTime::parse_from_rfc3339("2026-06-12T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    fn sample_checkpoint() -> CheckpointRecord {
        CheckpointRecord {
            id: "c1".into(),
            session_id: "s1".into(),
            boundary: Boundary::Batch,
            signals_evaluated: vec![SignalKind::Repetition, SignalKind::RepeatedFailure],
            signals_fired: vec![Signal::new(
                SignalKind::Repetition,
                "the action `bash cargo test` was invoked 4 times".into(),
                "bash cargo test",
            )],
            delivered_keys: vec!["repetition:abc".into()],
            review_ran: false,
            verdict: Verdict::Flag,
            suppressed: false,
            fail_open: false,
            latency_ms: 12,
            cost_usd: 0.0,
            created_at: DateTime::parse_from_rfc3339("2026-06-12T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    /// One shared global init for this test binary (OnceLock semantics);
    /// individual tests reset the exporters and assert their own slices.
    fn test_handles() -> (
        &'static InMemorySpanExporter,
        &'static InMemoryMetricExporter,
        &'static Guard,
    ) {
        static EXPORTERS: OnceLock<(InMemorySpanExporter, InMemoryMetricExporter, Guard)> =
            OnceLock::new();
        let (spans, metrics, guard) = EXPORTERS.get_or_init(|| {
            let spans = InMemorySpanExporter::default();
            let metrics = InMemoryMetricExporter::default();
            let guard = init_with_exporters(spans.clone(), metrics.clone(), "test-instance");
            (spans, metrics, guard)
        });
        (spans, metrics, guard)
    }

    fn attr<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a opentelemetry::Value> {
        attrs
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| &kv.value)
    }

    // SC-001 unit slice: field-for-field span equality against the record.
    #[test]
    fn invocation_span_matches_the_record_field_for_field() {
        // No reset(): the exporter is process-global and shared across
        // parallel tests — a reset here races other tests' emit/read windows
        // (observed flake). Each test finds its span by a unique name.
        let (spans, _, guard) = test_handles();
        let record = sample_invocation(Outcome::Success);
        emit_invocation(&record);
        guard.tracer_provider.force_flush().unwrap();

        let exported = spans.get_finished_spans().unwrap();
        let span = exported
            .iter()
            .find(|s| s.name == "parallax.verify")
            .expect("invocation span exported");
        assert_eq!(span.span_kind, SpanKind::Client);
        assert_eq!(span.status, Status::Ok);
        // Timing derived from the record (retroactive).
        let end: SystemTime = record.created_at.into();
        assert_eq!(span.end_time, end);
        assert_eq!(span.start_time, end - Duration::from_millis(1200));
        // Attribute equality with the record's values.
        let attrs = &span.attributes;
        assert_eq!(
            attr(attrs, "gen_ai.request.model").unwrap().as_str(),
            "claude-opus-4-8"
        );
        assert_eq!(
            attr(attrs, "gen_ai.usage.input_tokens")
                .unwrap()
                .to_string(),
            "300"
        );
        assert_eq!(
            attr(attrs, "gen_ai.usage.output_tokens")
                .unwrap()
                .to_string(),
            "30"
        );
        assert_eq!(
            attr(attrs, "gen_ai.provider.name").unwrap().as_str(),
            "anthropic"
        );
        assert_eq!(attr(attrs, "parallax.tool").unwrap().as_str(), "verify");
        assert_eq!(attr(attrs, "parallax.outcome").unwrap().as_str(), "success");
        assert_eq!(attr(attrs, "parallax.session_id").unwrap().as_str(), "s1");
        assert!(
            attr(attrs, "error.type").is_none(),
            "no error.type on success"
        );
        // FR-008: nothing beyond record fields.
        for kv in attrs {
            assert!(
                kv.key.as_str().starts_with("gen_ai.")
                    || kv.key.as_str().starts_with("parallax.")
                    || kv.key.as_str() == "error.type",
                "unexpected attribute {}",
                kv.key
            );
        }
    }

    #[test]
    fn error_outcomes_carry_error_status_and_type() {
        // No reset(): the exporter is process-global and shared across
        // parallel tests — a reset here races other tests' emit/read windows
        // (observed flake). Each test finds its span by a unique name.
        let (spans, _, guard) = test_handles();
        let mut record = sample_invocation(Outcome::Timeout);
        record.tool = "research".into();
        emit_invocation(&record);
        guard.tracer_provider.force_flush().unwrap();

        let exported = spans.get_finished_spans().unwrap();
        let span = exported
            .iter()
            .find(|s| s.name == "parallax.research")
            .expect("span exported");
        assert!(matches!(&span.status, Status::Error { description } if description == "timeout"));
        assert_eq!(
            attr(&span.attributes, "error.type").unwrap().as_str(),
            "timeout"
        );
    }

    #[test]
    fn voyage_models_attribute_the_voyage_provider() {
        // No reset(): the exporter is process-global and shared across
        // parallel tests — a reset here races other tests' emit/read windows
        // (observed flake). Each test finds its span by a unique name.
        let (spans, _, guard) = test_handles();
        let mut record = sample_invocation(Outcome::Success);
        record.tool = "recall".into();
        record.model = "voyage-4".into();
        emit_invocation(&record);
        guard.tracer_provider.force_flush().unwrap();

        let exported = spans.get_finished_spans().unwrap();
        let span = exported
            .iter()
            .find(|s| s.name == "parallax.recall")
            .expect("span exported");
        assert_eq!(
            attr(&span.attributes, "gen_ai.provider.name")
                .unwrap()
                .as_str(),
            "voyageai"
        );
    }

    // FR-008: checkpoint spans carry signal KINDS, never evidence strings.
    #[test]
    fn checkpoint_span_matches_the_record_and_omits_evidence() {
        // No reset(): the exporter is process-global and shared across
        // parallel tests — a reset here races other tests' emit/read windows
        // (observed flake). Each test finds its span by a unique name.
        let (spans, _, guard) = test_handles();
        let record = sample_checkpoint();
        emit_checkpoint(&record);
        guard.tracer_provider.force_flush().unwrap();

        let exported = spans.get_finished_spans().unwrap();
        let span = exported
            .iter()
            .find(|s| s.name == "parallax.checkpoint.batch")
            .expect("checkpoint span exported");
        assert_eq!(span.span_kind, SpanKind::Internal);
        assert_eq!(span.status, Status::Ok);
        let attrs = &span.attributes;
        assert_eq!(
            attr(attrs, "parallax.checkpoint.verdict").unwrap().as_str(),
            "flag"
        );
        assert_eq!(
            attr(attrs, "parallax.checkpoint.signals")
                .unwrap()
                .to_string(),
            "[\"repetition\"]"
        );
        // The evidence string must not appear anywhere in the attributes.
        for kv in attrs {
            assert!(
                !kv.value.to_string().contains("cargo test"),
                "evidence leaked into {}",
                kv.key
            );
        }
    }

    #[test]
    fn push_span_carries_counts_and_never_memory_content() {
        let (spans, _, guard) = test_handles();
        let record = crate::memory::push::PushRecord {
            id: "p-otel-1".into(),
            session_id: "otel-push-session".into(),
            surfaced_ids: vec!["mem-secret-content-id".into(), "mem-2".into()],
            latency_ms: 33,
            fail_open: false,
            input_tokens: 9,
            created_at: DateTime::parse_from_rfc3339("2026-07-23T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        emit_push(&record);
        guard.tracer_provider.force_flush().unwrap();

        let exported = spans.get_finished_spans().unwrap();
        let span = exported
            .iter()
            .find(|s| {
                s.name == "parallax.push"
                    && attr(&s.attributes, "parallax.session_id")
                        .is_some_and(|v| v.as_str() == "otel-push-session")
            })
            .expect("push span exported");
        assert_eq!(span.span_kind, SpanKind::Internal);
        assert_eq!(span.status, Status::Ok);
        let attrs = &span.attributes;
        assert_eq!(
            attr(attrs, "parallax.push.surfaced_count")
                .unwrap()
                .to_string(),
            "2"
        );
        assert_eq!(
            attr(attrs, "parallax.push.fail_open").unwrap().to_string(),
            "false"
        );
        assert_eq!(
            attr(attrs, "gen_ai.usage.input_tokens")
                .unwrap()
                .to_string(),
            "9"
        );
        // Counts and outcomes only — never memory ids or content (the
        // checkpoint no-evidence rule applied to push).
        for kv in attrs {
            assert!(
                !kv.value.to_string().contains("mem-secret-content-id"),
                "memory id leaked into {}",
                kv.key
            );
        }
    }

    #[test]
    fn metrics_reflect_emissions() {
        let (_, metrics, guard) = test_handles();
        emit_invocation(&sample_invocation(Outcome::Success));
        emit_checkpoint(&sample_checkpoint());
        guard.meter_provider.force_flush().unwrap();

        let exported = metrics.get_finished_metrics().unwrap();
        let names: Vec<String> = exported
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .map(|m| m.name().to_string())
            .collect();
        for expected in [
            "parallax.invocations",
            "parallax.invocation.duration",
            "parallax.cost",
            "gen_ai.client.token.usage",
            "parallax.checkpoint.evaluations",
            "parallax.checkpoint.duration",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing {expected}: {names:?}"
            );
        }
    }
}
