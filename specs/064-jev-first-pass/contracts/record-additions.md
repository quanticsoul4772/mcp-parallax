# Contract additions: records and outcomes (064)

These changes are made to the published contracts during implementation. The binding tests below fail until they are made.

## Invocation record

`specs/001-core-layer/contracts/invocation-record.schema.json` (the record schema bound by `src/error.rs:527-565`) gains:

| Field | Type | Meaning |
|---|---|---|
| `jev_cost_usd` | number \| null, ≥ 0 | Sum of the Jev rows' `cost_usd` for this invocation; null when no Jev call was made. Calls abandoned or cut off at their deadline may still be billed but carry no cost here; their rows say `cost unknown` |

The schema has already drifted from `InvocationRecord`: it lacks `models`, `usage_by_model`, `cost_estimated`, `depth`, `effort`, `passes` and `search_cost_usd` (`src/telemetry.rs:374-436`). Fixing that drift is a separate decision. 064 adds only `jev_cost_usd`.

## Checkpoint record

The `checkpoint_records` table and `CheckpointRecord` (no published schema file exists) gain:

| Field | Type | Meaning |
|---|---|---|
| `jev_cost_usd` | number \| null, ≥ 0 | Cost of the turn-review Jev request written with this record; null when none |

## Outcome list

`classifier_provider` is added to:

- `Outcome::ALL` (`src/error.rs`);
- the outcome enum in `invocation-record.schema.json`;
- the outcome list in `specs/007-observability-layer/contracts/telemetry.md`.

Mapping (research D4):

| Case | Outcome |
|---|---|
| Validator reject, or model mismatch | `validation_failure` |
| 4xx other than 429, including 400 `max_tokens_exceeded` | `provider_rejected` |
| Redaction failed (`redaction:too_large` / `redaction:pattern`); nothing sent | `invalid_input` |
| No time left in the run (`no_time_left`); nothing sent | `timeout` |
| No request slot before the deadline (`no_slot`); nothing sent | `timeout` |
| Deadline mid-request (`deadline; cost unknown`) | `timeout` |
| Verify finished first; call abandoned (`verify_finished; cost unknown`) | `timeout` |
| Refused after an earlier 401 (`disabled_after_401`); nothing sent | `provider_rejected` |
| 429, 529, 5xx, connect error | `classifier_provider` |

A Jev error never becomes the outcome of the call it shadows.

## Binding tests

- **`jev_records` ↔ `contracts/jev-record.schema.json`:** a two-way test, comparing `PRAGMA table_info(jev_records)` column names with the schema properties. The `site`, `mode` and `band` enums are derived from their Rust `::ALL`.
- **Outcome list:** the existing 057 test (`src/error.rs:527-565`) is extended. `jev_records.outcome` accepts `Outcome::ALL` plus `excluded`.
- **Price row:** `every_default_model_has_a_price_row` (`src/telemetry.rs:818-831`) includes `DEFAULT_JEV_MODEL`.
