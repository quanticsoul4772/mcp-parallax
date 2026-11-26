# Data Model: Jev at turn end

## TurnData (`src/checkpoint/turn_view.rs`, in memory only)

| Field | Type | Meaning |
|---|---|---|
| `start` | `TurnStart` | `Found` or `NotFound` |
| `user_prompt` | `String` | the turn's real user prompt, raw; never sent or stored |
| `tool_inputs` | `Vec<String>` | normalized inputs of the turn's tool calls, raw |
| `tool_outputs` | `Vec<String>` | full tool-result text, raw |
| `tool_calls_after_prompt` | `u32` | count, for the pushback structure check |

## Previous final messages (server, in memory)

- `HashMap<harness_session_id, String>`: the last `final_message` received at a turn end. Not persisted.

## Jev site (`src/traits/classifier.rs`)

- `JevSite::TurnSignals`: suffix `TURN_SIGNALS`, stored as `turn_signals`; added to `JevSite::ALL`.

## Turn signal row (`jev_records`)

- `site = turn_signals`, `mode = shadow`.
- `questions_json`: the questions asked (`pushback_reversal`, `unverified_fact`), version, answers.
- `label_json`: `{ "pair_found": bool, "entities_matched": n, "entities_unmatched": n, "recommendations": { "pushback_reversal": "verify", "unverified_fact": "grounded_verify" | "research" | null } }`.
- `state_hash` of the redacted request state; `state_json` null.
- `outcome`: a call outcome, or `excluded` with `error_class` naming the reason (`turn start not found`, `no previous final message`, `nothing to ask`).
- `truncated`, `eval_id`, `harness_session_id`, cost fields as in 064.

## Checkpoint record

| Field | Type | Values |
|---|---|---|
| `review_skipped_by` | text, nullable | null, `jev`, `audit` |

- `jev_cost_usd` sums every Jev row of the evaluation.

## Pinned tables (`src/jev/policy.rs`)

- `TURN_REVIEW_NO_BAND`: per model, the no-band threshold for `contradiction` and `capture_worthy`. Empty.
- `TURN_REVIEW_ORDER_CHECKED`: models with a recorded order check. Empty.

## Configuration (`src/config.rs`)

| Variable | Values | Default | Rules |
|---|---|---|---|
| `PARALLAX_JEV_MODE_TURN_SIGNALS` | `off`, `shadow` | `off` | non-off needs `TYPESAFE_API_KEY` |
| `PARALLAX_JEV_MODE_TURN_REVIEW` | `off`, `shadow`, `on` | `off` | `on` needs the configured model in both pinned tables |
| any other `PARALLAX_JEV_*` | — | — | startup error |
