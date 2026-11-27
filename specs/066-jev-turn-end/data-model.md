# Data Model: Jev at turn end

## TurnData (`src/checkpoint/turn_view.rs`, in memory only)

| Field | Type | Meaning |
|---|---|---|
| `start` | `Option<usize>` | line id of the turn's real user prompt in the tail; `None` when not found |
| `user_prompt` | `String` | raw; never sent or stored |
| `tool_inputs` | `Vec<String>` | raw tool inputs of the turn |
| `tool_outputs` | `Vec<String>` | full tool-result text of the turn |
| `tool_calls_after_prompt` | `u32` | for the pushback structure check |

## Previous final messages (`CheckpointDeps`, in memory)

- Per harness session: the last `final_message` received at a turn end, continuation turns included. Not persisted.

## turn_signal_records (new table)

| Column | Type | Meaning |
|---|---|---|
| `id` | text | row id |
| `created_at` | text | timestamp |
| `eval_id` | text | joins the checkpoint record |
| `harness_session_id` | text | |
| `transcript_ref` | text | transcript path |
| `turn_start_uuid` | text, nullable | `uuid` of the turn start's transcript line |
| `final_uuid` | text, nullable | `uuid` of the final message's transcript line |
| `pushback_result` | text | `fired`, `not_fired` or an exclusion reason |
| `unverified_result` | text | `fired`, `not_fired` or an exclusion reason |
| `unmatched_entities` | text (JSON) | `[{ "kind": "path" \| "version" \| "identifier" \| "number", "hash": "..." }]` |
| `recommendations` | text (JSON) | `{ "pushback_reversal": "verify", "unverified_fact": "grounded_verify" \| "research" \| null }` |

Exclusion reasons: `turn start not found`, `no previous final message`, `transcript unreadable`, `tool call after prompt`.

## Checkpoint record

| Field | Type | Values |
|---|---|---|
| `review_skipped_by` | text, nullable | null, `jev`, `audit` |

## Pinned tables (`src/jev/policy/skip.rs`)

- `TURN_REVIEW_THRESHOLDS`: entries `{ jev_model, big_model, question_version, contradiction_no, capture_worthy_no }`. Empty.
- `TURN_REVIEW_ORDER_CHECKED`: entries `{ jev_model, big_model, question_version }`. Empty.

Both turn-review questions share one version, so one version keys an entry.

## Configuration (`src/config.rs`)

| Variable | Values | Default | Rules |
|---|---|---|---|
| `CHECKPOINT_TURN_SIGNALS` | `off`, `on` | `off` | unparseable value is a startup error |
| `PARALLAX_JEV_MODE_TURN_REVIEW` | `off`, `shadow`, `on` | `off` | `on` refused in 066 |
| `PARALLAX_JEV_*` names outside the known set | — | — | startup error |
