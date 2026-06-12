# Implementation Plan: Core Layer — Working Server with First Corrective (Verify)

**Branch**: `001-core-layer` | **Date**: 2026-06-11 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/001-core-layer/spec.md`

## Summary

Make the scaffold a working MCP server: rmcp stdio transport exposing one tool
(`verify`), backed by a thin Anthropic structured-outputs client implementing the
existing `ModelClient` seam, with a schema sanitizer (schemars → Anthropic grammar
subset) and a defense-in-depth validator enforcing what the grammar can't. Verify
runs k independent, stance-blind model passes (default 3) and derives confidence
from ensemble agreement. Every invocation persists one record (tool, model,
tokens, cost, latency, outcome, session) through the `Storage` seam into SQLite.
Four pre-build spikes validate the load-bearing glue first.

## Technical Context

**Language/Version**: Rust, edition 2021, MSRV 1.94 (pinned; CI-verified)

**Primary Dependencies**: `rmcp` 1.x (`server`, `macros`, `transport-io`,
`schemars` features), `reqwest` (thin Anthropic client, rustls), `schemars` 1.x,
`jsonschema` (local validator), `sqlx` (sqlite, runtime-tokio) for invocation
records, existing `tokio`/`serde`/`thiserror`/`tracing`. Dev: `mockall`
(existing), `wiremock` (HTTP stub for the client tests).

**Storage**: SQLite at `DATABASE_PATH` (existing config), via the `Storage` trait
— invocation records only in this feature (stateless server; no session memory)

**Testing**: `cargo test` — unit tests with `mockall` mocks of the three seams;
client tests against a `wiremock` local stub (no network); integration test via
an in-process rmcp client round-trip (stdio not required in tests)

**Target Platform**: Single binary; Windows / macOS / Linux (developed on
Windows; CI on Linux)

**Project Type**: Single Rust crate (lib + bin), MCP stdio server

**Performance Goals**: One Verify call completes in <30s at defaults (SC-006);
server-side overhead (sanitize, validate, record) <50ms per call — model latency
dominates

**Constraints**: stdout is the JSON-RPC channel (compiler-denied prints); no
panics in production paths; mode schemas flat + closed (Anthropic grammar subset:
no numeric/length constraints, no recursion — validator covers those); grammar
cache favors stable schemas (modes are data, schemas don't churn per call)

**Scale/Scope**: Single-user dev tool; a handful of concurrent invocations; one
tool in this feature, but the mode registry must make tool #2 a data addition,
not a new subsystem

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Gate | Status |
|---|---|---|
| I. Design-corpus fidelity | Stack matches `SDK_LANDSCAPE.md` §core (rmcp, thin reqwest client, schemars + jsonschema) and `SDK_USAGE_CORE.md` wiring; the four named spikes run before core is built; no silent deviations (deviations: none) | ✅ PASS |
| II. Constrained-output contract | One schemars-derived schema feeds both hops; sanitizer forces `additionalProperties: false`; validator enforces stripped constraints; `stop_reason` checked before parsing; no free-text fallback anywhere | ✅ PASS |
| III. Compiler-enforced discipline | No new lint exceptions; thin client and storage are `Result`-based; no stdout writes (rmcp owns stdout); lint policy untouched | ✅ PASS |
| IV. Seams, composition, tests | Anthropic client behind `ModelClient`; SQLite behind `Storage`; time behind `TimeProvider`; all tests pass without network (wiremock is localhost-loopback in dev-deps only) or pre-existing disk state; every user story has test tasks | ✅ PASS |
| V. Deterministic over probabilistic | Schema conformance settled by validator (deterministic), never by a model judging "does this look right"; confidence derived from ensemble agreement, not self-report | ✅ PASS |
| VI. Capabilities off by default | Only egress is the configured Anthropic endpoint (the feature's core function, enabled by providing the key); no telemetry export, no other network paths | ✅ PASS |
| VII. Simplicity and scope | One tool, no router, no watchdog/memory/deterministic-layer work; new modules target ≤500 lines; mode registry is a map, not a framework | ✅ PASS |

**Post-Phase-1 re-check**: PASS — design artifacts introduce no new dependencies
or capabilities beyond the table above; the verify output schema in
`contracts/` is flat, closed, and inside the grammar subset.

## Project Structure

### Documentation (this feature)

```text
specs/001-core-layer/
├── plan.md              # This file
├── research.md          # Phase 0 output — decisions + the four spikes
├── data-model.md        # Phase 1 output — entities, schemas, error taxonomy
├── quickstart.md        # Phase 1 output — build, configure, connect, invoke
├── contracts/           # Phase 1 output — verify tool + invocation record schemas
│   ├── verify.tool.json
│   └── invocation-record.schema.json
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
src/
├── main.rs              # wire: config → deps → rmcp serve(stdio()); --version/--help
├── lib.rs               # crate docs + lint preamble (unchanged policy)
├── error.rs             # AppError grows the outcome taxonomy (refusal, truncation,
│                        #   timeout, retries_exhausted, invalid_input, validation_failure)
├── config.rs            # + ANTHROPIC_MODEL, VERIFY_ENSEMBLE_K (defaults; existing vars keep working)
├── traits/              # the existing seams; client.rs confirmed as complete(prompt, schema) → JSON
├── schema/
│   ├── mod.rs
│   ├── sanitize.rs      # schemars output → Anthropic grammar subset (load-bearing glue)
│   └── validate.rs      # jsonschema defense-in-depth (ranges/lengths the grammar drops)
├── client/
│   ├── mod.rs
│   └── anthropic.rs     # thin reqwest client: output_config.format, stop_reason map,
│                        #   retry/backoff (mcp-reasoning's lifted pattern)
├── storage/
│   ├── mod.rs
│   └── sqlite.rs        # sqlx SQLite impl of Storage; invocation_records table + migration
├── modes/
│   ├── mod.rs           # CorrectiveMode (data) + registry
│   └── verify.rs        # VerifyParams/Verdict types, prompt template (calibrated profile),
│                        #   k-pass fan-out + agreement aggregation
├── server.rs            # rmcp handler: #[tool_router], verify tool, per-call recording
└── telemetry.rs         # InvocationRecord construction + tracing span attrs (GenAI names)

tests/
└── integration.rs       # in-process rmcp round-trip; induced-failure matrix (US2);
                         #   record-completeness checks (US3)

examples/
├── spike_sanitizer.rs   # Spike 1 — schema sanitizer fidelity
├── spike_client.rs      # Spike 2 — one real structured-outputs call (manual, needs key)
├── spike_roundtrip.rs   # Spike 3 — rmcp Json<T> outputSchema/structured_content
└── spike_thinking.rs    # Spike 4 — thinking + output_config compatibility (manual)
```

**Structure Decision**: Single crate. New top-level modules (`schema`, `client`,
`storage`, `modes`, `server`, `telemetry`) slot behind the three existing trait
seams; each targets ≤500 lines. Spikes live in `examples/` so they compile under
the same lints but ship nowhere. The two spikes that need a live key
(`spike_client`, `spike_thinking`) are manual-run only — tests never touch the
network.

## Complexity Tracking

> No Constitution Check violations — table intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| — | — | — |
