# Implementation Plan: Join the caller's trace

**Branch**: `058-otel-trace-context` | **Date**: 2026-11-21 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/058-otel-trace-context/spec.md`

## Summary

Every exported Parallax span starts from an empty context today
(`start_with_context(&tracer, &Context::new())`), so each call is a root trace.
MCP 2026-07-28 carries the caller's W3C trace context in request `_meta`
(`traceparent`, `tracestate`). Read those two keys once per tool request,
parse them with the SDK's W3C propagator, carry the result beside the
invocation record to the emit point, and start the invocation span (and a
checkpoint tool's evaluation span) under it. No keys or invalid keys give an
empty context, so the span is exactly today's root span. Sampling follows the
caller through the SDK's default parent-based sampler. Nothing is stored.

## Technical Context

**Language/Version**: Rust 1.100.0 (pinned), MSRV 1.94

**Primary Dependencies**: existing only — `rmcp` (request `_meta` via `RequestContext.meta`), `opentelemetry` 0.33 and `opentelemetry_sdk` 0.33 (`propagation::TraceContextPropagator`, under the already-enabled `trace` feature)

**Storage**: none changed; the caller trace context is never written to SQLite

**Testing**: `cargo test`; the SDK in-memory span exporter through the existing `init_with_exporters` seam

**Target Platform**: the MCP stdio server

**Project Type**: MCP server (library + binary)

**Performance Goals**: one small map read and parse per request; no new I/O

**Constraints**: telemetry failures never propagate (007 FR-006); "one measurement, two sinks" stays one door (`InvocationRecord::publish`); results and records byte-identical (FR-006)

**Scale/Scope**: `src/observability.rs`, `src/telemetry.rs`, `src/server.rs` and `src/server/record.rs` (every tool handler passes the request's trace context), the checkpoint run path, the telemetry contract, a dated amendment to 007's spec, CHANGELOG

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Note |
|---|---|---|
| I. Design-Corpus Fidelity | Pass, with amendment | 007 named trace-context propagation as a deferral; its spec and `contracts/telemetry.md` get a dated amendment in this change (FR-008). |
| II. Constrained-Output Contract | N/A | No mode schema or prompt changes. |
| III. Compiler-Enforced Discipline | Pass | The parent travels as a typed value; no new lint exceptions. |
| IV. Seams, Composition, Tests | Pass | Tests first through the in-memory exporter seam; no new dependency. |
| V. Deterministic Over Probabilistic | N/A | Telemetry only. |
| VI. Capabilities Off By Default | Pass | Inert unless OTLP export is configured and the caller sends the keys. |
| VII. Simplicity and Scope Discipline | Pass | Parent only: no new spans, attributes, baggage, or onward propagation. |

Re-check after Phase 1: unchanged; see research D1–D8.

## Project Structure

### Documentation (this feature)

```text
specs/058-otel-trace-context/
├── plan.md              # This file
├── research.md          # Phase 0 decisions D1–D8
├── data-model.md        # The caller trace context and what stays the same
├── quickstart.md        # How to verify
├── contracts/
│   └── trace-context.md # The parenting rule, as published
├── checklists/
│   └── requirements.md
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
src/observability.rs     # CallerTrace (parse from _meta), emit_invocation/emit_checkpoint start under it
src/telemetry.rs         # publish takes the caller trace; still the only door
src/server/record.rs     # RecordGuard carries the caller trace to finish() and Drop
src/server.rs            # each tool handler reads context.meta once and passes it on
src/checkpoint/run.rs    # the checkpoint path passes the caller trace to emit_checkpoint
specs/007-observability-layer/contracts/telemetry.md   # parenting rule; deferral removed
specs/007-observability-layer/spec.md                  # dated amendment to the FR-011 deferral list
CHANGELOG.md             # Added entry
```

**Structure Decision**: Parse at the edge (the handler, where `_meta` is), carry a typed value, and change only the parent at the two emit points that serve tool calls. Consolidation and push spans keep the empty context.

## Complexity Tracking

None: no principle is violated.
