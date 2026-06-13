# Phase 0 Research: Observability Layer

All decisions below rest on a web-research pass against docs.rs 0.32.x,
the opentelemetry-rust changelogs, and the GenAI semconv repository
(2026-06-12); sources are cited inline. Two flagged uncertainties carry
into spike S1.

## D1 — Crate stack: the 0.32 release train, HTTP/protobuf, no bridge

- **Decision**: `opentelemetry` 0.32, `opentelemetry_sdk` 0.32.1,
  `opentelemetry-otlp` 0.32 with `default-features = false` and features
  `["trace", "metrics", "http-proto", "reqwest-blocking-client",
  "reqwest-rustls-webpki-roots", "internal-logs"]`,
  `opentelemetry-semantic-conventions` 0.32 (`semconv_experimental`
  defensively — see D5). Dev-dependency: `opentelemetry_sdk` `testing`
  feature (in-memory exporters; runtime-agnostic since 0.32). All four
  pinned to the same train — the ecosystem breaks API across minors.
- **Rationale**: `http-proto` pulls prost only (no tonic/hyper/tower; no
  protoc anywhere — `opentelemetry-proto` ships pre-generated files);
  rustls is already in-tree via reqwest. The SDK's batch processors run on
  **dedicated background threads** since 0.28–0.30 (no `rt-tokio` story
  anymore) and the crate docs direct background-thread processors to the
  **blocking** reqwest client — the async client + batch processor pairing
  is the documented #1 hazard.
- **`tracing-opentelemetry` and `opentelemetry-appender-tracing` are NOT
  used** — they bridge `tracing` spans/logs into OTel; this design
  constructs spans directly from record structs. Named deviation from
  `SDK_LANDSCAPE.md` §observability's crate list; the corpus is amended in
  the same change (the listed bridge was for an instrumentation style we
  did not adopt).
