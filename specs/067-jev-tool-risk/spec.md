# Feature Specification: Jev on tool calls

**Feature Branch**: `067-jev-tool-risk`

**Created**: 2026-11-26 (rewritten 2026-11-27 from the roadmap)

**Status**: Draft

**Input**: the Jev roadmap (`warm-kindling-shell.md`): §4 line 170 ("U3 risk classification, shadow only, after a hooks spike"), §6.3 items 1-9, §11's 067 test list, §5.5 acceptance, §4.1 line 186, with the shared rules in §3, §5.1 (config, client, records), §7 and §8. This spec carries that text as written. Where the roadmap contradicts itself or is silent, the owner's answers are recorded under Clarifications. Nothing else is added.

## Clarifications

### Session 2026-11-27

- **O.** Q: §6.3.1 widens the matcher, which would put newly matched tools through the memory hold; §6.3.8 says U3 changes no behavior in 067. Which governs? → A: §6.3.8. Tools newly reached by the widened matcher are classified only; the memory hold stays on risk-matched `Bash`, `Write` and `Edit` calls as today. No new permission prompts.
- **P.** Q: §6.3.5 runs Jev alongside the gate; §11 tests "gate latency unchanged with a never-returning classifier". Should a tool call wait for Jev? → A: No. The gate returns its verdict without waiting; Jev's answer is recorded when it arrives. Consequence accepted with the answer: a request still running when the process exits leaves no row (the roadmap's Appendix A rejected detached work for this reason).
- **M.** Q: The roadmap names no projection for Claude's built-in tools other than `Bash`, `Write` and `Edit`. What does Jev see for them? → A: The same as MCP tools: the tool name plus argument key names.
- **R.** Q: The allowlist's excluded operators (§6.3.3: `&&`, `;`, `|`, `$(`, backticks) miss `&`, `>`, `>>`, `<`, `<(` and `>(`. → A: Those are excluded too; only a plain command with plain arguments matches the allowlist. Newline and carriage return are excluded too; each starts another command in bash (found during implementation; decide-scored, 2026-11-27).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The hook reaches every tool except Parallax's own (Priority: P1)

A widened `PreToolUse` matcher includes MCP tools and excludes `mcp__parallax__*`, verified with no self-trigger by a spike before anything depends on it (§6.3.1).

**Independent Test**: a test parses `hooks.json` and checks the matcher; the spike shows the hook firing for a non-Parallax tool and not for a Parallax tool.

**Acceptance Scenarios**:

1. **Given** the widened matcher, **When** a non-Parallax tool is called, **Then** `checkpoint_action` runs for it.
2. **Given** the widened matcher, **When** an `mcp__parallax__*` tool is called, **Then** `checkpoint_action` does not run.
3. **Given** the hook's own call to `checkpoint_action`, **When** it runs, **Then** it does not trigger the hook again.

### User Story 2 - Every matched call is classified, recorded only (Priority: P1)

`run_action` is restructured so every matched call is classified, including with memory off (§6.3.2). Risk patterns come first, then an allowlist of whole commands, and Jev sees only the remainder (§6.3.3), with one choice question, `action_class` (§6.3.4). Shadow only: nothing the user sees changes (§6.3.8).

**Independent Test**: with `PARALLAX_JEV_MODE_TOOL_RISK=shadow`, fixture calls leave the expected records, risk-pattern and allowlisted calls make no Jev request, and every gate verdict and message equals the run with the site off.

**Acceptance Scenarios**:

1. **Given** shadow mode, **When** a call matches a risk pattern, **Then** no Jev request is made and an excluded record is written.
2. **Given** shadow mode, **When** a `Bash` command is a plain command on the allowlist, **Then** no Jev request is made and an excluded record is written.
3. **Given** shadow mode, **When** a command chains or redirects (any excluded operator), **Then** the allowlist does not apply.
4. **Given** shadow mode, **When** any other call is evaluated, **Then** Jev is asked `action_class` and the answer is recorded.
5. **Given** memory off, **When** a call is evaluated, **Then** it is still classified.
6. **Given** a stored memory that holds a risk-matched `Bash`, `Write` or `Edit` action, **When** Jev hangs, errors, or answers `read_only` at 1.0, **Then** the hold happens unchanged (§6.3.5).
7. **Given** a tool newly reached by the widened matcher, **When** it is evaluated, **Then** no hold is evaluated for it.

### User Story 3 - Only projections leave the machine (Priority: P1)

Jev receives projections only (§6.3.6): the `Bash` command, redacted, with heredoc bodies dropped; `file_path` only for `Write` and `Edit`; for MCP tools and the other built-in tools, the tool name plus argument key names, with values only for fields on a per-tool allowlist. If redaction fails, nothing is sent.

**Independent Test**: wire-level tests assert on the whole serialized request body.

### User Story 4 - On in the owner's setup (Priority: P2)

After merge and rebuild, `PARALLAX_JEV_MODE_TOOL_RISK=shadow` and the widened hook matcher are set in the owner's setup (§4.1 line 186), with a backup taken and the exact keys shown first; then the user restarts the session (§4.1).

### Edge Cases

