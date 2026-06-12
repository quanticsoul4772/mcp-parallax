# Feature Specification: Observability Layer — OTLP Export

**Feature Branch**: `007-observability-layer`

**Created**: 2026-06-12

**Status**: Draft

**Input**: User description: "Observability layer: OTLP export per SDK_LANDSCAPE.md §observability — OpenTelemetry traces and metrics for every tool invocation and checkpoint evaluation (latency, tokens, cost, outcome, model, verdict), GenAI semantic conventions, exported via OTLP with the existing per-call records as the source of truth; off by default, enabled by the standard OTEL env vars."

## The problem this solves

Every Parallax invocation already leaves one durable record (tool, model,
tokens, cost, latency, outcome) and every checkpoint evaluation leaves one
audit row (boundary, signals, verdict, suppression) — but they live only in
the server's local database. An operator who wants to watch the server's
health, spend, latency, or the checkpoint layer's catch-rate-vs-noise balance
has to query a SQLite file by hand, per machine. This feature exports the
same facts as standard telemetry — traces and metrics any OpenTelemetry
backend ingests — so operators see Parallax in the dashboards, alerts, and
cost views they already run, using the industry's GenAI naming conventions so
existing LLM-observability tooling works without translation. Telemetry is
strictly additive and strictly derived: the local records remain the source
of truth, behavior never changes, and with the feature disabled (the
default) nothing is emitted and nothing slows down.

## Clarifications

### Session 2026-06-12

- Q: Is inheriting the standard OTel env-var contract a deliberate-enough
  enablement (Constitution VI), given MCP child processes inherit the
  user environment? → A: Yes — standard vars only, no Parallax-specific
  gate. The inheritance behavior is documented prominently (README +
  integration docs) rather than gated; FR-008's metadata-only guarantee
  bounds what an inherited enablement can ever export.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Watch every invocation in an existing telemetry backend (Priority: P1)

An operator runs Parallax alongside other instrumented services and points it
at their existing telemetry collector via the standard environment variables.
From then on, every tool invocation — a `verify` ensemble, a `research` run,
a `check`, a memory call — appears in their backend as a trace span carrying
the invocation's tool name, model, token counts, cost, latency, and outcome
class, plus matching metrics (invocation counts by tool/outcome, latency
distributions, token and cost counters). The operator builds a spend
dashboard and an error-rate alert without writing any Parallax-specific
ingestion code.

**Why this priority**: This is the feature's core value — the existing
per-invocation record, visible where operators already look. Every other
story builds on the same export pipeline.

**Independent Test**: Run the server with telemetry enabled against a local
collector; invoke tools (success and failure paths); the collector receives
one span per invocation with the record's fields as attributes and the
metrics reflect the same counts. Disable telemetry; nothing is received.

**Acceptance Scenarios**:

1. **Given** telemetry is enabled and a collector is reachable, **When** a
   tool invocation completes (any outcome class), **Then** the collector
   receives exactly one span for it carrying tool, model, input/output
   tokens, cost, latency, and outcome — matching the stored invocation
   record's values exactly.
2. **Given** telemetry is enabled, **When** invocations complete, **Then**
   invocation-count, latency, token, and cost metrics are exported with
   tool/model/outcome attributes sufficient to chart spend per tool and
   error rate per outcome class.
3. **Given** telemetry is not configured (the default), **When** the server
   runs a full session, **Then** no telemetry network egress occurs at all
   and no measurable per-invocation overhead is added.
4. **Given** the model/token naming conventions of the GenAI observability
   standard, **When** spans are inspected in a standards-aware backend,
   **Then** model and token fields appear under the standard attribute names
   (alongside any Parallax-specific ones).

---

### User Story 2 - Measure the checkpoint layer's precision in a dashboard (Priority: P2)

An operator running the checkpoint sensor plane wants the design's
make-or-break number — catch rate vs noise — continuously visible instead of
hand-run SQL. With telemetry enabled, every checkpoint evaluation exports a
span/metrics set carrying the boundary, the verdict, which signals fired,
whether the delivery was cooldown-suppressed, whether the evaluation failed
open, and the review-hop usage when one ran. The operator charts flag rate,
hold rate, suppression rate, and fail-open rate per boundary over weeks of
real sessions, and alerts if noise creeps up.

