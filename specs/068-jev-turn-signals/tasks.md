# Tasks: Jev turn signals (068)

**Input**: [spec.md](spec.md), [plan.md](plan.md), [contracts/questions.md](contracts/questions.md)

Test-first throughout: each test task is written and run to fail before its implementation task. The full gate runs before each commit.

## Phase 1: Foundation (blocks the stories)

- [x] T001 Test the site and configuration (FR-011, plan D6): `JevSite::TurnSignals` listed as `turn_signals` / `TURN_SIGNALS`; `PARALLAX_JEV_MODE_TURN_SIGNALS` off by default, `shadow` accepted, `on` refused, `shadow` without the key refused; `PARALLAX_JEV_SEND_USER_TEXT` accepted as `true`/`false`, false unless set, other values refused. Files: `src/config.rs`, `src/traits/classifier.rs` tests.
- [x] T002 Implement T001; the startup line names the host (`api.typesafe.ai`) and the data sent (plan D8, §5.1); `--help` and the README list both variables; `NEW_SERVER_DESIGN.md` §10 classifies both as operator-owned; TypeSafe's terms (§8) appear where the Jev docs and startup log carry them. Exact doc text shown to the owner first (roadmap check items 14, 15, 36). Files: `src/config.rs`, `src/traits/classifier.rs`, `src/jev/startup.rs`, `src/main.rs`, `README.md`.
- [x] T003 Test the two questions word for word against `contracts/questions.md` (FR-001): ids, version 1, yes/no, criteria `true` then `false`. Files: `src/jev/questions.rs` tests.
- [x] T004 Implement the questions. Files: `src/jev/questions.rs`.
- [x] T005 Test the record contract (FR-012, plan D3, D7): the site enum lists `turn_signals`; the `cooldown_suppressed` column is bound two ways to the contract; a table written before 068 gains the column through the migration. Files: `src/storage/jev.rs`, `src/storage/sqlite.rs` tests.
- [x] T006 Implement T005. Files: `specs/064-jev-first-pass/contracts/jev-record.schema.json`, `src/storage/jev.rs`, `src/storage/sqlite.rs`, every `JevRecord` constructor.

## Phase 2: User Story 3 - 066's detectors are removed (P1)

- [x] T007 Remove the plan D5 list: code, migration, report code, example, `Cargo.toml` entry, fixtures' fields, and the documents that name them (066's own spec files stay). Check: a repository search finds no `CHECKPOINT_TURN_SIGNALS`, `turn_signal_records` or `TurnSignalsDeps` outside `specs/066-*` and `specs/068-*`, and the full gate passes, including 066's U2 review-skip tests (`src/checkpoint/run/tests/review_skip.rs`), which must still pass unchanged (roadmap check items 20, 32, 38). Files: plan D5.

## Phase 3: User Story 1 - The two turn signals are asked of Jev, recorded only (P1)

