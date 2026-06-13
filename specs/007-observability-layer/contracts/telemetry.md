# Telemetry Contract — what a backend receives

This is the operator-facing contract for 007 (the analog of the tool JSONs
in prior features): the complete exported surface. Anything not listed here
is not exported. Stability: `parallax.*` names are this project's contract
and change only with a recorded amendment; `gen_ai.*` names track the GenAI
semantic conventions (Development stability upstream).

## Enablement (the only switch)

| Condition | Effect |
|---|---|
| No `OTEL_EXPORTER_OTLP_ENDPOINT` (or `_TRACES_`/`_METRICS_` variant) in the environment | Telemetry fully disabled: no providers, no egress, no overhead beyond one atomic check |
| Endpoint set | Traces + metrics export over OTLP (`http/protobuf` default; the standard `OTEL_EXPORTER_OTLP_*` family is honored by the exporter) |
| `OTEL_SDK_DISABLED=true` | Disabled regardless of endpoint (honored app-side; the Rust SDK does not implement it — upstream #1936) |
| Malformed endpoint URL | Startup configuration error naming the variable (never a silent fallback) |

> **Inheritance note (spec clarification 2026-06-12):** these variables are
> shared across OTel-aware processes by design. A globally exported
> endpoint enables Parallax too. What that can ever export is bounded to
> the fields below — no input text, no memory/transcript content, no
> credentials.
>
> **0.32 gotcha:** schemeless endpoints default to `https://` — point local
> collectors at an explicit `http://localhost:4318`.

## Resource

`service.name` (default `mcp-parallax`; `OTEL_SERVICE_NAME` wins),
`service.version` (crate version), `service.instance.id` (per-process
session UUID), plus anything from `OTEL_RESOURCE_ATTRIBUTES`.

## Spans

### `parallax.{tool}` — one per tool invocation (kind CLIENT)

Timing = the invocation's measured window (start = end − latency). Status:
OK on success; ERROR with the outcome class otherwise.

| Attribute | Type | Values/source |
|---|---|---|
| `gen_ai.operation.name` | string | `execute_tool` |
| `gen_ai.request.model` | string | attributed model id |
| `gen_ai.usage.input_tokens` | int | summed input tokens |
| `gen_ai.usage.output_tokens` | int | summed output tokens |
| `gen_ai.provider.name` | string | `anthropic` (registry well-known value) \| `voyageai` (verified 2026-06-12: the conventions' registry has **no** Voyage entry, so this is a deliberate custom value in the registry's naming style) |
| `error.type` | string | outcome class (absent on success) |
| `parallax.tool` | string | `verify`, `unstick`, `check`, `save`, `recall`, `forget`, `research`, `checkpoint_action`, `checkpoint_batch`, `checkpoint_turn` |
| `parallax.outcome` | string | the outcome taxonomy (`success`, `refusal`, `truncation`, `timeout`, `retries_exhausted`, `invalid_input`, `validation_failure`, `search_provider`, `embedding_provider`, `cancelled`) |
| `parallax.cost_usd` | double | computed cost |
| `parallax.session_id` | string | per-process session UUID |

### `parallax.checkpoint.{boundary}` — one per checkpoint evaluation (kind INTERNAL)

Boundary ∈ `action` | `batch` | `turn`. Status: always OK (fail-open is
data).

| Attribute | Type | Values/source |
|---|---|---|
| `parallax.checkpoint.boundary` | string | `action` \| `batch` \| `turn` |
| `parallax.checkpoint.verdict` | string | `silence` \| `flag` \| `hold` |
| `parallax.checkpoint.signals` | string[] | fired signal kinds (`repetition`, `repeated_failure`, `memory_conflict`, `self_contradiction`) — kinds only, never evidence text |
| `parallax.checkpoint.suppressed` | bool | cooldown-suppressed delivery |
| `parallax.checkpoint.fail_open` | bool | evaluation degraded |
| `parallax.checkpoint.review_ran` | bool | the review hop ran (turn boundary) |
| `parallax.checkpoint.cost_usd` | double | review-hop cost (0 for pure paths) |
| `parallax.session_id` | string | the **harness session id** from the hook params (note: invocation spans carry the server's per-process session UUID; checkpoint spans carry the harness's — the useful correlation key at each level) |

## Metrics

| Name | Instrument | Unit | Attributes |
|---|---|---|---|
| `parallax.invocations` | counter (u64) | `{invocation}` | `parallax.tool`, `gen_ai.request.model`, `parallax.outcome` |
| `parallax.invocation.duration` | histogram (f64) | `s` | `parallax.tool`, `parallax.outcome` |
| `parallax.cost` | counter (f64) | `USD` | `parallax.tool`, `gen_ai.request.model` |
| `gen_ai.client.token.usage` | histogram (u64) | `{token}` | `gen_ai.token.type` (`input`\|`output`), `gen_ai.request.model`, `gen_ai.provider.name`, `parallax.tool` |
| `parallax.checkpoint.evaluations` | counter (u64) | `{evaluation}` | `parallax.checkpoint.boundary`, `.verdict`, `.suppressed`, `.fail_open` |
| `parallax.checkpoint.duration` | histogram (f64) | `s` | `parallax.checkpoint.boundary`, `.verdict` |

Derivable without the local DB (FR-002/SC-002): spend per tool, error rate
per outcome class, latency distributions, token totals per model, and
per-boundary checkpoint flag/hold/suppression/fail-open rates.

## Out of scope (FR-011 named deferrals)

Log export; intra-invocation child spans (verify passes, research phases);
trace-context propagation from the calling harness; dashboards.
