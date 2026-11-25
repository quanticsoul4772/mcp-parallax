# Tasks: Jev foundation and shadow answers (064)

**Input**: `specs/064-jev-first-pass/` (spec.md, plan.md, research.md, data-model.md, contracts/, quickstart.md)

**Rules for every task:**

- Each code task writes its failing test first, shows it failing, then implements.
- The full gate runs before every commit.
- Every step (each read, edit, commit and push) is asked first.
- Doc and contract text is shown as exact text before it is written.

Format: `- [ ] Txxx [P?] [Story?] Description (files)`. `[P]` means it can run in parallel: different files, no open dependency.

## Phase 1: Setup

- [x] T001 Save the seven research-agent reports of 2026-11-24 verbatim, with a source header per report (`claudedocs/jev-research-2026-11-24-agents.md`)
- [x] T002 [P] Move `retry_after`, `time_until` and `cap_detail` from `src/client/exa.rs` (`:173-229`) to a new `src/client/retry.rs`, and rewire Exa to use it. No behavior change; the existing Exa tests stay green unchanged.

## Phase 2: Foundation (blocks all stories)

- [x] T003 Test config:
  - closed `JevSite::ALL` suffix set;
  - unknown suffix, bad value, `on`, and a mode without a key are each startup errors naming the variable;
  - a blank key counts as unset;
  - both sites default to `off`;
  - `PARALLAX_JEV_MODEL` defaults to `jev-1.14.0` and is never parsed as a mode suffix.

  Files: `src/config.rs` tests.
- [x] T004 Implement config, prefix validation modeled on `src/routing.rs:471-483`, and the new fields in the three Config fixtures (`src/config.rs:652`, `tests/common/mod.rs:20`, `examples/common/mod.rs:55`). Files: `src/config.rs`.
- [x] T005 [P] Test that `classifier_provider` is in `Outcome::ALL`, with the 057 binding test (`src/error.rs:527-565`) extended to the schema and `telemetry.md` lists. Files: `src/error.rs` tests.
- [x] T006 Add `Outcome::ClassifierProvider`, and update `invocation-record.schema.json` and the `telemetry.md` outcome list (exact text shown first). Files: `src/error.rs`, contracts.
- [x] T007 [P] Test the validator and bands:
  - every rejection rule: range, undeclared choice, missing option, sum outside 0.01, id or count mismatch, model mismatch;
  - band edges via `next_up`/`next_down`;
  - `None` and partial responses in both directions.

  Files: `src/jev/policy.rs` tests.
- [x] T008 Implement `src/jev/mod.rs`, `src/jev/questions.rs` (versioned questions, fixed option orders) and `src/jev/policy.rs`.
- [x] T009 [P] Define the `Classifier` trait, the request/answer types and the mock. Test the client against wiremock:
  - request shape with bearer auth;
  - `model_answered` and request id captured;
  - each status: 200, 400 (including `max_tokens_exceeded`), 401, 413, 415, 422, 429, 529, 5xx;
  - exactly one request per call (`expect(1)`), so no retries;
  - 401 disables Jev for the process;
  - the per-attempt deadline includes the body;
  - nothing is sent when no time is left;
  - error bodies are never logged;
  - at most 4 requests in flight.

  Files: `src/traits/classifier.rs`, `src/client/typesafe/tests.rs`.
- [x] T010 Implement the client: `.bearer_auth()`, the named deadlines, a semaphore, constructor base-URL injection, and outcomes mapped per research D4. Files: `src/client/typesafe.rs`, `src/client/mod.rs`.
- [x] T011 [P] Test the redactor:
  - a fixture table covering every pattern family plus exact values of secret environment variables (fixtures built by concatenation);
  - nothing is produced on error;
  - the count is correct;
  - the client rejects unredacted state at the type level, by construction.

  Files: `src/jev/redact.rs` tests.
- [x] T012 Implement `RedactedState` and the redactor. Files: `src/jev/redact.rs`.
- [x] T013 [P] Test the `jev_records` two-way binding: `PRAGMA table_info` against `contracts/jev-record.schema.json`, with enums from `::ALL`. Test writes and reads on `SqliteStorage`, including a single-transaction batch write. Files: `src/storage/jev.rs` tests.
- [x] T014 Implement the table migration, `record_jev` and the batch write on `Storage` (`src/traits/storage.rs`), and the published schema. Files: `src/storage/jev.rs`, `specs/...` contracts.
- [x] T015 [P] Test `jev_cost_usd` on the checkpoint and invocation records, and `DEFAULT_JEV_MODEL` in `every_default_model_has_a_price_row` (`src/telemetry.rs:818-831`). Files: `src/telemetry.rs` tests.
- [x] T016 Implement the price row and the cost fields, plus the record schema additions (exact text shown first).
  - Includes a migration that adds `jev_cost_usd` to existing `checkpoint_records` and `invocation_records` tables when the column is missing.
  - The test opens an old-shape database, migrates it, and writes both record types.

  Files: `src/telemetry.rs`, `src/checkpoint/contract.rs`, `src/storage/sqlite.rs` migration, contracts.

