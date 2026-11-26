# Tasks: Jev injection warning on research sources (065)

**Input**: `specs/065-jev-injection-warning/` (spec.md, plan.md, research.md, data-model.md, contracts/, quickstart.md)

**Rules for every task:**

- Each code task writes its failing test first, shows it failing, then implements.
- The full gate runs before every commit.
- Every step (each read, edit, commit and push) is asked first.
- Doc and contract text is shown as exact text before it is written.

Format: `- [ ] Txxx [P?] [Story?] Description (files)`. `[P]` means it can run in parallel: different files, no open dependency.

## Phase 1: Foundation (blocks all stories)

- [x] T001 Test the new site and mode: `JevSite::ResearchInjection` (suffix `RESEARCH_INJECTION`, stored as `research_injection`) is in `JevSite::ALL`; `JevMode::On` is in `JevMode::ALL` and parses from `on`; the `jev_records` two-way binding test covers the new site value. Files: `src/traits/classifier.rs` tests, `src/storage/jev.rs` tests.
- [x] T002 Implement the site and mode, and add `research_injection` to `specs/064-jev-first-pass/contracts/jev-record.schema.json` (exact text shown first). Files: `src/traits/classifier.rs`, the schema.
- [x] T003 [P] Test the pinned threshold table and decision: `injection_threshold("jev-1.14.0") == Some(0.35)`, any other model gives `None`; `injection_decision` warns at exactly 0.35 and not just below it (`next_down`), and a missing probability is never `Warned`; the 0.35 value is bound to the T038 (e) section of `claudedocs/jev-064-measurements-2026-11-24.md`. Files: `src/jev/policy.rs` tests.
- [x] T004 Implement `INJECTION_THRESHOLDS`, `injection_threshold` and `injection_decision`. Files: `src/jev/policy.rs`.
- [x] T005 [P] Test the `directs_ai` question: yes/no, version 1, exa-mcp-server's instructions and both criteria verbatim, criteria declared `true` then `false`; bound to the wording the back-test sent. Files: `src/jev/questions.rs` tests.
- [x] T006 Implement `injection_question()` in `src/jev/questions.rs`; `jev_backtest injection` uses it and drops its question-file argument (usage text and CHANGELOG updated). Files: `src/jev/questions.rs`, `examples/jev_backtest/main.rs`, `examples/jev_backtest/live.rs`.
- [x] T007 [P] Test config (research D2): `PARALLAX_JEV_MODE_RESEARCH_INJECTION` accepts `off`, `shadow` and `on`, defaults to `off`; non-off without the key is an error naming the variable; `on` with a model lacking a pinned threshold is an error naming both variables; `on` for `TURN_REVIEW` or `RESEARCH_CLAIMS` is still an error. Files: `src/config.rs` tests.
- [x] T008 Implement the config, and the new field in the three Config fixtures (`src/config.rs` `test_config`, `tests/common/mod.rs`, `examples/common/mod.rs`). Files: `src/config.rs`.
- [x] T009 [P] Test the client's retries: a request with `retry` false makes exactly one attempt for 429, 529 and 5xx (every 064 caller); with `retry` true it retries 429, 529 and 5xx, waiting `Retry-After` when given and otherwise 100, 200 then 400 ms, never past the request's deadline; a 401 and any other 4xx are never retried. Files: `src/client/typesafe/tests.rs`.
- [x] T010 Implement the `retry` flag on `JevRequest` (false at every 064 call site) and the client's retry loop with `client::retry::{retry_after, time_until}`. Files: `src/traits/classifier.rs`, `src/client/typesafe.rs`, the 064 call sites.

## Phase 2: User Story 1 - Research marks sources that address the AI (P1)

**Goal**: with the site `on`, kept pages scoring at or above 0.35 carry an advisory warning; nothing else changes.

**Independent test**: a stand-in Jev scoring one page high and one low marks only the first; every other field equals the site-off result.

- [x] T011 [P] [US1] Test the injection module (research D5): request shape (`{page_url, page_text}` redacted, pinned model, the `directs_ai` question); `on` requests set `retry` and shadow requests do not; the deadline is `min(2000 ms, time left)`; each failure maps to a screening reason (`timeout`, `classifier_provider`, `provider_rejected`, `validation_failure`, `redaction_failed`, `disabled`); a wrong answering model never warns; secret fixtures placed in the page URL and text never appear anywhere in the whole captured request body (SC-004). Files: `src/jev/shadow/injection.rs` tests.
- [x] T012 [US1] Implement `src/jev/shadow/injection.rs`: `InjectionJev` (classifier, redactor, model, mode, threshold, deadline), prepare, ask with on-site retries, the row, and the source screening result. Files: `src/jev/shadow/injection.rs`, `src/jev/shadow.rs`.
- [x] T013 [P] [US1] Test the pipeline (SC-002, SC-003): with the site `on`, each kept page's call runs beside its extraction call (research D1) on the capped text 063 screens; high and low pages, an erroring, invalid and never-returning Jev; only `jev_injection`, `jev_warned_pages` and `jev_unscreened_pages` differ from the site-off result; a never-returning Jev never extends the run past its deadline. Files: `src/research/pipeline_tests.rs`.
- [x] T014 [US1] Wire `ResearchDeps.jev_injection` and the call beside `extract_source`; add the optional fields to `src/research/contract.rs` and fill them in `src/research/outcome.rs`; update the research output schema and its binding test to `contracts/research-result-additions.md`. Files: `src/research/pipeline.rs`, `contract.rs`, `outcome.rs`, `src/server.rs`.
- [x] T015 [US1] The research tool description and `docs/design/RESEARCH_PRIMITIVE.md` (research D8): the warning is advisory, "no warning" does not mean clean, and the measured rates; `docs/design/NEW_SERVER_DESIGN.md` §10 lists `PARALLAX_JEV_MODE_RESEARCH_INJECTION` as operator-owned (exact text shown first). Files: `src/server.rs`, the two design docs.

