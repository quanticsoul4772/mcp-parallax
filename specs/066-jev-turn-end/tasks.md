# Tasks: Jev at turn end (066)

**Input**: `specs/066-jev-turn-end/` (spec.md, plan.md, research.md, data-model.md, contracts/, quickstart.md)

**Rules for every task:**

- Each code task writes its test first, runs it to confirm it fails, then implements.
- The full gate runs before every commit.

Format: `- [ ] Txxx [P?] [Story?] Description (files)`. `[P]` means it can run in parallel: different files, no open dependency.

## Phase 1: Setup

- [x] T001 Transcript spike (research D1): read real Claude Code transcripts on the operator's machine for shape only, and write scrubbed fixtures for each line kind: user prompt (string and text blocks), subagent line, hook-injected context, interruption marker, meta line, compaction summary, tool input, tool result (string and block content). Record the field names that tell them apart in `research.md` D1. Not covered, recorded in D1: a Stop-hook block reason (not observed in any local transcript) and a `surface` block (hook output, carried by `attachment` lines; no surface fixture). Files: `tests/fixtures/transcripts/`.
- [x] T002 [P] Test that the committed fixtures contain no user names, no paths with an account name, no session ids and no real prompt text, and that each fixture's user prompt and tool results carry a sentinel string. Files: `tests/fixtures/transcripts/` test, `tests/integration.rs` or a new test module.

## Phase 2: Foundation (blocks all stories)

- [x] T003 Test that `TrajectoryReader::read` returns the window and the raw tail lines from one read, with the existing session check, and that the window is unchanged. Files: `src/traits/trajectory.rs` tests.
- [x] T004 Implement the `(TrajectoryWindow, Vec<String>)` return; update every caller and every mock stub (empty line list). Files: `src/traits/trajectory.rs`, `src/checkpoint/run.rs`, other callers.
- [x] T005 [P] Test the flag prefixes: each template (review, preference, batch, hold, surface) builds its message from an exported prefix constant. Files: `src/checkpoint/review.rs`, `preference.rs`, `run.rs`, `src/memory/push.rs` tests.
- [x] T006 Implement the exported prefix constants and use them in the templates. Files: same as T005.
- [x] T007 [P] Test `parse_turn` against every T001 fixture: turn start found (line id) or not found; excluded kinds and lines containing a flag prefix anywhere are skipped; user prompt, tool inputs and outputs, tool calls after the prompt. Round trip (SC-006): each flag built through its template (review, preference, batch, hold, surface) is skipped by the turn-start finder. Files: `src/checkpoint/turn_view.rs` tests.
- [x] T008 Implement `parse_turn`. Files: `src/checkpoint/turn_view.rs`, `src/checkpoint/mod.rs`.
- [ ] T009 [P] Test entity extraction and lookup (research D6): paths with backslashes and JSON escapes normalize to `/`; version strings; identifiers and hashes of 7+ characters; numbers of 3+ digits on word boundaries (`142` does not match `42`); no names, no dates; unmatched entities come back with kind and hash. Files: `src/jev/entities.rs` tests.
- [ ] T010 Implement `src/jev/entities.rs` and add it to `src/jev/mod.rs`.
- [ ] T011 [P] Test the `turn_signal_records` table: two-way binding with `contracts/turn-signal-record.schema.json` via `PRAGMA table_info`; write and read a row; the `review_skipped_by` migration on an old-shape database. Files: `src/storage/turn_signals.rs` tests, `src/storage/sqlite.rs` tests.
- [ ] T012 Implement the table, `record_turn_signals` on `Storage`, and the `review_skipped_by` migration. Files: `src/storage/turn_signals.rs`, `src/storage/sqlite.rs`, `src/traits/storage.rs`.

## Phase 3: User Story 1 - Turn signals recorded (P1)

**Goal**: with `CHECKPOINT_TURN_SIGNALS=on`, each covered turn end writes one row with both detector results; nothing changes in the turn and nothing new is sent.

**Independent test**: fixture transcripts give the expected rows; the checkpoint result and record are unchanged with the signals on and off.

