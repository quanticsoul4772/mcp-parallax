# Data Model: Decide — Methodology-Driven Choice

In-memory only; no persistence change. New entities for the `decide` mode.

## DecideParams (tool input)

| Field | Type | Notes |
|---|---|---|
| decision | String | the question to settle, stated neutrally — required, non-empty |
| options | Vec`<String>` | the candidate options — **≥ 2 required** (FR-008); order is the index basis for the score arrays |
| context | Option`<String>` | optional neutral context/criteria — the only extra subject input |

- Validated before any model call: `decision` non-empty/non-oversize; `options.len() >= 2`
  (a single/zero-option call is `invalid_input`, no fabricated comparison).
- The tool input is **not** grammar-constrained (it is `Deserialize`), so `Vec<String>` is
  fine here — the flat-schema rule governs only the model's *output*.

## Methodology (enum, model + output)

`Weigh | Causal | Probabilistic` — a scalar enum (flat-legal, grammar-enforced; the 011
H1 caveat applies only to `Option<enum>`, not this required non-null field).

## DecidePass (per-pass constrained output) — flat + closed

The single pass's structured output. Per-option data is **parallel scalar arrays** (an
array of objects is illegal under `assert_flat`), index-aligned to `DecideParams.options`.

| Field | Type | Notes |
|---|---|---|
| methodology | enum `weigh\|causal\|probabilistic` | the frame the model applied |
| option_scores | integer[] | 0–100, one per input option, in option order |
| option_rationales | string[] | one per input option — why it scored that |
| deciding_factors | string[] | the factors/criteria the methodology used |

- **Well-formedness validation** (a failed pass otherwise, FR-004): `option_scores.len()
  == option_rationales.len() == options.len()`, `deciding_factors` non-empty, and **every
  score within 0–100**. A score outside the range is a **failed pass** (loud), not clamped
  — a malformed assessment is treated like the arity mismatch, matching the project's
  loud-over-silent convention.

## OptionAssessment (server-internal / output element)

The server **zips** the parallel arrays with the option labels:

| Field | Type | Notes |
|---|---|---|
| option | String | from `DecideParams.options[i]` |
| score | i64 | from `option_scores[i]` (validated 0–100; out-of-range → failed pass) |
| rationale | String | from `option_rationales[i]` |

## Rank + calibrate (server, pure, deterministic) — research D2/D3

1. Build `OptionAssessment` per option (zip).
2. Sort by `score` descending; **ties resolve by input order** (stable sort keeps the
   earlier option ahead). The top is `recommended`, the next is `runner_up`.
3. `margin = recommended.score − runner_up.score`.
4. `confidence = 0.5 + 0.5 * min(margin, SCALE) / SCALE`, `SCALE = 100`, clamped
   `[0.5, 1.0]`. Tie (margin 0) → `0.5`; 100-point lead → `1.0`. **Semantics**: confidence
   is the certainty the recommended option beats the **runner-up**, not its lead over the
   whole field — a close top-two (both far above a third option) correctly reads ~0.5.
5. `runner_up_reason` is composed server-side: `"scored {margin} below {recommended}:
   {runner_up.rationale}"`.

## DecideResult (tool output, server-assembled)

| Field | Type | Notes |
|---|---|---|
| recommended | String | the top-scored option label |
| runner_up | String | the second option label |
| runner_up_reason | String | server-composed (margin + runner-up rationale) |
| confidence | f64 | margin-derived, `[0.5, 1.0]` |
| methodology | String | the surfaced frame (lowercased enum) |
| deciding_factors | string[] | the factors the methodology used |
| assessments | Vec`<OptionAssessment>` | full per-option breakdown (option, score, rationale), for audit |

- Server-assembled; nested (`assessments`) is fine — the output is not grammar-constrained
  (precedent: `grounded_verify`'s manifest). No `verdict`, no `next_step`.

## Aggregation (server) — single pass, no quorum (research D4)

`decide` does **not** use `verify::aggregate_core`. One pass: `complete → validate
(schema) → validate arity → zip → rank → calibrate → DecideResult`. A failed single pass
(refusal/timeout/validation) propagates directly — there is no quorum to fall back on.

## Configuration

No new variables. The input bound reuses `INPUT_MAX_CHARS`. `SCALE` and the
margin→confidence form are code constants. No new gate (FR-009).