## Phase 3: US1 + US2, turn review shadow (P1)

- [x] T017 [US1] Test that:
  - `review_once` also returns `ReviewRaw` (`contradicts`, `violates`, `capture_kind`);
  - `Candidate` carries its origin (memory pairs from `review.rs:192-203` are `Memory`);
  - `truncate()` returns a truncated flag.

  Files: `src/checkpoint/review.rs` tests.
- [x] T018 [US1] Implement T017's changes. Files: `src/checkpoint/review.rs`.
- [x] T019 [US1] [US2] Test the turn shadow:
  - `CheckpointResult` minus `latency_ms`, and the checkpoint record minus `id` and `latency_ms`, are identical with Jev off and with a Jev that is confident, errors, is rejected, or never returns;
  - the evaluation id is created at the start of `run_turn` and matches the row's `eval_id`;
  - no Jev call when the review doesn't run (continuation, empty final message, no candidates with memory off);
  - a review that fails open still writes a row with a null label;
  - truncation is recorded with no decision;
  - no memory text appears in the request body when recall returned memories;
  - `capture_worthy` is asked only with memory configured;
  - the Jev row and the checkpoint record are written in the same `record()` call.

  Files: `src/checkpoint/run.rs` tests.
- [x] T020 [US1] [US2] Implement `shadow_turn` (`src/jev/shadow.rs`), join it with `review_once` in `run_turn` (`src/checkpoint/run.rs:256-378`), move evaluation-id creation to the start, have `record()` write the row and `jev_cost_usd`, and add the classifier and mode to `CheckpointDeps`.

## Phase 4: US1 + US2, research claim shadow (P1)

- [x] T021 [P] [US1] Test the exclusion table: `2018`, `3.5%`, `v1.14`, `three times`, `March`, `Q3`, `1,000`, Arabic-Indic digits, a weekday, and 063-flagged sources. Excluded claims reach the classifier `times(0)` and are counted. Files: `src/jev/shadow.rs` tests.
- [x] T022 [US1] Implement the exclusion rule. Files: `src/jev/shadow.rs`.
- [x] T023 [US1] Test that:
  - the invocation id is created in `RecordGuard::new` and passed to `InvocationRecord::create`;
  - accumulated Jev rows are written in one transaction by `finish`, and also by `Drop` when a run is cancelled;
  - `jev_cost_usd` equals the sum of the rows.

  Files: `src/server/record.rs` tests, `tests/integration.rs`.
