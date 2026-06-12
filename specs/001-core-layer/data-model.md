# Data Model: Core Layer

**Date**: 2026-06-11 · **Source**: spec.md Key Entities + research.md decisions

## 1. CorrectiveMode (modes are data)

The registry entry for one corrective. Verify is the only instance in this
feature, but the type is the contract for every future mode.

| Field | Type | Notes |
|---|---|---|
| `id` | string | tool name as exposed over MCP (`"verify"`) |
| `description` | string | the MCP tool description — does the routing work (client selects by description) |
| `prompt_template` | string | instruction template; placeholders for claim/context only (blindness is structural: stance/history have no placeholder to flow through) |
| `output_schema` | JSON Schema | the *unsanitized* schemars-derived per-pass schema; sanitized form derived at startup, cached (grammar cache favors stable schemas) |
| `ensemble_k` | u8 | parallel passes; from config, default 3 |
| `thinking_budget` | option | reserved; unused until Spike 4 resolves compatibility |

**Invariant**: every `output_schema` is flat (one level of named fields; arrays
of scalars allowed) and closed. Enforced by a startup assertion over the
registry — a mode with an illegal schema fails boot, not the first call.

## 2. VerifyParams (tool input)

| Field | Type | Validation |
|---|---|---|
| `claim` | string | required; non-empty after trim (else `invalid_input` before any model call); length ≤ configured max (oversize → `invalid_input`, never silent trim) |
| `context` | string, optional | verbatim context the verifier may use; the *only* extra information a pass ever receives |

## 3. PassVerdict (per-pass model output — the constrained schema)

What each of the k model passes is grammar-constrained to produce.

| Field | Type | Grammar-enforced | Validator-enforced |
|---|---|---|---|
| `verdict` | enum `"supported"` \| `"refuted"` | ✅ enum | — |
| `findings` | array of string | ✅ shape | non-empty when `verdict == "refuted"` (a refutation must name its concrete error — calibrated profile) |

Flat, closed, no numeric fields — deliberately nothing for the sanitizer to
strip except boilerplate, minimizing grammar-subset risk on the first mode.

## 4. Verdict (tool output — aggregated)

| Field | Type | Constraint |
|---|---|---|
| `verdict` | enum `"supported"` \| `"refuted"` | majority across k passes; tie → `"refuted"` (fail toward scrutiny) with the tie noted in findings |
| `findings` | array of string | deduplicated findings from the majority-side passes |
| `confidence` | number | **agreement ratio** (majority count / k), range [0,1] — computed by the server, validated locally; never model self-report |
| `passes` | integer | k actually completed (transparency when a pass errors but quorum held) |

Aggregation rule: if any pass fails with a non-quorum-breaking error, aggregate
over the completed passes and report the reduced `passes`; if fewer than ⌈k/2⌉
complete, the invocation fails with the dominant failure class — a verdict is
never synthesized from a minority.

## 5. InvocationRecord (observability row — one per invocation)

Table `invocation_records` (SQLite, created by idempotent startup migration):

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | UUID v4 |
| `session_id` | TEXT | MCP session correlation id |
| `tool` | TEXT | mode id |
| `model` | TEXT | model id used |
| `input_tokens` | INTEGER | summed across passes |
| `output_tokens` | INTEGER | summed across passes |
| `cost_usd` | REAL | tokens × configured per-model pricing (invoice-exactness not required) |
| `latency_ms` | INTEGER | wall-clock via `TimeProvider` |
| `outcome` | TEXT | one of the Outcome taxonomy values (below) |
| `created_at` | TEXT | RFC 3339, via `TimeProvider` |

**Invariant** (FR-010 / SC-007): exactly one row per invocation, written on
every exit path — success and each failure class. Enforced by writing the record
in a single exit point that all paths funnel through.

## 6. Outcome taxonomy (one enum, two uses)

`success` · `refusal` · `truncation` · `timeout` · `retries_exhausted` ·
`invalid_input` · `validation_failure` · `config_error` · `cancelled`

- Use 1: `AppError` variants surfaced to the MCP client — each renders a
  distinct, descriptive message naming the class (FR-007, SC-005).
- Use 2: the `outcome` column on `InvocationRecord` (FR-010).
- Mapping from the model client: `stop_reason end_turn` → continue;
  `refusal` → `refusal`; `max_tokens` → `truncation`; transport timeout →
  `timeout`; retries exhausted → `retries_exhausted`; local schema validation
  failure → `validation_failure`.

## 7. Config (extended)

| Var | Default | New? |
|---|---|---|
| `ANTHROPIC_API_KEY` | — (required) | existing |
| `ANTHROPIC_MODEL` | `claude-opus-4-8` | **new** |
| `VERIFY_ENSEMBLE_K` | `3` | **new** |
| `DATABASE_PATH` | `./data/parallax.db` | existing |
| `LOG_LEVEL` | `info` | existing |
| `REQUEST_TIMEOUT_MS` | `30000` | existing |
| `MAX_RETRIES` | `3` | existing |

Existing contract holds: required-and-missing → refuse to start naming the
item; present-but-unparseable → hard error, never a silent default (FR-009).

## Relationships

```text
Config ──────────────► AnthropicClient (impl ModelClient)
   │                          ▲ k parallel complete(prompt, sanitized(schema))
   ▼                          │
CorrectiveMode("verify") ── verify tool ──► PassVerdict ×k ──► Verdict
                              │                                   (aggregate)
                              ▼ every exit path
                        InvocationRecord ──► Storage (impl: sqlx SQLite)
                              ▲ timestamps/latency
                        TimeProvider
```
