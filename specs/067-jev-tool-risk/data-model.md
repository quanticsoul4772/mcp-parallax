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
| `outcome` | one of the existing outcomes plus `excluded` (§5.1); `excluded` for calls excluded in code; `invalid_input` for a redaction failure |
| `error_class` | for excluded rows, `risk_pattern` or `allowlist`; for a redaction failure, `redaction:too_large` or `redaction:pattern` (the existing `redaction_failed` helper) |
| `label_json` | for rows Jev answers, a code label: `{ "risk_pattern_match": bool }`, whether a risk pattern would also have matched the call; `label_source` = `code` (owner's answer, 2026-11-27; the roadmap's §7 gives U3 no label source). Null on excluded rows |
| `state_json` | null |
| `state_hash` | hash of the redacted projection, plus transcript coordinates (§5.1) |
| `redaction_count` | from the redactor |

## Constants

- Jev request deadline: `GATE_BUDGET_MS` (500 ms), no retries (§6.3.5).
- Whole-command allowlist: `cargo test` and `cargo build`, each followed by any arguments (§6.3.3; owner's answers), with the excluded operators in the spec's FR-003.
- Per-tool argument-value allowlist (§6.3.6): `file_path`, `path` and `pattern` of `Read`, `Grep` and `Glob` (owner's answer, 2026-11-27).
