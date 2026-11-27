# Tasks: Jev on tool calls (067)

**Input**: `specs/067-jev-tool-risk/` (spec.md, plan.md, research.md, data-model.md, contracts/, quickstart.md). Every task cites the spec item it carries out.

**Rules for every task:**

- Each code task writes its test first, runs it to confirm it fails, then implements.
- The full gate runs before every commit.

Format: `- [ ] Txxx [P?] [Story?] Description (files)`. `[P]` means it can run in parallel: different files, no open dependency.

## Phase 1: Setup

- [x] T001 Spike (FR-001, §6.3.1, SC-001): install a widened `PreToolUse` matcher that includes MCP tools and excludes `mcp__parallax__*` in a test session; confirm in a live session that `checkpoint_action` runs for a non-Parallax tool, does not run for a Parallax tool, and that its own call does not trigger it. Record the verified pattern and result in `research.md` D1.

## Phase 2: Foundation (blocks all stories)

- [x] T002 [P] Test the site and config (FR-010, contracts/config.md): `JevSite::ToolRisk` (`tool_risk`, suffix `TOOL_RISK`) in `JevSite::ALL`; `PARALLAX_JEV_MODE_TOOL_RISK` defaults to `off`; `shadow` without the key, `on`, and any other value are startup errors; the startup log line names the host and data class; `--help` and the README list the variable. Files: `src/traits/classifier.rs`, `src/config.rs`, `src/main.rs` tests.
- [x] T003 Implement T002, plus the operator-owned row in `docs/design/NEW_SERVER_DESIGN.md` §10 (contracts/config.md) and the new field in every `Config` test fixture. Files: `src/traits/classifier.rs`, `src/config.rs`, `src/main.rs`, `README.md`, `docs/design/NEW_SERVER_DESIGN.md`, `tests/common/mod.rs`, `examples/common/mod.rs`.
- [x] T004 [P] Test the `tool_risk` branch of `jev-record.schema.json` (contracts/record-additions.md): two-way binding through `JevSite::ALL`; `state_json` null; `applied` 0; `label_json` the big model's `action_class` answer on Jev rows, with `label_source`, and null on excluded rows (data-model.md). Files: storage and contract tests.
- [x] T005 Implement the schema branch. Files: `specs/064-jev-first-pass/contracts/jev-record.schema.json`, `src/storage/jev.rs`.
- [x] T006 [P] Test the `action_class` question (FR-004, contracts/questions.md): id, version 1, choice, options in the order `spend_or_destructive`, `state_changing`, `read_only`, the instructions and criteria exactly as in the contract, model pinned. Files: `src/jev/questions.rs` tests.
- [x] T007 Implement the question. Files: `src/jev/questions.rs`.

## Phase 3: User Story 1 - The hook reaches every tool except Parallax's own (P1)

**Goal**: the widened matcher ships, checked by a test (FR-001).

- [x] T008 [US1] Test (§11 test 1, matcher binding): parse `integrations/claude-code/hooks.json` and check the matcher against the pattern T001 verified. Files: `src/checkpoint/contract.rs` tests.
- [x] T009 [US1] Update the matcher in `integrations/claude-code/hooks.json`, its README and `.claude/settings.json` to the T001 pattern.

## Phase 4: User Story 2 - Every matched call is classified, recorded only (P1)