- Jev is slow or down: the gate's verdict does not wait; the request ends at its deadline with no retry, and the outcome is recorded.
- `shadow` without `TYPESAFE_API_KEY`: a startup error (§5.1 config).
- Redaction fails on a projection: nothing is sent; the failure is recorded.
- The process exits while a Jev request is running: no row for that call (Clarifications).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001** (§6.3.1): `integrations/claude-code/hooks.json`, its README and `.claude/settings.json` use a `PreToolUse` matcher that includes MCP tools and excludes `mcp__parallax__*`, verified by the spike with no self-trigger. A test parses `hooks.json` and checks the matcher.
- **FR-002** (§6.3.2, Clarifications): `run_action` classifies every matched call, including with memory off. The memory hold applies only to risk-matched `Bash`, `Write` and `Edit` calls, as today.
- **FR-003** (§6.3.3, Clarifications R): Risk patterns first; then an allowlist of whole commands only, such as test and build commands, which never matches a command containing `&&`, `;`, `|`, `$(`, a backtick, `&`, `>`, `>>`, `<`, `<(`, `>(`, a newline or a carriage return; Jev sees only the remainder.
- **FR-004** (§6.3.4): One choice question, `action_class`, options ordered `spend_or_destructive`, `state_changing`, `read_only`, wording adapted from harness-bridge's fixed criteria, versioned, model pinned (§5.1).
- **FR-005** (§6.3.5, Clarifications): The Jev request runs beside the gate without the gate waiting for it, with a deadline sized to `GATE_BUDGET_MS` and no retries. The memory hold stands whatever Jev does. A contract test asserts the Jev budget plus `GATE_BUDGET_MS` stays well under the 5 s hook timeout (`hooks.json:18`): at most a quarter of it, 1.25 s (owner's answer, 2026-11-27).
- **FR-006** (§6.3.6, Clarifications): Requests carry only the projections in User Story 3, after the mandatory redactor (§8). If redaction fails, nothing is sent.
- **FR-007** (§6.3.7): The §8 "what leaves the machine" table lists that Voyage already receives `Write` file contents through the existing gate embed.
- **FR-008** (§6.3.8): U3 changes no behavior in 067, because it is shadow only. Before any later promotion, a decision is needed on whether a result becomes a non-blocking note or a flag, and what it recommends. The gate contract currently says it never flags (`src/checkpoint/contract.rs:5-6`). The note-or-flag decision belongs to promotion, not to 067, so no 067 task carries it.
- **FR-009** (§3.11, §5.1 records, §5.3): Every Jev request leaves one `jev_records` row at site `tool_risk`; every call excluded in code leaves an `excluded` row with no request (§5.3: "leaves an excluded row with no call"); a redaction failure is recorded (§3.11: "every attempted call and every exclusion leaves a record or a count") the way the existing Jev sites record it: outcome `invalid_input`, `error_class` `redaction:too_large` or `redaction:pattern` (the shared `redaction_failed` helper in `src/jev/shadow.rs`).
- **FR-010** (§5.1 config): `PARALLAX_JEV_MODE_TOOL_RISK` is `off` | `shadow`, default `off`; `on` is a startup error; `shadow` without the key is a startup error.
- **FR-011** (§6.3.9): Amend `WATCHDOG_LAYER.md:56` and `:84-86` (the roadmap's `:80-82`; the 064 and 066 amendments added 2 lines each), the 006 D4/FR-013 text, and the "No model pass" comment at `src/checkpoint/gate.rs:5-7`.
- **FR-012** (§4.1 line 186): The last task sets `PARALLAX_JEV_MODE_TOOL_RISK=shadow` and the widened hook matcher in the owner's setup.

### Tests required (§11's 067 list)

1. `hooks.json` matcher binding.
2. Pre-filter table, including chained-command rejection, with `times(0)` at the classifier.
3. Wire-level projection and redaction.
4. Hold independent of Jev: hang, error, or `read_only` at 1.0.
5. Gate latency unchanged with a never-returning classifier.
6. Budget-under-hook-timeout contract.
7. Memory questions off without the opt-in.
8. Live no-self-trigger assertion.

### Key Entities

- **`jev_records` row, site `tool_risk`** (§5.1 records): one per Jev request or code exclusion.
- **Projection** (§6.3.6): what Jev may see of a call.
- **Allowlists**: the whole-command allowlist (§6.3.3); the per-tool argument-value allowlist (§6.3.6).

## Success Criteria *(mandatory)*

### Measurable Outcomes (§5.5, applied to 067)

- **SC-001**: The spike shows the hook firing for every non-Parallax tool tried and for no Parallax tool, with no self-trigger.
- **SC-002**: The live example asserts and exits non-zero on failure: at least 25 rows at site `tool_risk` (owner's answer for §5.5's N, 2026-11-27); every outcome in the bound set (§5.1: the existing outcomes plus `excluded`); every row joins to its checkpoint record; `model_answered` equals the pinned version on every row that made a Jev request (excluded rows make none); no row has `applied=true`; every redaction fixture is absent from captured request bodies; no Jev request row exists for a call excluded in code (§5.5's "no row exists for an excluded claim", read with §5.3's `excluded` rows; owner's answer, 2026-11-27). For §5.5's "at least X% of rows carry `label_json`": every row Jev answers carries the big model's label for the same redacted projection, requested off the critical path like Jev's (owner's answer, 2026-11-27; §7 gives U3 no label source).
- **SC-003**: With a classifier that never returns, the gate's latency is unchanged: its response path never awaits the Jev request, shown by a never-returning classifier still getting a response in the same test run (owner's answer, 2026-11-27).
- **SC-004**: The live example asserts that Jev adds zero time to the critical path (§5.5's "Jev time on the critical path below a stated limit", which is zero under Clarifications P; owner's answer, 2026-11-27).
- **SC-005**: Load: acceptance runs with at least two sessions plus a research run at once, and measures hook p99 latency.
- **SC-006**: The full gate exactly as CLAUDE.md states it.
