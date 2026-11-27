# Data Model: Jev turn signals (068)

One migration adds `cooldown_suppressed` to `jev_records`; `turn_signal_records` and its migration are removed (plan D3, D5).

## `jev_records` rows at site `turn_signals` (plan D7)

One row per request, so two per covered turn end: one for `pushback_reversal`, one for `unverified_fact` (plan D1).

| Field | Value on turn-signal rows |
|---|---|
| `site` | `turn_signals` (new in the contract's site list) |
| `mode` | `shadow` |
| `questions_json` | one entry: the question's `id`, `version`, `option_order`, `answer`, `probabilities`, `band` (null: no bands in 068, plan D11), plus `recommends` (the corrective 066's rule names, given the catalog at the time; plan D7) and, on `pushback_reversal` only, `number_diff` (true when a number or date in the final text differs from one in a transcript pair's earlier statement; plan D2a) |
| `eval_id` | the turn's checkpoint record |
| `state_json` | null; `state_hash` plus `transcript_ref` (§5.1) |
| `truncated` | true when the final text was cut to 2000 characters (plan D1) |
| `cooldown_suppressed` | new nullable column: JSON array of the signal kinds whose yes answer would have been suppressed by the cooldown (session plus kind, `COOLDOWN_WINDOW_MS`); null on other sites (plan D3) |
| `label_json`, `label_source`, `label_cost_usd` | null: no label call in 068 (plan D7) |
| `outcome`, `error_class` | as every Jev site; a redaction failure is `invalid_input` with `redaction:too_large` or `redaction:pattern` |
| `cost_usd` | the Jev request's cost; also added to the turn's checkpoint `jev_cost_usd` when the row is written (plan D7a) |

## Removed

`turn_signal_records`, its migration, `src/storage/turn_signals.rs` and the report code that reads it (plan D5).
