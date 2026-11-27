# Feature Specification: Jev on tool calls

**Feature Branch**: `067-jev-tool-risk`

**Created**: 2026-11-26

**Status**: Draft

**Input**: the Jev roadmap §6.3 (U3 risk classification, shadow only, after a hooks spike) and §4.1 (turn the site on in the owner's setup as the last task). The feature description given to `/speckit-specify` is reproduced under Source below.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The gate sees every tool call, without seeing itself (Priority: P1)

Today the pre-action hook fires only for `Bash`, `Write` and `Edit`. A widened matcher makes it fire for every tool call, MCP tools included, except Parallax's own tools, so the gate never evaluates its own checkpoint calls. A spike verifies this against the live harness before anything depends on it.

**Why this priority**: every later story needs the hook to fire on the calls it classifies, and a gate that triggers itself would loop.

**Independent Test**: with the widened matcher installed, calling a non-Parallax MCP tool produces one pre-action evaluation, and calling a Parallax tool produces none.

**Acceptance Scenarios**:

1. **Given** the widened matcher, **When** the model calls `Bash`, `Write`, `Edit` or any non-Parallax MCP tool, **Then** the pre-action hook fires once for that call.
2. **Given** the widened matcher, **When** the model calls any `mcp__parallax__*` tool, **Then** the pre-action hook does not fire.
3. **Given** the repository's hook config and the project settings, **When** the test suite runs, **Then** a test parses the shipped matcher and fails if it would match a Parallax tool or miss a tool the spike verified.

---

### User Story 2 - Every matched call gets a risk class, recorded only (Priority: P1)

Each tool call the gate evaluates is given one of three classes: `spend_or_destructive`, `state_changing` or `read_only`. Code decides first: calls matching the existing risk patterns, and whole commands on a fixed allowlist, are classed in code without asking Jev. Jev answers one choice question, `action_class`, only for the remainder, with the options in that fixed order. The class is recorded; the gate's verdict, its message and its hold behavior do not change.

**Why this priority**: the recorded classes are what a later feature needs to measure Jev against the risk patterns and to decide whether a class should ever change what the gate does.

**Independent Test**: with `PARALLAX_JEV_MODE_TOOL_RISK=shadow`, a fixture set of calls produces one record per evaluated call with the expected class source (code or Jev), and every checkpoint result and record is byte-identical to the same run with the mode off, apart from timing and the new records.

**Acceptance Scenarios**:

1. **Given** shadow mode, **When** a call matches a risk pattern, **Then** no Jev request is made and the record shows a code-assigned class.
2. **Given** shadow mode, **When** a `Bash` call is a single whole command on the allowlist, **Then** no Jev request is made and the record shows a code-assigned class.
3. **Given** shadow mode, **When** a `Bash` command chains commands (`&&`, `;`, `|`, `$(` or backticks), **Then** the allowlist does not apply to it.
4. **Given** shadow mode, **When** any other call is evaluated, **Then** one Jev request asks `action_class` and the answer is recorded.
5. **Given** shadow mode and memory off, **When** a call is evaluated, **Then** it is still classified and recorded.
6. **Given** a stored memory that holds the action, **When** Jev answers `read_only` with probability 1.0, errors, or never returns, **Then** the hold still happens, unchanged.

---

### User Story 3 - Only a projection of the call leaves the machine (Priority: P1)

Jev never receives a whole tool input. It receives a projection, after the mandatory redactor: for `Bash`, the command with heredoc bodies removed; for `Write` and `Edit`, the file path only; for an MCP tool, the tool name and the names of its arguments, with argument values only for fields on a per-tool allowlist. If redaction fails, nothing is sent.

**Why this priority**: tool inputs carry file contents, credentials and private text; the classification must not become a new path for them to leave.

**Independent Test**: captured request bodies for a fixture set (a heredoc with a secret, a `Write` with file contents, an MCP call with a token argument) contain none of the sentinel strings planted in the parts that must not be sent.

**Acceptance Scenarios**:

1. **Given** a `Write` call, **When** it is classified by Jev, **Then** the request holds the file path and none of the file contents.
2. **Given** a `Bash` command with a heredoc, **When** it is classified, **Then** the heredoc body is absent from the request.
3. **Given** an MCP call, **When** it is classified, **Then** only allowlisted argument values appear; every other argument appears by name only.
4. **Given** the redactor fails on a projection, **When** the call is evaluated, **Then** no request is made and the failure is recorded.

---

### User Story 4 - Turned on in the owner's setup (Priority: P2)

After merge and rebuild, the owner's Parallax entry gets `PARALLAX_JEV_MODE_TOOL_RISK=shadow` and the owner's hook config gets the widened matcher, so classes start being recorded on real work.

**Why this priority**: shadow records only build up while the site runs.

**Independent Test**: after the setup change and a restart, a tool call in a real session leaves a tool-risk record in the server's database.

**Acceptance Scenarios**:

1. **Given** the setup change and a restart, **When** the owner's session calls a matched tool, **Then** a tool-risk record is written.

### Edge Cases

- Jev is slow or down: the gate's verdict is returned within its existing budget; the Jev request is abandoned at its deadline and the outcome recorded; no retry.
- The key is missing: setting the mode to `shadow` without `TYPESAFE_API_KEY` is a startup error, as for the other Jev sites.
- A tool input too large for the projection rules (for example a very long command): the projection is truncated to a fixed limit, the truncation is recorded, and no decision depends on it.
- The hook fires for a tool the per-tool allowlist does not know: its arguments are sent by name only.
- A risk-pattern match and an allowlist match on the same command: the risk pattern wins.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The shipped hook config and the project settings MUST use a pre-action matcher that fires for every tool call except `mcp__parallax__*`, as verified by the spike (task 1). A test MUST parse the shipped matcher and check it against the verified tool list.
- **FR-002**: The gate MUST evaluate every call the hook sends it, with or without memory configured. Calls that match no risk pattern keep today's silence verdict; only the classification is new for them.
- **FR-003**: Classification MUST run in this order: risk patterns (code), then the whole-command allowlist (code, `Bash` only, never for chained commands), then Jev for the remainder.
- **FR-004**: The Jev question MUST be one choice question, `action_class`, with options `spend_or_destructive`, `state_changing`, `read_only` in that order, each with fixed criteria, versioned, and the model pinned to `jev-1.14.0`.
- **FR-005**: The Jev request MUST run beside the gate's own work, with a deadline that keeps the gate inside its existing budget (`GATE_BUDGET_MS`, 500 ms) and no retries. A contract test MUST keep the Jev deadline plus `GATE_BUDGET_MS` well under the 5 s pre-action hook timeout.
- **FR-006**: The memory constraint hold MUST be unaffected by any Jev answer, error, timeout or missing key.
- **FR-007**: Requests MUST carry only the projection in User Story 3, after the mandatory redactor; a redaction failure MUST send nothing and be recorded.
- **FR-008**: Every evaluated call in shadow mode MUST leave one record: the class, its source (risk pattern, allowlist or Jev), the Jev outcome when asked, and the projection's truncation flag. Code-classed calls MUST leave a record with no request.
- **FR-009**: The gate MUST NOT flag, and its results and checkpoint records MUST NOT change, because of the classification (the gate's contract, `src/checkpoint/contract.rs:5-6`).
- **FR-010**: The site MUST be controlled by `PARALLAX_JEV_MODE_TOOL_RISK` (`off` | `shadow`), `off` in code. `on` MUST be rejected at startup in 067. Any other value, or `shadow` without the key, MUST be a startup error.
- **FR-011**: The design corpus MUST be amended where it says the gate has no model pass: `WATCHDOG_LAYER.md` :56 and :84-86, the 006 spec FR-013 and research D4, and the "No model" comment at `src/checkpoint/gate.rs:5`.
- **FR-012**: The last task MUST set `PARALLAX_JEV_MODE_TOOL_RISK=shadow` and the widened matcher in the owner's setup, with the exact keys shown first and a backup taken.

### Key Entities

- **Tool-risk record**: one Jev record per evaluated call at the new tool-risk site: class, class source, Jev answer and probabilities when asked, outcome, truncation flag, and the link to the checkpoint record.
- **Projection**: the redacted, reduced view of a tool call that Jev may see (User Story 3).
- **Allowlists**: the whole-command allowlist for `Bash`, and the per-tool argument-value allowlist for MCP tools.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In the spike, the widened matcher fires on 100% of the non-Parallax tool calls tried and on 0 Parallax tool calls.
- **SC-002**: With shadow on, the gate's verdicts and checkpoint records for a fixture set are identical to the mode-off run on every call, apart from timing and the new records.
- **SC-003**: With a Jev stand-in that never answers, every gate evaluation still returns within its existing budget.
- **SC-004**: No planted sentinel from a file body, heredoc body or non-allowlisted argument appears in any captured request body.
- **SC-005**: Every evaluated call in a live run leaves exactly one tool-risk record joined to its checkpoint record.
- **SC-006**: The memory hold fires on the same fixture calls with Jev answering `read_only` at 1.0, erroring, or never returning.

## Assumptions

- The per-tool argument-value allowlist starts empty: in 067 every MCP argument is sent by name only. Adding a field is a later, reviewed change.
- The whole-command allowlist covers test and build commands of the repository's own toolchain; its exact entries are set in planning.
- Code-classed calls record the class code assigns: risk-pattern matches as `spend_or_destructive`, allowlisted commands as `state_changing`, since test and build commands write build output.
- The Voyage embedding the gate already makes for memory recall (`Write` file contents included) is unchanged and is listed in the plan's "what leaves the machine" table.
- Turning the class into a flag, a note or a hold, and any promotion to `on`, are out of scope; they need a later decision because the gate's contract says it never flags.
- Memory questions to Jev are out of scope.

## Source

The feature description given to `/speckit-specify` (2026-11-26):

> Jev on tool calls (067, U3), shadow only. The checkpoint gate (checkpoint_action) classifies every matched tool call with one Jev choice question, action_class, options in order spend_or_destructive, state_changing, read_only, model pinned to jev-1.14.0. First task: a hooks spike. A widened PreToolUse matcher includes MCP tools and excludes mcp__parallax__*, verified with no self-trigger; a test parses hooks.json and checks the matcher; integrations/claude-code/hooks.json, its README and .claude/settings.json are updated. run_action (src/checkpoint/run.rs:165) is restructured so every matched call is classified, including with memory off. Deterministic order: risk patterns first, then an allowlist of whole commands only (no &&, ;, |, $( or backticks); Jev sees only the remainder. Jev runs alongside the gate with a deadline sized to GATE_BUDGET_MS (500, src/checkpoint/mod.rs:40) and no retries; the memory constraint hold stands whatever Jev answers; a contract test keeps the Jev budget plus GATE_BUDGET_MS well under the 5 s hook timeout (hooks.json:18). Only projections are sent, after the mandatory redactor: the Bash command with heredoc bodies dropped; file_path only for Write and Edit; for MCP tools the tool name plus argument key names, with values only for fields on a per-tool allowlist. If redaction fails, nothing is sent. Shadow only: answers are recorded in jev_records and nothing changes; the gate still never flags (contract.rs:5-6). Mode PARALLAX_JEV_MODE_TOOL_RISK (off | shadow), off in code; the last task sets shadow and the widened matcher in the owner's setup. Amendments: WATCHDOG_LAYER.md :56 and :84-86, the 006 spec FR-013 and research D4 text, gate.rs:5's "No model" comment. Out of scope: turning it on, any flag or note from the result, memory questions.
