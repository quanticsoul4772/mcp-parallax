# Data Model: Jev on tool calls (067)

No new table and no migration (§5.1 records).

## `jev_records`, site `tool_risk`

| Column | Value |
|---|---|
| `site` | `tool_risk` |
| `mode` | `shadow` |
| `eval_id` | the checkpoint record's id |
| `harness_session_id` | the hook's session id |
| `questions_json` | `[{ id: "action_class", version, option_order, answer, probabilities, band }]`; empty for excluded rows |
| `outcome` | the call outcome; `excluded` for calls excluded in code |
| `error_class` | for excluded rows, the reason: `risk_pattern` or `allowlist` |
| `label_json` | null in 067: the roadmap gives U3 no label source (§7); one is set when promotion is specified (owner's answer, 2026-11-27) |
| `state_json` | null |
| `state_hash` | hash of the redacted projection, plus transcript coordinates (§5.1) |
| `redaction_count` | from the redactor |

## Constants

- Jev request deadline: `GATE_BUDGET_MS` (500 ms), no retries (§6.3.5).
- Whole-command allowlist: whole commands only, such as test and build commands (§6.3.3), with the excluded operators in the spec's FR-003.
- Per-tool argument-value allowlist (§6.3.6).
