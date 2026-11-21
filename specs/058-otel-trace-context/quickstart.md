# Quickstart: verifying trace-context parenting

## Automated

```bash
cargo test --all-features --tests trace_context
```

The tests use the SDK's in-memory span exporter, so no collector is needed.

## Manual, with a local collector

1. Run any OTLP collector locally (for example the OpenTelemetry Collector or
   Jaeger) and set `OTEL_EXPORTER_OTLP_ENDPOINT` for the Parallax server.
2. From an MCP client, call any tool with
   `_meta: { "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01" }`.
3. In the collector, the `parallax.<tool>` span has trace id
   `4bf92f3577b34da6a3ce929d0e0e4736` and parent span id `00f067aa0ba902b7`.
4. Repeat with the flag `-00` instead of `-01`: no span appears, and the
   `parallax.invocations` counter still increases.
5. Call without `_meta`: the span is a new root trace, as before.

## Live check against a real client

Whether Claude Code sends `traceparent` is not known. With a collector
configured, run a session and look for `parallax.*` spans with a parent; if
every span is a root, the client does not send trace context.
