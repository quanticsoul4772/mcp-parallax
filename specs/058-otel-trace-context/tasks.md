# Tasks: Join the caller's trace

**Input**: Design documents from `specs/058-otel-trace-context/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/trace-context.md, quickstart.md

**Tests**: required. Every behavior change gets a test written to fail first
(project rule). Tests that pin existing behavior are marked as pins.

**Organization**: by user story. Both stories are P1; US2 is mostly pins that
hold once US1 lands.

The 15 tools: `verify`, `unstick`, `diverge`, `decide`, `elicit`, `check`,
`save`, `recall`, `forget`, `surface`, `research`, `grounded_verify`,
`checkpoint_action`, `checkpoint_batch`, `checkpoint_turn`. The gated ones are
served in tests through the existing helpers (`serve_with_memory`,
`serve_with_research`, `serve_with_grounded` in `tests/common/mod.rs`, moved
there from `tests/integration.rs`).

## Phase 1: Setup

None: no new dependency (`opentelemetry_sdk`'s `trace` feature already
provides the W3C propagator, research D2).

## Phase 2: Foundational (blocks both stories)

- [X] T001 Write failing unit tests for `CallerTrace` parsing from a request `_meta` map in `src/observability.rs`: valid `traceparent`; valid `traceparent` + valid `tracestate`; valid `traceparent` + invalid `tracestate` (kept parent, dropped state); malformed and all-zero `traceparent`; non-string values; `tracestate` only; `baggage` only (all of these `None` or root); sampled-off flag preserved in the parsed context
- [X] T002 Implement `CallerTrace::from_meta` in `src/observability.rs` with `opentelemetry_sdk::propagation::TraceContextPropagator` over the two keys only (research D1, D2)
- [X] T003 Write failing unit tests in `src/observability.rs` that `emit_invocation` and `emit_checkpoint` start their span under a given `CallerTrace` (same trace id, parent span id), and that with none the span is identical to today's (name, kind, attributes, status)
- [X] T004 Add a caller-trace parameter to `emit_invocation` and `emit_checkpoint` in `src/observability.rs` and to `InvocationRecord::publish` in `src/telemetry.rs` (still the only door, research D3); every existing caller passes none, so behavior is unchanged

**Checkpoint**: parsing and parenting work at the emit level; no tool passes a parent yet.

## Phase 3: User Story 1 — A Parallax call appears inside the caller's trace (P1)

**Goal**: every tool's span is a child of the trace context its request carries.

**Independent test**: call a tool with a `traceparent` in `_meta` against the in-memory exporter and read the span's trace id and parent.

- [X] T005 [US1] Create `tests/trace_context.rs` (its own process, so its own `init_with_exporters`): call each of the 15 tools through the MCP client with a distinct `traceparent` in `_meta`, using the doubles and helpers shared in `tests/common/mod.rs`, and assert each `parallax.<tool>` span (and the checkpoint evaluation span for the three checkpoint tools) has that trace id and parent; write it to fail
- [X] T006 [US1] Carry `Option<CallerTrace>` in `RecordGuard` in `src/server/record.rs` and pass it to `publish` from both `finish()` and the cancelled-on-drop path
- [X] T007 [US1] In `src/server.rs`, have every tool handler build `CallerTrace::from_meta(&context.meta)` once and pass it through `run_recorded` / `run_recorded_usage` to `RecordGuard`
- [X] T008 [US1] Pass the caller trace from the checkpoint tool handlers through `src/checkpoint/run.rs` to `emit_checkpoint` (research D4)
- [X] T009 [US1] Add tests to `tests/trace_context.rs` that a failing call (e.g. invalid input) and a `tracestate`-carrying call are parented, and that the `tracestate` is on the span context; make T005 and T009 pass

**Checkpoint**: US1 complete; every tool joins the caller's trace.

## Phase 4: User Story 2 — Nothing changes without trace context (P1)

**Goal**: callers who send nothing, or garbage, see today's telemetry.

**Independent test**: calls with no or invalid keys produce root spans identical to before.

- [X] T010 [US2] Add tests to `tests/trace_context.rs` (pins after US1): no `_meta`, `_meta` without `traceparent`, malformed and all-zero `traceparent`, and `tracestate`/`baggage` alone each give a root span whose attributes match an unparented call of the same tool
- [X] T011 [US2] Add a unit test in src/observability.rs that a sampled-off `traceparent` exports no span while `parallax.invocations` still counts the call (FR-009)
- [X] T012 [US2] Add a test that tool results and the stored invocation record are identical with and without trace context (SC-004)
- [X] T012a [US2] Create tests/trace_context_export_off.rs (its own process, never initializes telemetry): a call with traceparent returns the same result and stores the same record as one without (FR-007)
- [X] T012b [US2] Add a unit test in src/observability.rs that consolidation and push spans start under a given caller trace (spec edge case; flipped from "have no parent" by review R1, with an end-to-end test in tests/trace_context.rs)

**Checkpoint**: US2 complete.

## Phase 5: Polish & Cross-Cutting

- [X] T013 Add a "Trace context" section to `specs/007-observability-layer/contracts/telemetry.md` linking `specs/058-otel-trace-context/contracts/trace-context.md`, and remove "trace-context propagation from the calling harness" from its deferrals (FR-008)
- [X] T014 Add a dated amendment to `specs/007-observability-layer/spec.md` where FR-011 names the deferral — not needed: FR-011 never named trace-context propagation; only the telemetry contract did, and T013 amends it.
- [X] T015 Add an "Added" entry to `CHANGELOG.md` under Unreleased
- [X] T016 Run the full gate (`cargo fmt --all -- --check`, `cargo clippy --all-features --all-targets -- -D warnings`, `cargo test --all-features --tests`)
- [X] T017 Independent review of the branch diff by a fresh subagent; act on findings
- [X] T018 Update the CLAUDE.md active-feature block for 058

## Deferred

- T019 (optional) live check (`quickstart.md`): with a collector configured,
  see whether a Claude Code session sends `traceparent`; record the result in
  `quickstart.md`. Deferred 2026-11-21: it needs the MCP server restarted with
  OTLP export configured and a collector running; the parenting itself is
  covered by `tests/trace_context.rs`.

## Dependencies & Execution Order

- Phase 2 blocks everything: T001 → T002, T003 → T004.
- US1: T005 first (red), then T006 → T007 → T008, then T009.
- US2 depends on US1 (it pins behavior US1 must not disturb).
- Polish after both stories; T016 before T017; T018 after T017.

## Parallel Opportunities

- T001 and T003 touch the same file; write them in sequence.
- T013, T014 and T015 are documentation in different files and can be done in any order.

## Implementation Strategy

MVP is Phase 2 + US1: spans join the caller's trace. US2 then pins that
callers without trace context see no change. One commit per phase at least,
each after the full gate.
