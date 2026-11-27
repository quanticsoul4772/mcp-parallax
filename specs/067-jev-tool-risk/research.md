# Research: Jev on tool calls (067)

Each item carries the roadmap line it implements (`warm-kindling-shell.md`) and the code facts it rests on (checked 2026-11-27). Nothing here goes beyond the roadmap and the owner's answers in the spec's Clarifications.

## D1. Hook matcher (§6.3.1)

- Today: `PreToolUse` matcher `Bash|Write|Edit` (`integrations/claude-code/hooks.json:6`, `.claude/settings.json:56`).
- The spike: a widened matcher that includes MCP tools and excludes `mcp__parallax__*`, verified with no self-trigger. A test parses `hooks.json` and checks the matcher.

## D2. Order (§6.3.3, Clarifications R)

1. Risk patterns: `gate::risk_matched` (`src/checkpoint/gate.rs:17`), unchanged. Excluded row, reason `risk_pattern`.
2. An allowlist of whole commands only, such as test and build commands (§6.3.3), never matching a command containing `&&`, `;`, `|`, `$(`, a backtick, `&`, `>`, `>>`, `<`, `<(` or `>(` (Clarifications R). Excluded row, reason `allowlist`.
3. Everything else: Jev.

## D3. Projections (§6.3.6, Clarifications M)

- `Bash`: the `command`, heredoc bodies removed, then redacted.
- `Write`, `Edit`: `file_path` only, redacted.
- MCP tools and the other built-in tools (Clarifications M): the tool name plus argument key names, with values only for fields on a per-tool allowlist (§6.3.6).
- Every projection goes through the mandatory redactor (`Redactor::redact_json`, `src/jev/redact.rs:182`); a redaction failure sends nothing and is recorded.
- Wire-level tests assert on the whole serialized body.

## D4. Timing (§6.3.5, Clarifications P)

- The gate's work and verdict are unchanged and do not wait for Jev.
- The Jev request is started after classification and is not awaited by the hook response; its deadline is `GATE_BUDGET_MS` (500 ms, `src/checkpoint/mod.rs:40`), with no retries. Its row is written when it ends.
- The memory hold (`gate.rs` constraint hold) is computed exactly as today for `Bash`, `Write` and `Edit`, whatever Jev does.
- Contract test: Jev deadline plus `GATE_BUDGET_MS` stays well under the 5 s hook timeout read from `hooks.json:18`.
- A request still running at process exit leaves no row (accepted in Clarifications P).

## D5. Records (§5.1 records, §3.11)

- One `jev_records` row per Jev request, site `tool_risk`, linked by `eval_id` to the checkpoint record.
- One `excluded` row per call excluded in code, with no request; the exclusion reason (`risk_pattern` or `allowlist`) goes in `error_class`.
- `state_json` is null; the row stores only `state_hash` plus transcript coordinates (§5.1).

## D6. Question (§6.3.4)

`action_class`, choice, version 1, options in the order `spend_or_destructive`, `state_changing`, `read_only`. The wording is adapted from harness-bridge's fixed criteria (§6.3.4), replacing only harness-bridge's own terms ("dispatch", "stages", "the envelope's grant"); the text is in `contracts/questions.md`.

## D7. Configuration (§5.1 config)

`JevSite::ToolRisk` joins `JevSite::ALL` (`src/traits/classifier.rs:25`). The existing rules give: unknown values are errors, `shadow` needs the key, and `on` is refused (`src/config.rs:175`).

## D8. What leaves the machine (§8, §6.3.7)

| Destination | What | When |
|---|---|---|
| TypeSafe (Jev) | the projections in D3 | shadow on, call reaches Jev |
| Voyage | `Bash`, `Write`, `Edit`: tool name plus whole input, including `Write` file contents (unchanged, now listed per §6.3.7) | memory on, risk-matched |
