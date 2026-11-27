# Data Model: Jev on tool calls (067)

No new table; one migration adds label_cost_usd to jev_records (found during implementation; decide-scored, 2026-11-27).

## `jev_records`, site `tool_risk`

| Column | Value |
|---|---|
| `site` | `tool_risk` |
| `mode` | `shadow` |
| `eval_id` | the checkpoint record's id |
| `harness_session_id` | the hook's session id |
| `questions_json` | `[{ id: "action_class", version, option_order, answer, probabilities, band }]`; empty for excluded rows |
| `outcome` | one of the existing outcomes plus `excluded` (§5.1); `excluded` for calls excluded in code; `invalid_input` for a redaction failure |
| `error_class` | for excluded rows, `risk_pattern` or `allowlist`; for a redaction failure, `redaction:too_large` or `redaction:pattern` (the existing `redaction_failed` helper) |
| `label_json` | on rows Jev answers, the big model's `action_class` answer for the same redacted projection, `{ "class": ... }`; `label_source` = model id plus prompt version (§5.1). Null on excluded rows (owner's answer, 2026-11-27; §7 gives U3 no label source) |
| `label_cost_usd` | the label call's cost (backend figure when reported, else priced at the label model's rate); null where no label call was made. The label model is the checkpoint review's routed model (found during implementation; decide-scored, 2026-11-27) |
| `state_json` | null |
| `state_hash` | hash of the redacted projection, plus transcript coordinates (§5.1) |
| `redaction_count` | from the redactor |

## Constants

- Jev request deadline: `GATE_BUDGET_MS` (500 ms), no retries (§6.3.5).
- Whole-command allowlist: `cargo test` and `cargo build`, each followed by any arguments (§6.3.3; owner's answers), with the excluded operators in the spec's FR-003.
- Per-tool argument-value allowlist (§6.3.6): `file_path`, `path` and `pattern` of `Read`, `Grep` and `Glob` (owner's answer, 2026-11-27).