- **Alternatives**: `grpc-tonic` (heavier dependency tree for zero
  functional gain here); bridging via `tracing-opentelemetry` (couples the
  telemetry surface to log statements instead of the canonical records,
  violating FR-009's one-measurement rule).

## D2 — Enablement gating (FR-004/FR-005, Constitution VI)

- **Decision**: at startup, telemetry initializes **iff** any of
  `OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` /
  `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` is set **and**
  `OTEL_SDK_DISABLED` is not `true`. Absent → no providers are built, no
  exporter exists, no egress is possible; every emission point early-returns
  on a static `AtomicBool`. A present-but-unparseable endpoint URL is a
  startup `ConfigError` naming the variable (existing convention).
- **Rationale**: the exporter's own default is `http://localhost:4318`
  even with no env set — "build unconditionally" would silently attempt
  egress on every box, violating off-by-default. **`OTEL_SDK_DISABLED` is
  not implemented by the Rust SDK** (open issue #1936, tracked in #3374) —
  honoring it app-side is a named upstream-gap workaround, kept so the
  spec-standard kill switch works for operators with globally inherited
  endpoints (the spec clarification's documented-not-gated posture).
- **`OTEL_SDK_DISABLED` parse semantics**: the OTel specification's —
  case-insensitive `"true"` disables, any other value (including garbage)
  means not-disabled. A named exception to this project's
  loud-on-malformed config convention: the variable belongs to OTel's
  contract, and erroring on values the OTel spec accepts would break the
  standard behavior operators expect. Pinned in T003's truth table.
- **Gate testability**: the gate is a pure function over an env-lookup
  closure so its truth table tests with injected maps — `std::env::set_var`
  in parallel tests is a race (analysis finding U1).
- Endpoint/protocol/headers/timeout details beyond presence are left to
  the exporter's own env handling (it reads the full
  `OTEL_EXPORTER_OTLP_*` family at `build()`, signal-specific over generic
  over defaults).

## D3 — Emission: pure functions of records at the record-write exit points

- **Decision**: `observability::emit_invocation(&InvocationRecord)` and
  `observability::emit_checkpoint(&CheckpointRecord)`, called at exactly
  the two places the rows are written (`RecordGuard::finish`, checkpoint
  `run::record()`). Span timing is derived: end = `created_at`, start =
  `created_at − latency_ms`. Telemetry failures never propagate (the batch
  queue absorbs; export errors surface as `tracing` warnings on stderr).
- **Rationale**: FR-009's one-measurement-two-sinks made literal — there
  is no second measurement anywhere, so the surfaces cannot disagree
  (SC-001's value-equality criterion holds by construction).
- **Retroactive-span API facts (0.32, verified)**: `SpanBuilder` has
  `start_time` but **no end-time field** — the span is ended via the
  `Span` trait's `end_with_timestamp(SystemTime)`. Spans are started with
  `start_with_context(&tracer, &Context::new())` — a fresh context keeps
  record-derived spans root-level (plain `start()` would parent to
  whatever context is current).

## D4 — Span + metric surface (the contract; full tables in contracts/telemetry.md)

- **Decision**: invocation spans named `parallax.{tool}`, kind `CLIENT`,
  status from outcome (`Ok` for success, `error(outcome)` otherwise);
  checkpoint spans named `parallax.checkpoint.{boundary}`, kind
  `INTERNAL`. Metrics: `parallax.invocations` (u64 counter;
  tool/model/outcome), `parallax.invocation.duration` (f64 histogram, s;
  tool/outcome), `parallax.cost` (f64 counter, USD; tool/model),
  `gen_ai.client.token.usage` (u64 histogram, {token}, the GenAI-standard
  buckets; `gen_ai.token.type` = input|output, model, provider) and
  `parallax.checkpoint.evaluations` (u64 counter;
  boundary/verdict/suppressed/fail_open). FR-002's rates are all
  computable from these.
- **Rationale**: cost has no GenAI-standard instrument → `parallax.*`;
  token usage has one → emit the standard instrument so existing GenAI
  dashboards work unmapped (SC-006), alongside the span attributes.

## D5 — GenAI semantic conventions (verified current state)

- **Decision**: span attributes use crate constants where they exist —
  `GEN_AI_OPERATION_NAME` (value: `execute_tool`), `GEN_AI_REQUEST_MODEL`,
  `GEN_AI_USAGE_INPUT_TOKENS`, `GEN_AI_USAGE_OUTPUT_TOKENS` — plus the
  literal key `"gen_ai.provider.name"` (value `anthropic` / `voyageai` per
  the record's model): the conventions moved to the standalone
  semantic-conventions-genai repo and renamed `gen_ai.system` →
  `gen_ai.provider.name`, but the crate (semconv 1.36) predates the
  rename, so the new key is hardcoded with a comment. Parallax-specific
  fields are namespaced `parallax.*` (tool, outcome, cost, session,
  checkpoint fields).
- **Flagged uncertainty (S1 confirms)**: whether the `GEN_AI_*` constants
  sit behind `semconv_experimental` — the feature is enabled defensively
  and dropped if the build doesn't need it.

## D6 — Resource identity

- **Decision**: `Resource::builder()` (which already honors
  `OTEL_SERVICE_NAME` / `OTEL_RESOURCE_ATTRIBUTES` via its env detectors)
  with programmatic fallbacks: `service.name = "mcp-parallax"` when the
  env doesn't set one, `service.version` = crate version,
  `service.instance.id` = the per-process session UUID (one Parallax
  process per harness session — instance identity is what separates
  concurrent sessions in a backend).

## D7 — Lifecycle (FR-010)

- **Decision**: `observability::init()` runs in `main` after config load,
  returning an optional guard owning both providers; on clean exit the
  guard runs `force_flush()` then `shutdown_with_timeout(5 s)` on each
  (constant `FLUSH_TIMEOUT_MS = 5000`), logging failures at warn — never
  hanging exit, never propagating. Batch buffering is the SDK default
  (bounded queue; `OTEL_BSP_*` env-tunable by operators), which satisfies
  the bounded-buffer edge case without bespoke knobs.

## D8 — Known 0.32 traps the spike must confirm (recorded so they aren't re-learned)

1. No `with_end_time` on `SpanBuilder` — end via `end_with_timestamp`.
2. Schemeless endpoints now default to **https** — local collectors need
   an explicit `http://` (documented in quickstart + README).
3. `OTEL_SDK_DISABLED` is app-side (D2).
4. Background-thread batch processors require the **blocking** reqwest
   client.
5. OTel internal diagnostics flow through `tracing` (`internal-logs`
   feature; `global::set_error_handler` no longer exists) — they inherit
   our stderr subscriber; default the noise down via the existing
   `EnvFilter` (`opentelemetry*=warn`).
6. Shutdown must run before tokio teardown; abrupt exit drops the queue
   tail.

## Test strategy (Principle IV without a bespoke seam)

- **Unit/integration**: a test constructor injects the SDK's
  `InMemorySpanExporter` / `InMemoryMetricExporter` (dev-only `testing`
  feature) — emission logic is asserted span-by-span against record
  structs with no network. The export boundary itself is the SDK's own
  exporter abstraction; wrapping it in a Parallax trait would mock code we
  don't own and is deliberately not done (named).
- **Disabled-path inertness**: with no endpoint env, assert no providers
  exist and emission points are no-ops (SC-003's egress half; the latency
  half is a benchmark note in acceptance).
- **Wire path**: spike S1 + an example run against a wiremock OTLP/HTTP
  double (`POST /v1/traces`, `/v1/metrics` with protobuf bodies — sound
  but roll-your-own; no canonical upstream example exists, flagged) and/or
  a local collector.

## Spike S1 — `examples/spike_otlp.rs` (gates everything)

Build the full path on 0.32 for real: gated init from env → emit one
invocation span + metrics from a synthetic record → force_flush →
shutdown_with_timeout, against a wiremock double, asserting (a) requests
arrive at `/v1/traces` and `/v1/metrics` when enabled, (b) zero requests
when the endpoint env is absent, (c) the two flagged uncertainties
(`semconv_experimental` gating; retroactive timestamps accepted).