- [x] T008 Test the date and number stripping (FR-004, plan D2): a rule table of final-message claims (digits, number words, months, weekdays, relative dates) taken out of the shared state, counted, and every other claim kept. Files: `src/jev/shadow/signals.rs` tests.
- [x] T009 Implement T008 with `claim_exclusion`. Files: `src/jev/shadow/signals.rs`.
- [x] T009a Test the number and date difference check (plan D2a, §3.2): a final text whose number or date differs from one in the earlier statement of a transcript pair sets `number_diff` true on the `pushback_reversal` entry; matching or absent numbers set it false; Jev is still asked either way. Then implement it. Files: `src/jev/shadow/signals.rs`.
- [x] T010 Test the two states on the wire (plan D1, D1a, FR-009, SC-002, User Story 2), asserting on each whole serialized body: `pushback_reversal`'s holds the turn activity summary, the transcript-only statement pairs and the final message; `unverified_fact`'s holds the turn activity summary, the transcript-only statement pairs and the final message with date and number claims removed, never user text; each request's final message is cut to 2000 characters, with `truncated` set on a cut; no user text in either without `PARALLAX_JEV_SEND_USER_TEXT=true`, and with it only that turn's user text in `pushback_reversal`'s; a redaction failure sends nothing and is recorded. Files: `src/jev/shadow/signals.rs` tests.
- [x] T011 Implement the two states and requests (one per question, the pinned model, one attempt, deadline `JEV_TURN_DEADLINE_MS`), the per-question `recommends` field and the `pushback_reversal` `number_diff` field (plan D2a, D7); no label call (`label_json` null). Files: `src/jev/shadow/signals.rs`, `src/jev/shadow.rs`.
- [x] T012 Test the wiring at turn end (SC-001, User Story 1 scenarios 1, 3, 4, plan D1a, D7a): the requests run off the Stop hook's response path, never awaited; one row per question per non-continuation turn end with a final message; the turn's checkpoint `jev_cost_usd` gains each row's cost and a `parallax.jev.turn_signals` span is exported per row; continuation turns never flag and make no request; site off makes no request and writes no row; the checkpoint result and record equal the site-off run; Jev errors, timeouts and wrong-model answers are recorded only. Files: `src/checkpoint/run/tests/turn_signals_jev.rs`.
- [x] T013 Implement the wiring. Files: `src/checkpoint/run.rs`, `src/server.rs`.
- [x] T014 Test the would-be cooldown record (FR-007, plan D3): a second yes for the same session and signal kind inside `COOLDOWN_WINDOW_MS` lists that kind in `cooldown_suppressed`; a different session or kind does not. Files: `src/checkpoint/run/tests/turn_signals_jev.rs`.
- [x] T015 Implement T014. Files: `src/jev/shadow/signals.rs`, `src/checkpoint/run.rs`.
- [x] T015a Test (§6.2, plan D10, roadmap check item 24): window eviction (`WINDOW_ENTRIES` = 200) doesn't change existing signals: a transcript longer than the window yields the same batch-screen signals and turn-review candidates as its last 200 entries alone. Files: `src/checkpoint/run/tests/` or `src/traits/trajectory.rs` tests.
- [x] T015b Catalog check (FR-006, §11, roadmap check items 30 and 39), test-first: the `recommends` field names `verify` for `pushback_reversal`; for `unverified_fact` it names `grounded_verify` only when configured, else `research` only when configured, else none. Files: `src/jev/shadow/signals.rs` tests.
- [x] T015c After T007, confirm the signal-enum binding test (`src/checkpoint/contract.rs`, `the_turn_contract_lists_exactly_the_kinds_a_turn_can_fire`) still passes; check the `parallax.checkpoint.signals` list in `specs/007-observability-layer/contracts/telemetry.md` against the fired-signal kinds and correct it if it differs (FR-010).
- [x] T016 Test the back-test report's downstream-cost estimate (FR-008, plan D4) from fixture rows and invocation records. Files: `src/jev/report/` tests.
- [x] T017 Implement T016. Files: `src/jev/report/`.
- [x] T017a Print the downstream-cost estimate in the back-test report (plan D4): the `report` subcommand in `examples/jev_backtest/offline.rs` loads each invocation record's tool and `cost_usd` and prints `jev::report::cost::summary_lines` after the existing tables. Files: `examples/jev_backtest/offline.rs`, plus a storage reader for invocation records if none exists.

## Phase 4: Polish and acceptance

- [x] T018 The stance-exception amendment in `WATCHDOG_LAYER.md` and `NEW_SERVER_DESIGN.md` (plan D9), and the README, CLAUDE.md and CHANGELOG text. Exact text shown to the owner first.
- [ ] T019 Live example `examples/acceptance_jev_signals.rs` (SC-005): exits non-zero unless SC-001 to SC-003 hold.
- [ ] T019a After the live run (§4.1, roadmap check item 11): count the turn-signal rows and yes answers so far, and report in the plan the data still needed for §5.4 (60 hand-labeled items) and §7.3 (60 flags, the false-positive bound, a recall floor) and the expected date at the observed rate.
- [ ] T020 Full gate, independent review, live acceptance, PR.

## Phase 5: User Story 4 - On in the owner's setup (P2)

- [ ] T021 After merge and rebuild (FR-014, §4.1): back up `~/.claude.json`; set `PARALLAX_JEV_MODE_TURN_SIGNALS=shadow`, set `PARALLAX_JEV_SEND_USER_TEXT` only if the owner chooses, remove `CHECKPOINT_TURN_SIGNALS`; the exact keys and values shown first (§4.1); `PARALLAX_JEV_MODE_TURN_REVIEW` stays `shadow` until §7 passes; the owner restarts.

## Dependencies

- Phase 1 before Phases 2-3; T007 before T012 (the old wiring goes before the new one lands in `run_turn`).
- T009 before T011; T011 before T013; T013 before T015.
- Phase 4 after Phases 2-3; T021 after merge.