## Phase 3: User Story 2 - Shadow first, separate from the pattern screen (P1)

**Goal**: shadow records the same calls and changes nothing; 063 reports exactly as today in every mode.

**Independent test**: shadow equals site-off except elapsed time; 063's fields are identical in all three modes.

- [x] T016 [P] [US2] Test SC-001: with the site off and in shadow, the research result is byte-identical to the site-off result except `stats.elapsed_ms`, for a confident, erroring, invalid and never-returning Jev; with it off, no call and no row. Test that 063's `injection_patterns`, `pages_flagged` and `flagged_pages` are identical in off, shadow and on, including pages one screen flags and the other does not; a source Jev warned on but 063 did not flag still has its claims sent in 064's claim shadow. Files: `src/research/pipeline_tests.rs`.
- [x] T017 [US2] Make whatever T016 shows is needed (shadow path through the same module, fields omitted with `skip_serializing_if`). Files: `src/research/*`, `src/jev/shadow/injection.rs`.
  - No change needed: T016's tests passed against T014's code (2026-11-25).

## Phase 4: User Story 3 - Safe configuration and records (P2)

**Goal**: off unless set; misconfiguration stops startup; every call leaves a record the harness can read.

**Independent test**: config error tests (T007) and a records test joining every injection row to its invocation.

- [x] T018 [P] [US3] Test records (research D7) on `SqliteStorage`: one `research_injection` row per page request, joined to the invocation and source id; `label_json` holds the 063 pattern names; `state_json` null; `applied` 1 only for an `on` row that warned; rows written on cancellation; the invocation's `jev_cost_usd` includes injection rows; Jev tokens stay out of the research token ceiling. Files: `src/research/pipeline_tests.rs`, `tests/integration.rs` if needed.
  - Passed on first run against T012/T014; invocation linkage, cancellation and jev_cost_usd are covered by 064's site-independent RecordGuard tests (2026-11-25).
- [x] T019 [US3] Implement the rows through the run's `JevShadow` accumulator. Files: `src/research/pipeline.rs`, `src/jev/shadow/injection.rs`.
  - Done in T014 (2026-11-25).
- [x] T020 [P] [US3] Test the startup line (host and data class) and `--help`, README and the `PARALLAX_JEV_MODE_` prefix help test listing the new site and `on`. Files: `src/jev/startup.rs` tests, `src/main.rs` tests.
  - The startup test was added with T002; the help-and-README test covers the new site and `on` through `JevSite::ALL` and `JevMode::ALL` (2026-11-25).
- [x] T021 [US3] Implement the startup line and the help and README text (exact text shown first). Files: `src/jev/startup.rs`, `src/main.rs`, `README.md`.
  - The startup line, --help and README text were done early, with T002 (2026-11-25).

## Phase 5: Polish and cross-cutting

- [x] T022 [P] Test, then add the telemetry counts (research D9) `parallax.jev.injection.warned` and `parallax.jev.injection.unscreened` on the research invocation span, with `telemetry.md` updated (exact text first). Files: `src/observability.rs`, `specs/007-observability-layer/contracts/telemetry.md`.
- [x] T023 [P] Test, then extend `jev_backtest report` with the injection site from stored records (FR-009): share warned, failures by outcome, agreement with 063 on the same pages. Files: `src/jev/report.rs`, `src/jev/report/tests.rs`.
- [x] T024 Write the live example `examples/acceptance_jev_injection.rs` asserting SC-005, exiting non-zero on failure; register it in `Cargo.toml` with `test = true` for its helpers.
- [ ] T025 CHANGELOG `[Unreleased]` entry (exact text first). Files: `CHANGELOG.md`.
- [ ] T026 Full gate; independent review by a fresh general-purpose subagent given only the diff, plus the constitution's `design-reviewer` (design corpus) and `code-reviewer` (Rust conventions) agents, each brief shown first; each finding asked.
- [ ] T027 Account-name check, push and PR (text shown first); CI.
- [ ] T028 After merge and rebuild, the user runs the live example (quickstart §3); results go to `quickstart.md` (exact text first).
- [ ] T029 Turn it on in the user's setup (FR-010): back up `~/.claude.json`; add `PARALLAX_JEV_MODE_RESEARCH_INJECTION=on` to the Parallax `env` block, changing only that key, exact key shown first; the user restarts.
- [ ] T030 Update the CLAUDE.md active-feature block to the shipped state (exact text first).

## Dependencies

- Phase 1 blocks every story. T001 → T002; T003 → T004; T005 → T006; T007 → T008 (needs T002, T004); T009 → T010.
- US1 (T011–T015) needs Phase 1. US2 (T016–T017) needs T014. US3 (T018–T021) needs T012 and T014.
- Phase 5 needs all stories; T026 → T027 → merge → T028 → T029 → T030.

## Parallel examples

- Phase 1: T003, T005, T007 and T009 tests can be written together (different files).
- US1: T011 and T013 tests can be written together.
- US3: T018 and T020 tests can be written together.

## Implementation strategy

MVP is Phase 1 plus US1 (the warning with the site on). US2 confirms shadow and 063 are untouched; US3 hardens configuration and records; Phase 5 ships it and turns it on in the user's setup.
