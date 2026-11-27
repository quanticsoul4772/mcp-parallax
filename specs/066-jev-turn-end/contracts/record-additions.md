# Contract: record additions (066)

## turn_signal_records (new)

- Schema: `turn-signal-record.schema.json`, bound to the table two ways by a `PRAGMA table_info` test.
- One row per covered turn end; joins the checkpoint record by `eval_id`; holds no conversation text (unmatched entities are stored as a kind and a hash).

## checkpoint_records

- New column `review_skipped_by`: null, `jev` or `audit`. Migration adds it to existing databases.

## jev_records

- Unchanged, except turn-review rows carry question version 2 from 066 on.

## Telemetry (`specs/007-observability-layer/contracts/telemetry.md`)

- New attribute `parallax.checkpoint.review_skipped_by` (enum: `jev`, `audit`), present only when set, derived at the same exit point as the record. Jev request count and cost per turn are unchanged.
