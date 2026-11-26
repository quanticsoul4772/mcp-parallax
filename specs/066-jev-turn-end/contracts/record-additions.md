# Contract: record additions (066)

## jev_records

- `site` gains `turn_signals` (`specs/064-jev-first-pass/contracts/jev-record.schema.json`).
- `turn_signals` rows: `state_json` is always null; `label_json` holds `pair_found`, `entities_matched`, `entities_unmatched` and `recommendations`; an `excluded` row names its reason in `error_class`.

## checkpoint_records

- New column `review_skipped_by`: null, `jev` or `audit`. Migration adds it to existing databases.
- `jev_cost_usd` sums every Jev row with the evaluation's `eval_id`.

## Telemetry (`specs/007-observability-layer/contracts/telemetry.md`)

- Line 97: `parallax.jev.requests` becomes a count (0-2) per turn boundary; `parallax.jev.cost_usd` equals the record's `jev_cost_usd` (the sum).
- New attribute `parallax.checkpoint.review_skipped_by` (enum: `jev`, `audit`), present only when set, derived at the same exit point as the record.
