# Contract: Trace-context parenting

Applies only when OTLP export is configured. Linked from
`specs/007-observability-layer/contracts/telemetry.md`.

## Rule

For each tool call, Parallax reads `traceparent` and `tracestate` from the
request's `_meta` (MCP 2026-07-28; W3C Trace Context format).

| Request carries | Exported span |
|---|---|
| a valid `traceparent` | child of that span, same trace id |
| a valid `traceparent` and a valid `tracestate` | as above, carrying that `tracestate` |
| a valid `traceparent` and an invalid `tracestate` | child of the `traceparent`; `tracestate` dropped |
| a valid `traceparent` with the sampled flag off | no span; the call is still counted in metrics |
| no `traceparent`, or an invalid or all-zero one | a new root trace, as before this change |
| `tracestate` or `baggage` only | a new root trace |
| a non-string value for either key | treated as absent |

## Scope

- Spans covered: every tool's invocation span (`parallax.<tool>`) and a
  checkpoint tool call's evaluation span.
- Not covered: consolidation and push spans (no request); child spans inside
  an invocation (still deferred).
- `baggage` is never read.
- Span name, kind, attributes and status are the same as without a parent.
- Outgoing calls Parallax makes do not carry the context onward.
- Operators who want every span regardless of the caller's flag set
  `OTEL_TRACES_SAMPLER=always_on`.