**Why this priority**: The checkpoint layer's own design names per-trigger
trace events as the alarm-fatigue measurement mechanism
(`WATCHDOG_LAYER.md`); this story is that mechanism, exported. It reuses
US1's pipeline.

**Independent Test**: With telemetry enabled, drive checkpoint evaluations
producing each verdict (silence, flag, hold, suppressed, fail-open); the
collector receives one span per evaluation with boundary/verdict/suppression
attributes, and the per-boundary rates computed from the exported metrics
match the same rates computed from the local audit rows.

**Acceptance Scenarios**:

1. **Given** telemetry is enabled, **When** a checkpoint evaluation completes
   with any verdict, **Then** the collector receives exactly one span for it
   carrying boundary, verdict, fired signal kinds, suppressed, fail-open,
   and latency — matching the stored checkpoint record.
2. **Given** a set of evaluations spanning all verdicts, **When** per-boundary
   flag/hold/suppression/fail-open rates are computed from exported metrics,
   **Then** they equal the rates computed from the local records.

---

### User Story 3 - Telemetry can never hurt the server (Priority: P3)

An operator's collector goes down mid-session, or was never reachable, or the
server exits while telemetry is buffered. Nothing about Parallax's behavior
changes: invocations succeed exactly as before, no error reaches the calling
model or the user, nothing is ever written to the protocol channel, and on a
clean shutdown buffered telemetry is flushed within a bounded time rather
than lost or hanging the exit.

**Why this priority**: The export path touches every invocation; its failure
modes must be proven harmless before anyone trusts it on in their
environment. It hardens what US1/US2 built.

**Independent Test**: Run with telemetry enabled and no collector
(unreachable endpoint): all invocations behave identically to a
telemetry-disabled run, the protocol channel carries only protocol frames,
and shutdown completes within the bounded flush window.

**Acceptance Scenarios**:

1. **Given** telemetry is enabled but the collector is unreachable, **When**
   tools are invoked, **Then** every invocation completes with results and
   records identical to a telemetry-disabled run, and no telemetry error
   surfaces to the caller.
2. **Given** buffered telemetry at shutdown, **When** the server exits
   cleanly, **Then** export is flushed within a bounded time and the process
   exits without hanging.
3. **Given** any telemetry state (enabled, disabled, failing), **When** the
   server runs, **Then** the protocol channel carries only protocol frames —
   telemetry and its diagnostics never touch it.

---

### Edge Cases

- **Partially configured environment**: an endpoint variable present but
  malformed (unparseable URL) is a startup configuration error, named and
  loud — never a silent fall-back to disabled.
- **Secrets in attributes**: exported attributes carry only record fields
  (names, numbers, classes) — never API keys, claim/query text, memory
  content, or transcript content. Telemetry must not become an exfiltration
  channel for inputs the records themselves don't store.
- **Slow collector**: export buffering is bounded; a slow or backed-up
  collector drops telemetry rather than growing memory without limit or
  back-pressuring invocations.
- **Cancelled/abandoned invocations**: these already produce a record; they
  export like any other outcome class — the cancelled class must be visible
  in dashboards, not silently absent.
- **Timing source**: span timings come from the same measurements the
  records store — the two surfaces cannot disagree.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST export one trace span per completed tool
  invocation and per checkpoint evaluation, carrying the same fields the
  corresponding stored record carries (tool/boundary, model, token counts,
  cost, latency, outcome/verdict, and for checkpoints: fired signal kinds,
  suppressed, fail-open, review-ran).
- **FR-002**: The system MUST export metrics sufficient to compute, without
  access to the local database: invocation counts by tool and outcome,
  latency distributions by tool, token and cost totals by tool and model,
  and checkpoint evaluation counts by boundary and verdict (including
  suppressed and fail-open).
- **FR-003**: Model and token-usage attributes MUST follow the GenAI
  semantic conventions' standard names where they exist, with
  Parallax-specific fields namespaced alongside them.
