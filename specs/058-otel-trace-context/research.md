# Research: Join the caller's trace

## D1 — The keys

**Decision**: read `traceparent` and `tracestate` from the request's `_meta`,
unprefixed, as strings. Do not read `baggage`. A non-string value is absent.

**Rationale**: the MCP 2026-07-28 changelog names exactly these keys: "Document
OpenTelemetry trace context propagation conventions for `_meta` keys
(`traceparent`, `tracestate`, `baggage`)." They carry no
`io.modelcontextprotocol/` prefix, unlike other `_meta` keys in that release.

**Alternatives**: also reading `baggage` (out of scope; it would let a caller
put arbitrary values on our spans); reading prefixed keys (not in the spec).
SEP-414, the source of the changelog entry, was not read.

## D2 — Parsing

**Decision**: parse with `opentelemetry_sdk::propagation::TraceContextPropagator`,
fed a two-entry map. An invalid or all-zero `traceparent` yields an empty
context, which gives a root span.

**Rationale**: the SDK implements W3C Trace Context validation already, under
the `trace` feature the crate enables. Hand-parsing would be a new scanner, the
class of code this project has repeatedly found defects in.

**Alternatives**: a hand-written parser (rejected above); a new crate (not
needed).

## D3 — How the parent reaches the emit point

**Decision**: each tool handler reads `context.meta` once and builds an
`Option<CallerTrace>`. It travels into `run_recorded`/`run_recorded_usage`,
is held by `RecordGuard`, and is passed by `finish()` and by the `Drop`
(cancelled) path to `InvocationRecord::publish`, which passes it to
`emit_invocation`. It is never a field of `InvocationRecord`.

**Rationale**: `InvocationRecord` is the stored row (FR-006: no schema
change), and `RecordDims` is `Copy`, which a span context is not. `publish`
stays the single door (007 FR-009), now taking the parent.

**Alternatives**: a non-persisted field on `InvocationRecord` (blurs the stored
record); a task-local (implicit, and the cancelled path runs in `Drop`, where
task-locals are unreliable).

## D4 — Checkpoint tools

**Decision**: the checkpoint tool handlers pass their `CallerTrace` through the
checkpoint run to `emit_checkpoint`, which starts its span under it.

**Rationale**: FR-005; a checkpoint evaluation span is emitted per tool call.

**Alternatives**: leaving checkpoint spans as roots (an inconsistent rule
across tools).

## D5 — What else changes on the span

**Decision**: nothing. Name, kind (`Client`), attributes, status and
timestamps are unchanged; only the parent context differs.

**Rationale**: FR-003 parity for root spans, and the smallest change for
child spans.

**Alternatives**: switching kind to `Server` for parented spans (a second
behavior change; out of scope).

## D6 — Sampling

**Decision**: follow the caller. `init_with_exporters` sets no sampler, so the
SDK default `ParentBased(AlwaysOn)` (`opentelemetry_sdk` 0.33.1,
`trace/config.rs:33`) applies: a sampled-off `traceparent` means no span;
metrics, emitted separately from the record, still count the call. Operators
override with the standard `OTEL_TRACES_SAMPLER`.

**Rationale**: W3C and OTel treat the flag as the caller's decision, so a trace
is kept or dropped whole. Spec amended (FR-009, commit 2b7b3d7).

**Alternatives**: forcing `AlwaysOn` in code (overrides the operator's
variable); parenting only sampled contexts (orphan root spans).

## D7 — Tests

**Decision**: tests first with the in-memory span exporter via
`init_with_exporters`:

- a valid `traceparent` gives the same trace id and that span as parent;
- a valid `tracestate` is carried;
- a failing call is still parented;
- no keys, `tracestate` alone, `baggage` alone, malformed and all-zero values
  give a root span identical to today's;
- a sampled-off flag gives no span and one metric count;
- one test per tool (SC-003), driven through the MCP client in
  `tests/trace_context.rs` (helpers shared from `tests/common/mod.rs`), plus
  the checkpoint span.

**Rationale**: SC-001 to SC-004 are each checked by a named test.

## D8 — Contract and design corpus

**Decision**: add `specs/058-otel-trace-context/contracts/trace-context.md`;
add a "Trace context" section to `specs/007-observability-layer/contracts/telemetry.md`
that states the rule and links it; remove "trace-context propagation from the
calling harness" from that contract's deferrals. (A dated amendment to 007's
spec was planned and dropped: its FR-011 never named this deferral, T014.)

**Rationale**: Constitution I and FR-008.