- [ ] T013 [P] [US1] Test the pushback detector (research D5): fires on a candidate pair after a real user prompt with no tool call after it; records `tool call after prompt`, `no previous final message` and `turn start not found` in their cases; runs the pair miner with no stored memories; recommends `verify` (FR-007); fires and is recorded on each of several consecutive turns of one session, with no cooldown (FR-008). Files: `src/checkpoint/signals.rs` tests.
- [ ] T014 [P] [US1] Test the unverified-fact detector: fires when an entity is missing from the turn's tool inputs, outputs and user prompt; not fired when all are found; records unmatched kinds and hashes; recommends `grounded_verify` when configured, else `research` when configured, else none (FR-007); fires and is recorded on each of several consecutive turns (FR-008). Files: `src/checkpoint/signals.rs` tests.
- [ ] T015 [US1] Implement `src/checkpoint/signals.rs`: the previous-final map (updated at every turn end, continuation included), both detectors, the recommendation mapping (FR-007), and the row.
- [ ] T016 [US1] Test `run_turn` with the signals on: one row per covered turn end, whether or not the review ran; no row on continuation turns; a transcript read failure records `transcript unreadable`; the checkpoint result and record are identical with the signals on and off (SC-001); no sentinel string in any request body (SC-002); with `CHECKPOINT_TURN_SIGNALS` off, `parse_turn` is not called and no row is written (US3 scenario 1). Files: `src/checkpoint/run.rs` tests.
- [ ] T017 [US1] Wire the signals into `run_turn` and `CheckpointDeps`. Files: `src/checkpoint/run.rs`, `src/server.rs`.

## Phase 4: User Story 2 - Review skip, off until measured (P2)

**Goal**: the skip is built and tested through injected thresholds and stays off.

**Independent test**: each skip condition made false or missing makes the review run; `on` fails startup.

- [ ] T018 [P] [US2] Test the turn-review questions at version 2 with the criteria in `contracts/questions.md`, `true` first, each built as its own question value; the research `same_subject` questions stay at version 1 with no criteria (`noul()` unchanged). Files: `src/jev/questions.rs` tests.
- [ ] T018a [P] [US2] Test that the report separates turn-review rows by question version and feeds only version-2 rows to the promotion decision. Files: `src/jev/report/tests.rs`.
- [ ] T018b [US2] Implement the version separation in the report. Files: `src/jev/report.rs`.
- [ ] T019 [US2] Implement version 2 as separate question values, leaving `noul()` unchanged; correct the comment at `src/traits/classifier.rs:109-111` (options are not always empty for yes/no questions). Files: `src/jev/questions.rs`, `src/traits/classifier.rs`.
- [ ] T019a [P] [US2] Test the saved measurements: noise, self-agreement and order results carry the question version; a version-2 run does not overwrite version-1 results; `reproduce` prints the version. Files: `src/jev/measure.rs` tests.
- [ ] T019b [US2] Implement the version field in the saved measurements. Files: `src/jev/measure.rs`, `examples/jev_backtest/`.
- [ ] T019c [P] [US2] Test the swapped turn-review questions (criteria order reversed) and a yes/no order result type that is saved and reproduced. Files: `src/jev/questions.rs` tests, `src/jev/measure.rs` tests, `src/client/typesafe/tests.rs`.
- [ ] T019d [US2] Implement the swapped turn-review questions, the yes/no order result type, and the harness subcommand that runs both orders. Files: `src/jev/questions.rs`, `src/jev/measure.rs`, `examples/jev_backtest/`.
- [ ] T020 [P] [US2] Test the skip decision: pinned tables keyed by Jev model, big-model id and question versions (empty); memory required; any candidate blocks; truncated state blocks; the audit share from `fnv1a64(eval_id)`. Files: `src/jev/policy.rs` tests.
- [ ] T021 [US2] Implement the tables and the decision. Files: `src/jev/policy.rs`.
- [ ] T022 [US2] Test `run_turn` with injected thresholds and evaluation ids: a skipped turn sets `review_skipped_by = jev` and the 064 row's `applied = 1`; an audited turn runs the review and sets `audit`; any failed condition runs the review; Jev's added time is at most the turn deadline (SC-005); with the skip off, today's concurrent order is unchanged. Files: `src/checkpoint/run.rs` tests.
- [ ] T023 [US2] Implement the skip path, the injection points in `CheckpointDeps`, `review_skipped_by` on the record, and its OTLP mirror. Files: `src/checkpoint/run.rs`, `src/checkpoint/mod.rs`, `src/observability.rs`.