- **FR-004**: Telemetry MUST be off by default and enabled exclusively
  through the standard OpenTelemetry environment variables (endpoint et
  al.) — no Parallax-specific switch, no code change, no config file. A
  present-but-malformed telemetry variable is a startup error, never a
  silent fallback (the existing config convention). Because these
  variables are shared across OTel-aware processes by design, a globally
  set endpoint enables Parallax too — this inheritance MUST be documented
  prominently in the operator-facing docs (clarified 2026-06-12; FR-008
  bounds what an inherited enablement can export).
- **FR-005**: With telemetry disabled, the system MUST add no measurable
  per-invocation overhead and make no telemetry-related network egress.
- **FR-006**: Telemetry failures (unreachable collector, export errors,
  buffer overflow) MUST NOT affect invocation behavior, results, records,
  or the error surface — telemetry is fire-and-forget with bounded
  buffering, and its diagnostics go to the existing diagnostic channel
  only.
- **FR-007**: The protocol channel MUST carry only protocol frames under
  every telemetry state — enabled, disabled, or failing.
- **FR-008**: Exported attributes MUST be limited to the fields the records
  store: no request/response text, no claim/query/memory/transcript
  content, no credentials.
- **FR-009**: The stored records remain the canonical source of truth;
  telemetry MUST be derived from the same values at the same exit points so
  the two surfaces cannot disagree (one measurement, two sinks).
- **FR-010**: On clean shutdown the system MUST flush buffered telemetry
  within a bounded time and MUST NOT hang the process if the collector is
  unreachable.
- **FR-011**: Named deferrals: log export (diagnostics already flow to the
  local diagnostic channel), per-phase child spans inside multi-step tools
  (research phases, verify ensemble passes), and any dashboard/UI
  artifacts. v1 exports invocation- and evaluation-level spans and metrics
  only.

### Key Entities

- **Invocation span**: the per-tool-call telemetry twin of an invocation
  record — same identity, same fields, plus the standard GenAI attribute
  names for model and token usage.
- **Checkpoint span**: the per-evaluation telemetry twin of a checkpoint
  record — boundary, verdict, signal kinds, suppressed, fail-open,
  review-ran, latency.
- **Metric instruments**: counters (invocations, tokens, cost, checkpoint
  verdicts) and histograms (latency), attributed by tool/model/outcome or
  boundary/verdict.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With telemetry enabled against a reference collector, 100% of
  completed invocations and checkpoint evaluations produce exactly one span
  each, and every span's attribute values equal the corresponding stored
  record's values.
- **SC-002**: Spend per tool, error rate per outcome class, and per-boundary
  checkpoint flag/hold/suppression/fail-open rates are each computable from
  the exported telemetry alone, and match the same figures computed from
  the local records over the same run.
- **SC-003**: With telemetry disabled, a benchmark run shows no measurable
  per-invocation latency difference versus the prior release, and zero
  telemetry network egress.
- **SC-004**: With an unreachable collector, 100% of invocations in a test
  session complete with results, records, and error surfaces identical to a
  telemetry-disabled run, and clean shutdown completes within the bounded
  flush window.
- **SC-005**: An exported-attribute audit over a full test session finds no
  input text, memory content, transcript content, or credential material —
  only record-field names, numbers, and classes.
- **SC-006**: A standards-aware backend resolves the model and token fields
  via the GenAI semantic-convention names without custom mapping.

## Assumptions

- Enablement follows the OpenTelemetry SDK's standard environment-variable
  contract (exporter endpoint/protocol/headers); "endpoint unset = disabled"
  is the off-by-default mechanism, consistent with Constitution VI (network
  egress gated on explicit operator configuration).
- v1 exports traces and metrics; log export is a named deferral (FR-011) —
  the existing diagnostic stream already serves local debugging.
- Span granularity is one span per invocation / per checkpoint evaluation
  (matching the one-record-per-call spine); intra-invocation child spans
  are a named deferral (FR-011).
- The cancelled/abandoned outcome class exports like any other (it already
  produces a record).
- Verification uses a local reference collector in tests/examples; no
  hosted backend is required to accept the feature.
- "Bounded flush window" and buffer limits are fixed at planning as
  constants, consistent with how prior layers fixed tunables.
