# Contract additions: telemetry (064)

Additions to `specs/007-observability-layer/contracts/telemetry.md`. They are derived at the same exit point as the records, so the two can't disagree (007 FR-009).

## Span attributes

Added to the existing **checkpoint** span (turn-review site) and **invocation** span (research). There are no child spans: `telemetry.md:136` keeps them out of scope.

All values are numeric or enumerated. No text, no answers and no state are exported.

| Attribute | Type | On | Meaning |
|---|---|---|---|
| `parallax.jev.mode` | enum `off`\|`shadow` | both | Site mode for this call |
| `parallax.jev.requests` | int | both | Jev requests made (1 or 0 per turn; per claim count for research) |
| `parallax.jev.excluded` | int | invocation | Claims excluded in code |
| `parallax.jev.failures` | int | both | Requests that ended in a non-ok outcome |
| `parallax.jev.cost_usd` | double | both | Same value as the record's `jev_cost_usd` |
| `parallax.jev.latency_ms` | int | checkpoint | The turn request's wall time |
| `parallax.jev.critical_path_ms` | int | both | Time the shadowed call waited on Jev beyond its own work (SC-002) |

## Metrics

Jev tokens are **not** added to `gen_ai.client.token.usage`; they are not model tokens of the shadowed call. The `parallax.jev.cost_usd` attribute and the existing cost metric's record value carry the money.

## Outcome list

`classifier_provider` is added to the outcome list (see `record-additions.md`).

## Housekeeping

The signal list at `telemetry.md:89` is already stale: it lacks `preference_violation`. It is corrected in the same edit.