## Phase 5: User Story 3 - Configuration and records (P3)

- [ ] T024 [P] [US3] Test configuration: `CHECKPOINT_TURN_SIGNALS` defaults to `off`; an unparseable value is a startup error; `PARALLAX_JEV_*` names outside the known set are startup errors; `PARALLAX_JEV_MODE_TURN_REVIEW=on` is refused (SC-003). Files: `src/config.rs` tests.
- [ ] T025 [US3] Implement the configuration, the turn-review `on` startup wording, `--help` and the README row. Files: `src/config.rs`, `src/jev/startup.rs`, `src/main.rs`, `README.md`.
- [ ] T026 [P] [US3] Test the `checkpoint_turn` signal-enum binding to its contract. Files: `src/checkpoint/contract.rs` tests.
- [ ] T027 [US3] Contracts: the checkpoint record contract gains `review_skipped_by`; `specs/007-observability-layer/contracts/telemetry.md` gains the attribute. Files: `specs/006-checkpoint-layer/contracts/`, `specs/007-observability-layer/contracts/telemetry.md`.

## Phase 6: Polish and cross-cutting

- [ ] T028 [P] `jev_backtest report` covers `turn_signal_records` (fire rates and exclusion reasons). Files: `examples/jev_backtest/`, `src/jev/report.rs`.
- [ ] T029 Live example `examples/acceptance_turn_signals.rs` (SC-004): fixtures through `run_turn` with the signals on and a scratch database; one joined row per covered turn end with the expected results; no sentinel string in any request body; exits non-zero on failure. Register it in `Cargo.toml`.
- [ ] T030 Amendments (FR-013): `docs/design/MEMORY_LAYER.md`, `docs/design/WATCHDOG_LAYER.md:59`, `docs/design/CORRECTIVE_SELECTION.md:89`, `docs/design/NEW_SERVER_DESIGN.md` §10, `specs/017-memory-consolidation/spec.md`, and the CHANGELOG.
- [ ] T031 Analyze (Spec Kit) across spec, plan and tasks, with the three reviewers on the version-2 criteria in `contracts/questions.md`.
- [ ] T032 Full gate; independent review by a fresh general-purpose subagent given only the diff, plus the constitution's `design-reviewer` and `code-reviewer` agents.
- [ ] T033 Push and PR; CI.
- [ ] T034 After merge and rebuild, run the live example (quickstart §3); results go to `quickstart.md`.
- [ ] T034a After merge and rebuild, re-run the noise and self-agreement measurements, and run the order check in both criteria orders (research D9, FR-010), on the version-2 turn-review questions with `jev_backtest`; results go to the 064 measurements doc under a 066 section.
- [ ] T035 Update the CLAUDE.md active-feature block to the shipped state.

## Dependencies

- Phase 1 blocks Phase 2's parser tasks (T007 needs T001's fixtures).
- Phase 2 blocks every story. T003 → T004; T005 → T006; T007 → T008 (needs T006); T009 → T010; T011 → T012.
- US1 (T013–T017) needs T004, T008, T010, T012. US2 (T018–T023) needs T012. US3 (T024–T027) needs T012.
- Within US2: T018a → T018b → T019 (the report separates versions before version 2 is sent); T018 → T019; T019a → T019b; T019c → T019d (needs T019 and T019b).
- Phase 6 needs all stories; T031 → T032 → T033 → merge → T034 → T034a → T035.

## Parallel examples

- Phase 2: T005, T007, T009 and T011 tests can be written together.
- US1: T013 and T014 tests together.
- US2: T018, T018a, T019a, T019c and T020 tests together.

## Implementation strategy

MVP is Phases 1-3: the turn signals recorded with anchors. US2 builds the skip behind injected thresholds and leaves it off. US3 publishes the configuration and contracts. Phase 6 ships it.