**Goal**: every matched call gets a code exclusion or a Jev answer, recorded; nothing the user sees changes (FR-002, FR-003, FR-005, FR-008's no-behavior-change part, FR-009).

- [x] T010 [P] [US2] Test the pre-filter table (§11 test 2, FR-003): risk-pattern matches and allowlisted whole commands are excluded with no Jev request (`times(0)` at the classifier); a command containing `&&`, `;`, `|`, `$(`, a backtick, `&`, `>`, `>>`, `<`, `<(` or `>(` never matches the allowlist. Files: `src/checkpoint/tool_risk.rs` tests.
- [x] T011 [US2] Implement the order and the allowlist (research D2). Files: `src/checkpoint/tool_risk.rs`, `src/checkpoint/mod.rs`.
- [ ] T012 [US2] Test `run_action` (FR-002, FR-005, FR-008, FR-009, SC-003; §11 tests 4 and 5): every matched call is classified, memory on or off; a newly matched tool gets no hold; a risk-matched `Bash`, `Write` or `Edit` hold is unchanged when Jev hangs, errors, or answers `read_only` at 1.0; with a never-returning classifier the gate still returns a response in the same test run, because its response path never awaits the Jev request (SC-003); the gate's verdict and message are identical with the site off and `shadow`; excluded calls leave an `excluded` row with the reason in `error_class`; a Jev answer leaves its row when it arrives; one attempt, deadline `GATE_BUDGET_MS`. Files: `src/checkpoint/run/tests/tool_risk.rs`.
- [ ] T013 [US2] Implement the `run_action` restructure and the Jev request (research D4, D5). Files: `src/checkpoint/run.rs`, `src/jev/shadow/tool.rs`, `src/jev/shadow.rs`, `src/server.rs`.
- [ ] T014 [P] [US2] Contract test (§11 test 6, FR-005): the Jev deadline plus `GATE_BUDGET_MS` is at most a quarter (1.25 s) of the `checkpoint_action` timeout read from `hooks.json`. Files: `src/checkpoint/contract.rs` tests.
- [ ] T015 [P] [US2] §11 test 7, "memory questions off without the opt-in": test that the tool-risk site asks Jev no memory question at all (owner's answer, 2026-11-27). Files: `src/jev/questions.rs` or `src/jev/shadow/tool.rs` tests.

## Phase 5: User Story 3 - Only projections leave the machine (P1)

**Goal**: Jev sees only the projections (FR-006).

- [ ] T016 [US3] Test the projections (§11 test 3, FR-006): `Bash` command with heredoc bodies dropped, redacted; `file_path` only for `Write` and `Edit`; MCP and other built-in tools as name plus argument key names, values only for allowlisted fields; a redaction failure sends nothing and is recorded; wire-level assertions on the whole serialized body. Files: `src/checkpoint/tool_risk.rs` tests.
- [ ] T017 [US3] Implement the projections (research D3). Per-tool argument-value allowlist: `file_path`, `path` and `pattern` of `Read`, `Grep` and `Glob` (owner's answer, 2026-11-27). Files: `src/checkpoint/tool_risk.rs`.

## Phase 6: Polish and acceptance

- [ ] T018 Amendments (FR-011): `docs/design/WATCHDOG_LAYER.md:56` and `:84-86`, the 006 FR-013 and D4 text, the "No model pass" comment at `src/checkpoint/gate.rs:5-7`. Exact text shown to the owner first.
- [ ] T019 FR-007: add the Voyage row (Write file contents through the existing gate embed) to the roadmap's §8 table. Exact text shown to the owner first.
- [ ] T020 Live example `examples/acceptance_tool_risk.rs` (SC-002, SC-004; §11 test 8, live no-self-trigger): exits non-zero unless every SC-002 assertion holds and Jev adds zero time to the critical path (SC-004).
- [ ] T021 Load run (SC-005): the live example with at least two sessions plus a research run at once; record hook p99 in `quickstart.md`.
- [ ] T022 CHANGELOG `[Unreleased]` entry and the CLAUDE.md active-feature and configuration lines (roadmap §10). Exact text shown to the owner first.
- [ ] T023 Full gate (SC-006), independent review, live acceptance, PR (roadmap line 163).

## Phase 7: User Story 4 - On in the owner's setup (P2)

- [ ] T024 [US4] After merge and rebuild (FR-012, §4.1 line 186): back up `~/.claude.json`, set `PARALLAX_JEV_MODE_TOOL_RISK=shadow` in the Parallax entry and the widened matcher in the owner's hook config, exact keys shown first; the owner restarts.

## Dependencies

- T001 before T008 and T009.
- Phase 2 before Phases 3-5.
- T011 before T012; T016 and T017 before T013 (the request is built from the projections).
- Phase 6 after Phases 3-5; T024 after merge.
