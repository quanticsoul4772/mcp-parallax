# Data Model: Observability Layer

No new stored entities — telemetry is a derived, in-flight view of the two
existing record types. This file fixes the derivation.

## 1. Constants (`src/observability.rs`)

| Constant | Default | Use |
|---|---|---|
| `FLUSH_TIMEOUT_MS` | 5_000 | per-provider `shutdown_with_timeout` bound (FR-010) |
| `SERVICE_NAME` | `"mcp-parallax"` | `service.name` fallback when env doesn't set one |
| `TRACER_NAME` / `METER_NAME` | `"mcp-parallax"` | instrumentation scope |

Buffering is the SDK batch default (bounded queue; operator-tunable via the
standard `OTEL_BSP_*` / `OTEL_METRIC_EXPORT_INTERVAL` vars) — no bespoke
knobs.

## 2. Gating state

```text
init() -> Result<Option<Guard>, ConfigError>
  enabled  := any of OTEL_EXPORTER_OTLP_ENDPOINT|_TRACES_|_METRICS_ set
              AND OTEL_SDK_DISABLED != "true"        (app-side; SDK gap)
  disabled -> Ok(None): no providers, no exporter, no egress;
              static ENABLED=false → emit_* are one atomic load
  enabled  -> providers built (exporter reads the OTEL_* family itself),
              global ENABLED=true, instruments created once
  malformed endpoint URL -> ConfigError (named variable, loud)
```

`Guard` owns both providers; `Guard::shutdown()` = `force_flush` +
`shutdown_with_timeout(FLUSH_TIMEOUT_MS)` on each, warn-logged, never
propagated, run in `main` before exit.

## 3. Invocation span ← `InvocationRecord` (emitted in `RecordGuard::finish`)

FR-009's "source of truth" is the record **value** at the exit point (the
struct both sinks consume), not the database write: telemetry emits
fire-and-forget even if the row write fails (one sink's availability never
gates the other). SC-001's equality criterion applies where both sinks
landed.

| Span field | Derivation |
|---|---|
| name | `parallax.{tool}` |
| kind | `CLIENT` |
| start | `created_at − latency_ms` |
| end | `created_at` (via `end_with_timestamp`) |
| status | `Ok` iff outcome == success, else `error(outcome.as_str())` |
| context | fresh (`Context::new()`) — root span, no parenting |

| Attribute | Source |
|---|---|
| `gen_ai.operation.name` | `"execute_tool"` |
| `gen_ai.request.model` | `record.model` |
| `gen_ai.usage.input_tokens` / `gen_ai.usage.output_tokens` | token counts (i64) |
| `gen_ai.provider.name` | `"anthropic"` / `"voyageai"` derived from the model id (literal key — crate predates the rename from `gen_ai.system`) |
| `error.type` | `outcome.as_str()` (only when not success) |
| `parallax.tool` | `record.tool` |
| `parallax.outcome` | `outcome.as_str()` |
| `parallax.cost_usd` | `record.cost_usd` (f64) |
| `parallax.session_id` | `record.session_id` |

## 4. Checkpoint span ← `CheckpointRecord` (emitted in checkpoint `record()`)

| Span field | Derivation |
|---|---|
| name | `parallax.checkpoint.{boundary}` |
| kind | `INTERNAL` |
| start/end | `created_at − latency_ms` / `created_at` |
| status | always `Ok` (fail-open is data, not an error — the evaluation completed) |

| Attribute | Source |
|---|---|
| `parallax.checkpoint.boundary` | `boundary.as_str()` |
| `parallax.checkpoint.verdict` | `verdict.as_str()` |
| `parallax.checkpoint.signals` | fired signal kinds (string array) |
| `parallax.checkpoint.suppressed` / `.fail_open` / `.review_ran` | booleans |
| `parallax.checkpoint.cost_usd` | `cost_usd` |
| `parallax.session_id` | `session_id` |

Evidence strings are **not** exported (FR-008: record-field names, numbers,
classes only — evidence quotes trajectory content).

## 5. Metric instruments (created once at init)

| Instrument | Type | Unit | Attributes |
|---|---|---|---|
| `parallax.invocations` | u64 counter | `{invocation}` | tool, model, outcome |
| `parallax.invocation.duration` | f64 histogram | `s` | tool, outcome |
| `parallax.cost` | f64 counter | `USD` | tool, model |
| `gen_ai.client.token.usage` | u64 histogram | `{token}` | `gen_ai.token.type` (input\|output), `gen_ai.request.model`, `gen_ai.provider.name`, `parallax.tool` — GenAI-standard buckets (1, 4, 16, … 67108864) |
| `parallax.checkpoint.evaluations` | u64 counter | `{evaluation}` | boundary, verdict, suppressed, fail_open |
| `parallax.checkpoint.duration` | f64 histogram | `s` | boundary, verdict |

FR-002 check: spend/tool (`parallax.cost`), error rate/outcome
(`parallax.invocations`), latency distribution/tool (`.duration`), token
totals (`gen_ai.client.token.usage`), checkpoint rates incl.
suppressed/fail-open (`parallax.checkpoint.evaluations`). ✓

## 6. Resource

`Resource::builder()` (env detectors active: `OTEL_SERVICE_NAME`,
`OTEL_RESOURCE_ATTRIBUTES` honored) + programmatic
`service.name = "mcp-parallax"` fallback, `service.version` = crate
version, `service.instance.id` = the per-process session UUID (one server
process per harness session — instance identity separates concurrent
sessions in a backend).

## 7. Privacy bound (FR-008 / SC-005)

Exported values are exactly the table cells above: identifiers, enums,
numbers. Never: claim/query text, memory content, transcript content,
checkpoint evidence strings, message text, headers, keys.
