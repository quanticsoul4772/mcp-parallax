# Implementation Plan: Observability Layer — OTLP Export

**Branch**: `007-observability-layer` | **Date**: 2026-06-12 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/007-observability-layer/spec.md`

## Summary

OTLP export of what the server already measures: one trace span per tool
invocation and per checkpoint evaluation plus matching metrics, derived as
**pure functions of the existing record structs at the exact code points
where the records are written** (`RecordGuard::finish`, checkpoint
`record()`) — one measurement, two sinks, the surfaces cannot disagree
(FR-009). Spans are emitted retroactively (start = `created_at − latency`,
ended with an explicit timestamp), carry GenAI semantic-convention names for
model/token fields plus a `parallax.*` namespace for the rest, and are
root-level (no accidental parenting). Off by default: telemetry initializes
only when a standard OTLP endpoint variable is present and `OTEL_SDK_DISABLED`
is not set (the SDK doesn't implement that variable — we honor it
app-side, a named upstream gap). Fire-and-forget with bounded batching;
flush + bounded-timeout shutdown on exit; all diagnostics ride the existing
stderr `tracing` subscriber — nothing can touch stdout.

## Technical Context

**Language/Version**: Rust (pinned stable via `rust-toolchain.toml`, MSRV 1.94)

**Primary Dependencies** (research.md D1, all one release train, pinned
together): `opentelemetry` 0.32, `opentelemetry_sdk` 0.32.1 (traces+metrics;
`testing` feature as dev-dependency), `opentelemetry-otlp` 0.32
(`http-proto` + `reqwest-blocking-client` + `reqwest-rustls-webpki-roots`,
no tonic/protoc), `opentelemetry-semantic-conventions` 0.32. **No
`tracing-opentelemetry` / appender bridge** — spans are constructed directly
from records, not bridged from `tracing` (a named deviation from
`SDK_LANDSCAPE.md`'s crate list, amended in the same change).

**Storage**: none new — records remain the canonical store; telemetry reads
the record structs in memory at write time.

**Testing**: in-memory span/metric exporters (`opentelemetry_sdk` `testing`
feature) injected through a test constructor — emission logic tests without
network; integration test for the disabled path (no providers, no egress);
spike + example against a wiremock OTLP/HTTP double and/or a local collector
for the env-driven wire path.

**Target Platform**: unchanged (stdio binary, Windows dev / Linux CI).

**Performance Goals**: disabled = one atomic-bool check per emission point
(SC-003); enabled = attribute assembly + queue push per record (batch
processor exports on its own background thread with the blocking reqwest
client — the documented pairing).

**Constraints**: stdout carries only protocol frames in every state (FR-007
— OTel 0.32 has no separate diagnostic channel: internals log via `tracing`,
which already goes to stderr); export failures never surface to callers
(FR-006); attributes limited to record fields (FR-008); flush + shutdown
bounded (FR-010, `OTEL_FLUSH_TIMEOUT` constant 5 s).

**Scale/Scope**: two span kinds, ~6 metric instruments, one new module;
logs export / child spans / dashboards are named deferrals (FR-011).

## Constitution Check

*GATE: evaluated against constitution v1.0.0 before Phase 0; re-checked after
Phase 1 design.*

| Principle | Status | Notes |
|---|---|---|
| I. Design-corpus fidelity | PASS | Implements `SDK_LANDSCAPE.md` §observability (OTel + OTLP, GenAI semconv as standard span attributes, "one instrumentation, two consumers" — here: one measurement, records + OTLP). Named deviations: `tracing-opentelemetry`/`opentelemetry-appender-tracing` dropped (no bridge needed when spans derive from records; corpus amended in the same change); OTLP arrives in v0.1.0+ rather than "from the first server commit" (sequencing already chosen at 001, recorded then). |
| II. Constrained-output contract | PASS | No model hops in this feature; nothing to constrain. |
| III. Compiler discipline | PASS | No unsafe; stdout untouched by construction (OTel diagnostics ride the existing stderr `tracing` subscriber; `set_error_handler` no longer exists upstream); export errors logged at warn, never propagated. |
| IV. Seams + tests | PASS | The emission layer is pure functions of record structs (testable with zero infrastructure); the export boundary uses the SDK's own exporter abstraction with in-memory exporters injected in tests — the seam exists upstream, no eighth trait needed (named: wrapping the OTel SDK in a bespoke trait would mock what we don't own and test nothing). |
| V. Deterministic over probabilistic | PASS | Pure derivation; no judgment anywhere. |
| VI. Capabilities off by default | PASS | Network egress gated on explicit operator configuration (endpoint env present, `OTEL_SDK_DISABLED` honored app-side); absent → no providers built, no egress, one atomic check of overhead. The env-inheritance consequence is documented prominently (spec clarification 2026-06-12). Malformed endpoint = startup `ConfigError`, never silent (config convention). |
| VII. Simplicity / ≤500-line modules | PASS | One `src/observability.rs` module; constants over config subsystem; v1 surface minimal (FR-011 deferrals). |

**Post-Phase-1 re-check**: PASS — the telemetry contract introduces no new
violations.

## Project Structure

### Documentation (this feature)

```text
specs/007-observability-layer/
├── plan.md              # This file
├── research.md          # Phase 0 output (decisions D1–D8, spike S1)
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── telemetry.md     # the exported surface: span names, attributes, metrics
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── observability.rs      # init-from-env (gating, resource, providers, instruments),
│                         # emit_invocation(&InvocationRecord), emit_checkpoint(&CheckpointRecord),
│                         # shutdown (flush + bounded timeout); ENABLED fast-path flag
├── main.rs               # init after config load; shutdown before exit
├── server/record.rs      # RecordGuard::finish → emit_invocation (same exit point as the DB write)
└── checkpoint/run.rs     # record() → emit_checkpoint (same exit point as the DB write)

tests/integration.rs      # disabled-path inertness; emission via injected in-memory exporters
examples/spike_otlp.rs    # S1: env gating + live OTLP round trip (wiremock double / local collector)
```

**Structure Decision**: single new module; emission calls live exactly at the
two record-write exit points (FR-009). Instruments and providers are held in
a process-global handle (the OTel-idiomatic global) guarded by a static
enabled flag so the disabled path is one atomic load.

## Complexity Tracking

No constitution violations to justify. One named engineering risk: the
opentelemetry-rust 0.32 API was verified by web research (research.md
sources), not yet by compilation — **spike S1** (`examples/spike_otlp.rs`)
builds the full init→emit→flush path first and is the gate for everything
else; the known 0.32 traps it must confirm are recorded in research.md D8
(no `with_end_time` on the builder — end via `end_with_timestamp`;
schemeless endpoints defaulting to https; `OTEL_SDK_DISABLED` being
app-side; the blocking-client/batch-processor pairing).
