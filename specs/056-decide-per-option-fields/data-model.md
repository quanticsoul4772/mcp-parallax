# Data Model: decide per-option fields

## Pass format for N options (model-facing, one per N in 2..=10)

| Field | Type | Required | Notes |
|---|---|---|---|
| `methodology` | enum `weigh` / `causal` / `probabilistic` | yes | unchanged |
| `deciding_factors` | array of strings | yes | unchanged; non-empty checked after parsing |
| `option_<NN>_score` for each option (NN: its number, two digits) | integer | yes | 0–100 checked after parsing |
| `option_<NN>_rationale` for each option | string | yes | one sentence |

Flat and closed (`additionalProperties: false`); no arrays of objects; checked by
`assert_flat` at startup for every N.

## Option assessment (server-assembled, unchanged)

`{ option, score, rationale }` for option position i, from `options[i-1]`,
`option_<NN>_score`, `option_<NN>_rationale`, in input order.

## Decide result (caller-facing, unchanged)

`recommended`, `runner_up`, `runner_up_reason`, `confidence`, `methodology`,
`deciding_factors`, `assessments` — same fields, same meaning, identical values
for identical scores.

## Input rule change

`options` (or `options_text`): 2 to 10 entries. Fewer than 2 or more than 10 is
`InvalidInput` before any model call, naming the count and the limit.

## Removed

The `option_scores` and `option_rationales` arrays, and with them the
"assessment arity mismatch" failure.
