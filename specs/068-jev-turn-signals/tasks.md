# Tasks: Jev turn signals (068)

**Input**: [spec.md](spec.md), [plan.md](plan.md), [contracts/questions.md](contracts/questions.md)

Test-first throughout: each test task is written and run to fail before its implementation task. The full gate runs before each commit.

## Phase 1: Foundation (blocks the stories)

- [ ] T001 Test the site and configuration (FR-011, plan D6): `JevSite::TurnSignals` listed as `turn_signals` / `TURN_SIGNALS`; `PARALLAX_JEV_MODE_TURN_SIGNALS` off by default, `shadow` accepted, `on` refused, `shadow` without the key refused; `PARALLAX_JEV_SEND_USER_TEXT` accepted as `true`/`false`, false unless set, other values refused. Files: `src/config.rs`, `src/traits/classifier.rs` tests.
- [ ] T002 Implement T001; the startup line names what is sent (plan D8); `--help` and the README list both variables. Files: `src/config.rs`, `src/traits/classifier.rs`, `src/jev/startup.rs`, `src/main.rs`, `README.md`.
- [ ] T003 Test the two questions word for word against `contracts/questions.md` (FR-001): ids, version 1, yes/no, criteria `true` then `false`. Files: `src/jev/questions.rs` tests.
- [ ] T004 Implement the questions. Files: `src/jev/questions.rs`.
- [ ] T005 Test the record contract (FR-012, plan D3, D7): the site enum lists `turn_signals`; the `cooldown_suppressed` column is bound two ways to the contract; a table written before 068 gains the column through the migration. Files: `src/storage/jev.rs`, `src/storage/sqlite.rs` tests.
- [ ] T006 Implement T005. Files: `specs/064-jev-first-pass/contracts/jev-record.schema.json`, `src/storage/jev.rs`, `src/storage/sqlite.rs`, every `JevRecord` constructor.

## Phase 2: User Story 3 - 066's detectors are removed (P1)

- [ ] T007 Remove the plan D5 list: code, migration, report code, example, `Cargo.toml` entry, fixtures' fields, and the documents that name them (066's own spec files stay). Check: a repository search finds no `CHECKPOINT_TURN_SIGNALS`, `turn_signal_records` or `TurnSignalsDeps` outside `specs/066-*`, and the full gate passes. Files: plan D5.

## Phase 3: User Story 1 - The two turn signals are asked of Jev, recorded only (P1)

- [ ] T008 Test the date and number stripping (FR-004, plan D2): a rule table of final-message claims (digits, number words, months, weekdays, relative dates) taken out of the shared state, counted, and every other claim kept. Files: `src/jev/shadow/signals.rs` tests.
- [ ] T009 Implement T008 with `claim_exclusion`. Files: `src/jev/shadow/signals.rs`.
- [ ] T010 Test the shared state on the wire (plan D1, FR-009, SC-002, User Story 2): the whole serialized body holds the previous final message, the stripped final message and the turn's tool inputs and outputs, redacted and capped; no user text without `PARALLAX_JEV_SEND_USER_TEXT=true`; with it, only that turn's user text; a redaction failure sends nothing and is recorded. Files: `src/jev/shadow/signals.rs` tests.
- [ ] T011 Implement the state and the shadow request (one request, both questions, the pinned model, one attempt). Files: `src/jev/shadow/signals.rs`, `src/jev/shadow.rs`.
- [ ] T012 Test the wiring at turn end (SC-001, User Story 1 scenarios 1, 3, 4): one row per non-continuation turn end with a final message; continuation turns never flag and make no request; site off makes no request and writes no row; the checkpoint result and record equal the site-off run; Jev errors, timeouts and wrong-model answers are recorded only. Files: `src/checkpoint/run/tests/turn_signals_jev.rs`.
- [ ] T013 Implement the wiring. Files: `src/checkpoint/run.rs`, `src/server.rs`.
- [ ] T014 Test the would-be cooldown record (FR-007, plan D3): a second yes for the same session and signal kind inside `COOLDOWN_WINDOW_MS` lists that kind in `cooldown_suppressed`; a different session or kind does not. Files: `src/checkpoint/run/tests/turn_signals_jev.rs`.
- [ ] T015 Implement T014. Files: `src/jev/shadow/signals.rs`, `src/checkpoint/run.rs`.
- [ ] T016 Test the back-test report's downstream-cost estimate (FR-008, plan D4) from fixture rows and invocation records. Files: `src/jev/report/` tests.
- [ ] T017 Implement T016. Files: `src/jev/report/`.

## Phase 4: Polish and acceptance

- [ ] T018 The stance-exception amendment in `WATCHDOG_LAYER.md` and `NEW_SERVER_DESIGN.md` (plan D9), and the README, CLAUDE.md and CHANGELOG text. Exact text shown to the owner first.
- [ ] T019 Live example `examples/acceptance_jev_signals.rs` (SC-005): exits non-zero unless SC-001 to SC-003 hold.
- [ ] T020 Full gate, independent review, live acceptance, PR.

## Phase 5: User Story 4 - On in the owner's setup (P2)

- [ ] T021 After merge and rebuild (FR-014, §4.1): back up `~/.claude.json`; set `PARALLAX_JEV_MODE_TURN_SIGNALS=shadow`, set `PARALLAX_JEV_SEND_USER_TEXT` only if the owner chooses, remove `CHECKPOINT_TURN_SIGNALS`; exact keys shown first; the owner restarts.

## Dependencies

- Phase 1 before Phases 2-3; T007 before T012 (the old wiring goes before the new one lands in `run_turn`).
- T009 before T011; T011 before T013; T013 before T015.
- Phase 4 after Phases 2-3; T021 after merge.
