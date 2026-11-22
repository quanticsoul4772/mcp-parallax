# Feature Specification: Join the caller's trace

**Feature Branch**: `058-otel-trace-context`

**Created**: 2026-11-21

**Status**: Draft

**Input**: User description: "Join the caller's OpenTelemetry trace. MCP 2026-07-28 defines `_meta` keys `traceparent`, `tracestate` and `baggage` for trace-context propagation. When a tool request carries `traceparent` (and optionally `tracestate`) in `_meta`, Parallax's exported invocation span starts as a child of that context instead of a new root trace; with no keys, or invalid ones, spans stay root traces exactly as today. Baggage is not read. Applies to every tool's invocation span; nothing changes when OTLP export is not configured. No SQLite schema change. Update the published telemetry contract deliberately."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A Parallax call appears inside the caller's trace (Priority: P1)

An operator traces an agent run end to end. The agent's harness sends the
current trace context with each tool request. Today every Parallax call shows
up as its own unrelated trace, so the operator cannot see which agent step
called `verify` or what a `research` run cost inside that step. With this
feature, each Parallax call appears as a child of the step that made it.

**Why this priority**: It is the whole feature. Without it the trace context
the protocol now carries is thrown away.

**Independent Test**: Send one tool request whose request metadata carries a
valid trace context, with export directed at a test collector, and confirm the
exported span belongs to that trace and names the sent span as its parent.

**Acceptance Scenarios**:

1. **Given** export is configured and a request carries a valid `traceparent`,
   **When** the tool completes, **Then** its exported span has the same trace
   id as the `traceparent` and the `traceparent`'s span as its parent.
2. **Given** the request also carries a valid `tracestate`, **When** the span
   is exported, **Then** its context carries that `tracestate`.
3. **Given** the tool call fails (any outcome other than success), **When** the
   span is exported, **Then** it is still a child of the sent context.

---

### User Story 2 - Nothing changes for callers that send no trace context (Priority: P1)

Most callers today send no trace context, and some may send malformed values.
Their telemetry must be exactly what it is today.

**Why this priority**: A change to every tool's telemetry must not disturb the
callers who do not use it, or emit spans parented to garbage.

**Independent Test**: Send requests with no metadata, with metadata but no
`traceparent`, and with malformed values; confirm each exported span is a new
root trace with the same attributes it has today.

**Acceptance Scenarios**:

1. **Given** a request with no `traceparent`, **When** the span is exported,
   **Then** it starts a new trace, as today.
2. **Given** a malformed or all-zero `traceparent`, **When** the span is
   exported, **Then** it starts a new trace and the call itself is unaffected.
3. **Given** only `tracestate` or only `baggage` is sent, **When** the span is
   exported, **Then** it starts a new trace.
4. **Given** export is not configured, **When** any request arrives with or
   without trace context, **Then** nothing is exported and tool results and
   stored invocation records are identical to today's.

---

### Edge Cases

- A `traceparent` whose value is not a string: treated as absent.
- A request carrying `baggage`: baggage is not read and does not appear on
  any span.
- A `tracestate` that is malformed while `traceparent` is valid: the span is
  still parented to the `traceparent`; the bad `tracestate` is dropped.
- A sampled-off flag in `traceparent`: the caller chose not to record this
  trace, so no span is exported for the call; metrics still count it.
- Spans a tool call emits besides its invocation span (memory push inside
  `surface`; memory consolidation inside `save` and `checkpoint_turn`) start
  under the same caller trace, so one call's telemetry stays in one trace and
  follows one sampling decision.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: When a tool request's metadata carries a valid `traceparent`,
  the invocation span exported for that call MUST be a child of the context it
  describes.
- **FR-002**: A valid `tracestate` sent with a valid `traceparent` MUST be
  carried on the span's context; `tracestate` alone MUST NOT parent a span.
- **FR-003**: With no `traceparent`, or one that is not a valid trace context,
  the span MUST be a new root trace, identical in name and attributes to the
  span exported before this feature.
- **FR-004**: `baggage` MUST NOT be read.
- **FR-005**: FR-001 to FR-004 MUST hold for every tool's invocation span,
  for the checkpoint evaluation span exported by a checkpoint tool call, and
  for the push and consolidation spans emitted during a tool call.
- **FR-006**: Tool results, errors, and stored invocation and checkpoint
  records MUST be unchanged; no stored schema changes.
- **FR-007**: When export is not configured, behavior MUST be identical to
  today's, whatever the request carries.
- **FR-008**: The published telemetry contract MUST describe the parenting
  rule and remove trace-context propagation from its named deferrals.
- **FR-009**: A valid `traceparent` whose sampled flag is off MUST result in
  no exported span for that call; the call MUST still be counted in metrics.

### Key Entities

- **Caller trace context**: the `traceparent` and optional `tracestate`
  values a request carries in its metadata; read once per request, never
  stored.
- **Invocation span**: the span exported per tool call, unchanged except for
  its parent.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For a request carrying a valid trace context, 100% of exported
  invocation spans share its trace id and name its span as parent.
- **SC-002**: For requests with no or invalid trace context, exported spans
  match the span of the same call made without trace context, attribute for
  attribute, in every tested case.
- **SC-003**: Every tool (all of them, not a sample) is covered by a test that
  checks the parenting rule.
- **SC-004**: Tool results and stored records are byte-identical with and
  without trace context in every tested case.

## Assumptions

- Keys and format follow the MCP 2026-07-28 changelog entry ("Document
  OpenTelemetry trace context propagation conventions for `_meta` keys
  (`traceparent`, `tracestate`, `baggage`)") and the W3C Trace Context format
  those names come from. SEP-414 was not read; if it adds requirements this
  spec misses, that is a follow-up.
- Sampling follows the caller: the tracer's sampler is the SDK default, which
  follows the parent, so a sampled-off `traceparent` means no span for that
  call while metrics still count it. An operator who wants every span sets the
  standard `OTEL_TRACES_SAMPLER=always_on`.
- Whether Claude Code sends these keys is unknown; the feature is useful to any
  harness that does, and inert otherwise.
- Child spans inside an invocation (verify passes, research phases) remain out
  of scope, as the telemetry contract already defers them.
- Outgoing calls Parallax makes (model provider, search, embeddings) do not
  carry the trace context onward; that is a separate feature.
