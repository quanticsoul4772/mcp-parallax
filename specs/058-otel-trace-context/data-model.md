# Data Model: Join the caller's trace

## Caller trace context

- **Source**: the tool request's `_meta`, keys `traceparent` and `tracestate`
  (string values only).
- **Read**: once per tool request, in the handler.
- **Form**: `Option<CallerTrace>` — `None` when there is no valid
  `traceparent`; otherwise the parsed remote span context, including a valid
  `tracestate` if one was sent.
- **Lifetime**: the request. Held by the invocation's record guard until the
  span is emitted, on every exit (success, failure, cancellation).
- **Never**: stored, logged, attached as an attribute, or sent onward.

## Invocation span

Unchanged in name, kind, attributes, status and timing. Its parent is the
caller trace context when present, otherwise empty (a root span, as before).

## Checkpoint evaluation span

The same rule, for a checkpoint tool call's evaluation span.

## Unchanged

- `InvocationRecord` and `CheckpointRecord` and their SQLite columns.
- Tool results and error messages.
- Metrics: every call is still counted, including calls whose span the
  caller's sampled-off flag suppresses.
- Consolidation and push spans: root spans (not tied to a request).