- [x] T024 [US1] Implement the id move and the `JevShadow` accumulator in `RecordDims`. Files: `src/server/record.rs`, `src/telemetry.rs`.
- [x] T025 [US1] [US2] Test the research shadow:
  - `ResearchResult` minus `stats.elapsed_ms`, the `ScriptedClient` prompt log, `stats.tokens`, and usage minus the Jev entry are identical with Jev off and with a Jev that is confident, errors, is rejected, never returns, or is slower than verify (its row records timeout); a unit test shows that with no time left Jev sends nothing and records timeout;
  - per-source questions in the fixed option order;
  - (the 4-request cap is the client's, proven by T009; no pipeline-level test);
  - Jev tokens stay out of the token ceiling;
  - no secret fixture appears in any request body.

  Files: `src/research/pipeline_tests.rs`.
- [x] T026 [US1] [US2] Implement `shadow_claim`, run it beside `verify::run` inside the claim permit, abandoned (recorded as timeout) once verify finishes (`src/research/pipeline.rs:499-520`), and add the classifier and mode to `ResearchDeps` (`:45-75`).

## Phase 5: Telemetry

- [x] T027 [P] Test that:
  - the `parallax.jev.*` attributes (contracts/telemetry-additions.md) appear on the checkpoint and invocation spans with the same values as the records;
  - Jev is absent from `gen_ai.client.token.usage`;
  - the span/record parity test is extended (`tests/integration.rs:2382`).

  Files: `src/observability.rs` tests.
- [x] T028 Implement the attributes, update `telemetry.md` (exact text first), and fix its stale signal list (`:89`). Files: `src/observability.rs`, contracts.

## Phase 6: US3, measurement (P2)

- [x] T029 [P] [US3] Test `metrics.rs`:
  - hand-computed confusion matrices;
  - Wilson and Clopper-Pearson bounds against known values;
  - undefined precision never NaN;
  - an integer threshold grid on observed scores;
  - the band function shared with `policy.rs`;
  - deterministic, disjoint tune and confirm splits, with confirm looks logged;
  - cluster bootstrap;
  - always-no and always-yes baselines;
  - excluded items counted as undecided;
  - `promotion_decision()` returns its reasons.

  Files: `src/jev/metrics.rs` tests.
- [x] T030 [US3] Implement `src/jev/metrics.rs`.
- [x] T031 [US3] Build the harness `examples/jev_backtest.rs`:
  - subcommands `noise`, `self-agreement`, `order`, `base-rate`, `injection` and `report`;
  - synthetic fixtures in `tests/fixtures/jev/` with a provenance-marker test;
  - the `detect-private-key` hook from pre-commit-hooks over that folder (`.pre-commit-config.yaml`; exact change shown first);
  - a Rust test that runs the redactor's own patterns over every fixture file and fails if any match unredacted;
  - `data/jev/` gitignored.

## Phase 7: Wiring and docs

- [x] T032 Test that `--help` and the README list the new variables with the `contracts/config.md` wording (a prefix test from enums, whole words, modeled on `src/main.rs:433`). With no key: zero Jev rows and one startup line. The startup line never contains the key. Files: `src/main.rs` tests, `tests/integration.rs`.
- [x] T033 Wiring and docs:
  - Server construction builds the classifier when the key is set and a site is on, and writes the startup lines (`src/server.rs`).
  - Add `TYPESAFE_API_KEY` to `ENV_REMOVE` (`src/client/claude_cli/invocation.rs:17-24`).
  - Docs, each with exact text shown first:
    - the README environment table, and `--help`;
    - CHANGELOG `[Unreleased]`;
    - `docs/design/SDK_LANDSCAPE.md` (TypeSafe mechanism-tier row);
    - `docs/design/CORRECTIVE_SELECTION.md` (signals-table row);
    - `docs/design/NEW_SERVER_DESIGN.md` §10 (owner table);
    - CLAUDE.md's runtime-config paragraph (the new variables).

## Phase 8: Acceptance and ship

- [x] T034 Write the live example `examples/acceptance_jev.rs`. It asserts the SC-004 checks, including at least 5 turn-review rows and at least 10 research-claim rows. It exits non-zero on failure, and prints rows, outcomes, the excluded share and critical-path time.
- [x] T035 Reviews before merge, as the constitution requires (each brief shown to the user first; findings listed, then one question per finding):
  - an independent reviewer given only the diff;
  - the `design-reviewer` agent, because 064 touches schemas, trait seams and the dependency stack;
  - the `code-reviewer` agent, for all Rust changes.
- [x] T036 Push (account-name check first), PR text shown first, CI.
- [x] T037 The user runs the live acceptance example (command per quickstart §3). Results go to `quickstart.md` §6 (exact text first).
- [x] T038 FR-011 measurements per quickstart §4:
  - the base-rate query is approved first;
  - live calls are made only after the user approves;
  - results are saved to `claudedocs/jev-064-measurements-<date>.md` (exact text first).
  - Done 2026-11-25: noise, self-agreement (research and turn), option order, base rate and the injection back-test, recorded in `claudedocs/jev-064-measurements-2026-11-24.md`.
- [ ] T039 Run the harness `report` twice, saving both outputs with the measurement results:
  - on the T038 saved inputs, to reproduce the measurement tables (SC-006);
  - on the stored records from the T037 live run, to print the §7 tables for both sites (SC-005).
- [x] T040 Turn it on in the user's setup (FR-010), after merge and rebuild:
  - back up `~/.claude.json`;
  - add the two `PARALLAX_JEV_MODE_*=shadow` keys to the Parallax `env` block, changing only those keys;
  - install the project's Stop hook in `.claude/settings.json`, with the exact entry shown first;
  - the user restarts.
  - Done 2026-11-25. The Stop hook went into the user-level `~/.claude/settings.json` instead, by the user's choice when asked, so turn reviews run in every project. The first live shadow row was written 2026-11-25 04:21 UTC.
- [x] T041 Update the CLAUDE.md active-feature block to the shipped state (exact text first).

## Dependencies

- **Phase 1** comes before everything else. T002 has no dependents in 064; it lands first so 065 finds one shared copy of the retry helpers.
- **Phase 2** blocks Phases 3–6. Within Phase 2, T004 comes before T010, T008 and T012 come before T010, and T014 and T016 come before T020 and T024.
- **Phase 3** (T017 → T018 → T019 → T020) and **Phase 4** (T021 → T022, T023 → T024, then T025 → T026) depend on Phase 2. They can proceed in either order.
- **Phase 5** needs T020 and T026.
- **Phase 6** (T029 → T030 → T031) needs T014. T031 needs T020 and T026 to produce real records.
- **Phase 7** needs Phases 3–5.
- **Phase 8**, in order: T034 → T035 → T036 → merge → T037–T041.

## Parallel opportunities

These are test-writing tasks in different files, with no open dependency:

- Within Phase 2: T005, T007, T009, T011, T013 and T015.
- T021 alongside T017.
- T027 and T029 once their prerequisites land.

(Execution is still one tool call per message under the user's rules; "parallel" means there is no ordering constraint.)

## Implementation strategy

- **MVP:** Phases 1–3 give turn-review shadow end to end. Phase 4 adds research.
- **Measurement:** Phases 6 and 8 produce the evidence 065 and 066 need.
- **No Jev answer changes behavior anywhere in 064 (FR-009).**

## Follow-ups (outside 064's scope)

- Flaky test `client::openai_compat::tests::an_omitted_usage_block_records_zeros_and_warns` (`src/client/openai_compat.rs:1266`): failed once in about 15 gate runs on 2026-11-24 with no log output captured; 0 failures in 20 reruns. Suspected cause: tracing's cached callsite interest with a thread-local subscriber under parallel tests (not proven). Needs its own test-first fix.
