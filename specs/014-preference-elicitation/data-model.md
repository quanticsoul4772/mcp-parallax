# Data Model: Preference Elicitation

In-memory only; no persistence change (the recall reads memories, writes nothing). New
entities for the `elicit` mode.

## ElicitParams (tool input)

| Field | Type | Notes |
|---|---|---|
| task | String | what the caller is about to do — required, non-empty |
| context | Option`<String>` | optional neutral context — the only extra caller-prose input |

- Validated before any model call: `task` non-empty/non-oversize (FR-008). Recalled
  preferences are **server-fetched**, never caller-supplied (stance-blind).

## SignalLevel (enum, model + output)

`Low | Medium | High` — a scalar enum (flat-legal, grammar-enforced; the 011 H1 caveat is
only about `Option<enum>`). The model's self-report of how much preference signal it had.

## ElicitPass (per-pass constrained output) — flat + closed

A string, a scalar enum, and five arrays of scalars (per-item data as parallel arrays,
index-aligned; arrays of objects are illegal under the flat-schema gate).

| Field | Type | Notes |
|---|---|---|
| assumed_objective | string | the objective a surface reading would commit to |
| preference_texts | string[] | the governing preferences/constraints |
| preference_signals | string[] | where each preference was inferred from (request / context / stored memory) |
| preference_strengths | string[] | `"revealed"` or `"stated"` — **server-validated**; stored/revealed outrank stated |
| divergence_questions | string[] | the assumptions worth resolving (the divergence points) |
| divergence_signals | string[] | the conflicting signal each divergence cites |
| signal_level | enum `low\|medium\|high` | the model's self-report of available signal |

- **Well-formedness** (else a loud failed pass, 013 convention): the three `preference_*`
  arrays are equal length; the two `divergence_*` arrays are equal length; every
  `preference_strengths` value is `"revealed"` or `"stated"`. **Empty arrays are valid**
  (low signal — FR-005; the server does not fabricate).

## GoverningPreference / DivergencePoint (server-assembled, output elements)

The server zips the parallel arrays:

| GoverningPreference | Type | | DivergencePoint | Type |
|---|---|---|---|---|
| preference | String | | question | String |
| signal | String | | signal | String |
| strength | String (`revealed`\|`stated`) | | | |

## ElicitResult (tool output, server-assembled)

| Field | Type | Notes |
|---|---|---|
| assumed_objective | String | the surfaced objective |
| governing_preferences | Vec`<GoverningPreference>` | zipped; may be empty (low signal) |
| divergence_points | Vec`<DivergencePoint>` | zipped; empty when signals are consistent |
| signal_level | SignalLevel | the surfaced signal level |
| memory_consulted | bool | true when stored preferences were recalled (memory present) |

- Server-assembled; nested is fine (output not grammar-constrained). **No** `verdict`, no
  chosen option, **no action/hold/modify field** — surfacing only (FR-006/SC-005).

## Recall integration (server, when memory present) — research D2

When `memory: Option<&MemoryDeps>` is `Some`:

1. `recall(deps, &RecallParams { query: params.task, kind: None, limit: RECALL_LIMIT })`
   (a small constant, default 5).
2. Keep the **trusted** recalled memories (the verified/revealed signal).
3. Format them into the `<<preferences>>` prompt slot as *"stored verified preferences
   (revealed signal — outrank merely stated ones)"*. The model weights them above stated
   ones and raises a divergence point on any stated-vs-revealed conflict.

When `memory` is `None`: the `<<preferences>>` slot reads *"(no stored preferences — memory
not configured)"*; `memory_consulted = false`.

## Run (server) — single pass (research D1/D4)

`elicit` does **not** use `verify::aggregate_core`. One pass: `check_input → (optional
recall) → build prompt → complete → validate (schema) → typed ElicitPass → validate
well-formedness → zip → ElicitResult`. A failed single pass (refusal/timeout/validation)
propagates; no quorum.

## Configuration

No new variables. The input bound reuses `INPUT_MAX_CHARS`; `RECALL_LIMIT` is a code
constant. No new gate — always in the catalog (FR-009); the recall uses the existing
`VOYAGE_API_KEY`-gated memory capability when present.
