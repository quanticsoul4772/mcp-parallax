# Quickstart: Observability Layer

## Enable

Nothing Parallax-specific — the standard OpenTelemetry contract is the only
switch:

```bash
# point at your collector (note the explicit http:// for local ones — 0.32
# treats schemeless endpoints as https)
set OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
```

Unset = fully disabled (no providers, no egress, one atomic check of
overhead). `OTEL_SDK_DISABLED=true` force-disables regardless of endpoint.
`OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`, and the full
`OTEL_EXPORTER_OTLP_*` family are honored.

> These variables are shared across OTel-aware processes by design — a
> globally exported endpoint enables Parallax too. What that exports is
> bounded to record metadata (see `contracts/telemetry.md`): no input text,
> no memory/transcript content, no credentials.

## What a backend shows

- One `parallax.{tool}` span per invocation (model/tokens under GenAI
  semantic-convention names; cost, outcome, session under `parallax.*`).
- One `parallax.checkpoint.{boundary}` span per checkpoint evaluation
  (verdict, signal kinds, suppressed, fail-open).
- Metrics for spend per tool, error rates, latency distributions, token
  usage (`gen_ai.client.token.usage`), and per-boundary checkpoint rates —
  the alarm-fatigue dashboard the corpus demands, chartable continuously.

## Spike (gates the build)

```bash
cargo run --example spike_otlp     # S1: gated init -> emit -> flush against a wiremock double
```

## Acceptance

```bash
cargo run --release --example acceptance_otlp
```

Drives invocations + checkpoint evaluations against an in-process OTLP
double and asserts SC-001 (one span per record, attribute values equal the
stored record), SC-002 (rates from telemetry == rates from records),
SC-003 (endpoint unset → zero telemetry requests), SC-004 (unreachable
collector → behavior identical, bounded shutdown), SC-005 (attribute audit
finds no content/credentials), SC-006 (GenAI names present). Results
recorded below when run.

### Results (2026-06-12)

- **S1 spike (`spike_otlp`): PASS on the first run** — env-gated init,
  retroactive spans, GenAI constants under `semconv_experimental`,
  `application/x-protobuf` on the wire, bounded shutdown, zero requests
  with the endpoint env absent. One stack correction recorded in
  research.md: the otlp crate's `reqwest-rustls-webpki-roots` feature is
  incompatible with reqwest 0.13 (TLS rides the workspace-unified
  `rustls` feature instead).
- **Acceptance (`acceptance_otlp`): PASS** — SC-003 disabled fast path
  **6 ns/call** (bound 1000); the zero-egress half holds by construction
  (disabled init returns no providers, so no exporter exists to send);
  SC-001 **10/10 spans matched their stored records** (7 invocation
  records across success/invalid-input + 3 checkpoint rows spanning
  flag/silence/fail-open); SC-002 telemetry-computed counts equal
  record-computed counts (successes, flags, fail-opens); SC-005 **139
  attribute values audited** — no claim text, no credentials, no
  checkpoint evidence; SC-006 GenAI names resolve in the decoded
  payloads.
- **SC-004 session-level**: the spawn-the-binary smoke test runs the
  server with telemetry enabled against an unreachable collector —
  protocol behavior identical, stdout protocol-only, and the process
  exits within the bounded flush window (no hang).

## Inspect locally

Any OTLP collector works; the zero-infra option used in tests is the
wiremock double. For a real view:
`docker run -p 4318:4318 otel/opentelemetry-collector` and point the
endpoint at it.
