---

description: "Task list for 054: decide stops claiming calibrated confidence"
---

# Tasks: Decide Stops Claiming Calibrated Confidence

**Input**: Design documents from `/specs/054-decide-reliability/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/description.md

**Tests**: REQUIRED (Constitution Principle IV). Four new tests, written before the text they check.

**Scope reminder**: words only. If any task appears to need a change to a computed value, a result field, the prompt, or an assertion in an existing test, stop and ask the owner; do not make it.

**Every step that changes a file, commits, pushes or calls a model is asked of the owner first.**

**The gate** is `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test` (run on this machine with `-j 2` on the clippy and test steps, which changes nothing about what is checked). The gate does **not** include the markdown check; that runs only at commit time, so documents get their own check in T022.

## Format: `[ID] [Story] Description`

No task is marked parallel: the work is small and done in order.

---

## Phase 0: Clear the probe out of the build

**Goal**: the temporary probe stops being compiled by every gate run, before any gate result is relied on.

- [ ] T001 [US3] Create `claudedocs/experiments/decide-stability/` and copy the source of `examples/probe_decide_sensitivity.rs` into it as `probe.rs.txt`, with the header changed from "TEMPORARY, not committed" to a statement of what it was used for and a warning that running it makes paid model calls. (FR-010)
- [ ] T002 [US3] Give the owner the command to delete `examples/probe_decide_sensitivity.rs` (deletion is blocked for the agent). After the owner runs it, run the gate and record the output as the starting state. (FR-011)

---

## Phase 1: Tests first (US1, US2)

**Goal**: each check is seen to fail for its stated reason before the text it checks is changed, or is stated plainly as a pin that passes.

- [ ] T003 [US2] In `src/modes/decide.rs` tests, add `the_prompt_the_model_sees_is_unchanged_by_054`. For one fixed input (a decision, two options, a context) it asserts three things equal literals: the output of `build_prompt` with no violation; the output of `build_prompt` with a fixed violation string (this covers the retry wording, which the model also sees and which lives outside `PROMPT_TEMPLATE`); and the registered mode's sanitized schema, which is what the request carries. **Source of the literals**: write the test first with empty literals and run it; it fails and prints the actual values; record that output (this is the test seen failing), then paste the printed values in as the literals and run again. **Expected then: passes.** (FR-005)
- [ ] T004 [US1] In `src/modes/decide.rs` tests, add `decide_description_states_what_was_measured`. It collects every problem into one list and asserts the list is empty: each of the three sentences in `data-model.md` must occur exactly once in `DECIDE_DESCRIPTION`, and the lower-cased description must not contain the stem. Keep the stem's search string on one line. Run it. **Expected: fails, listing four problems** (three missing sentences and the stem). Record the output. (FR-001, FR-002, FR-004)
- [ ] T005 [US1] In `src/modes/decide.rs` tests, add `decide_description_of_confidence_matches_the_contract`, with two separate assertions: (a) the `confidence` description in the derived `DecideResult` schema equals the FR-003 text; (b) it equals the `confidence` description in `specs/013-decide-methodology/contracts/decide.tool.json`. Run it. **Expected: fails on (a)**; (b) is true today. Record the output. (FR-003, FR-004)
- [ ] T006 [US1] In `tests/integration.rs`, add `the_served_decide_description_is_the_registered_one`, modelled on the `grounded_verify` test: the `decide` description in the served tool list equals `DECIDE_DESCRIPTION`. Run it. **Expected: passes today**, because the two copies are identical. Record the output. (FR-004)

**Checkpoint**: four tests exist. T003 was seen failing and now passes; T004 and T005 fail for their stated reasons; T006 passes and will be seen failing in T007.

---

## Phase 2: The description (US1)

**Goal**: callers read the corrected text.

- [ ] T007 [US1] Edit `DECIDE_DESCRIPTION` in `src/modes/decide.rs` to the "After" text in `contracts/description.md`. Run T004's test (**expected: passes**) and T006's test (**expected: fails**, the served copy now differs). Record both outputs. (FR-001, FR-002)
- [ ] T008 [US1] Edit the `decide` tool's description literal in `src/server.rs` to the same text. Run T006's test. **Expected: passes.** (FR-001, FR-002)
- [ ] T009 [US1] Change the `confidence` field's doc comment in `src/modes/decide.rs` to the FR-003 text on one source line. Run T005's test. **Expected: (a) now passes and (b) fails**, because the contract file still has the old text. Record the output. (FR-003)
- [ ] T010 [US1] Change the `confidence` property's `description` in `specs/013-decide-methodology/contracts/decide.tool.json` to the same text; touch nothing else in that file. Run T005's test and `derived_schemas_match_the_contract_file`. **Expected: both pass.** (FR-003, FR-007)
- [ ] T011 [US1] In `specs/013-decide-methodology/contracts/decide.md`, replace the quoted description block whole with the "After" text. Touch nothing else in that file. (FR-007)
- [ ] T012 [US2] Run the gate. **Expected: passes.** Then confirm by reading the diff, file by file, that only the intended lines moved: `src/modes/decide.rs` (the description constant, the one field comment, and added tests; no existing assertion edited); `src/server.rs` (the description literal only, so far); `specs/013-decide-methodology/contracts/decide.tool.json` (exactly one line: the `confidence` description; no property name, type or constraint); `tests/integration.rs` (one added test). Record the gate output and the per-file line counts. (FR-005, SC-003, US2 scenario 2)

**Checkpoint**: US1's four acceptance scenarios and US2's two hold.

---

## Phase 3: Remaining live text (US3)

**Goal**: no current document or comment about `decide` still says the confidence is calibrated.

- [ ] T013 [US3] In `src/modes/decide.rs`, reword the module documentation (two places), the two function doc comments and the two comments in the test module so none contains the stem. Comments only. (FR-007)
- [ ] T014 [US3] In `src/server.rs`, reword the doc comment above the `decide` tool. Leave the `check` tool's description alone. (FR-007)
- [ ] T015 [US3] In `tests/integration.rs`, rename `decide_returns_calibrated_recommendation_and_one_record` to `decide_returns_margin_derived_recommendation_and_one_record` and reword the comment above it. No assertion changes. (FR-007)
- [ ] T016 [US3] In `examples/acceptance_decide.rs`, reword the header comment. No code changes. (FR-007)
- [ ] T017 [US3] In `CLAUDE.md`, change the `decide` sentence in the status paragraph from "calibrates confidence from the score margin" to "derives confidence from the score margin". **This is an instruction file: the owner is shown the exact before and after and asked before the edit.** The Active feature section is not touched here; see T031. (FR-007)
- [ ] T018 [US3] In `src/memory/push.rs`, change "decided at margin 30, the stable band" to "decided at margin 30" (the comma goes with the words). (FR-009)
- [ ] T019 [US3] Run the one-time search for `calibrat`, ignoring case, over the seven live-text locations in `data-model.md`. **Expected: exactly two hits**, T004's search string and the `check` tool's description in `src/server.rs`. Record the output. Then run the gate. (SC-001)

---

## Phase 4: The record (US3)

**Goal**: the design document is corrected and older documents point here.

- [ ] T020 [US3] Draft the dated amendment for `docs/design/NEW_SERVER_DESIGN.md`, to sit after the 2026-07-21/22 amendment, containing the seven statements of FR-006. **The owner is shown its outline and asked before it is written into the file**, since it is new text longer than ten lines in the project's source of truth. (FR-006)
- [ ] T021 [US3] Add the dated pointer line from `data-model.md` to: `specs/013-decide-methodology/spec.md`, `claudedocs/experiments/decide-order-bias/analysis.md`, `specs/016-push-memory/spec.md`, `specs/016-push-memory/research.md`, `specs/017-memory-consolidation/spec.md`. In each file it goes directly under the title line as its own paragraph, with a blank line above and below, so it cannot merge with a neighbouring quote. One line each, nothing else. Then add one entry to `CHANGELOG.md` under Unreleased, Changed: the `decide` description no longer claims calibrated confidence and states what was measured; no behaviour changed. The owner is shown the change-log entry before it is written. (FR-008, FR-009)
- [ ] T022 [US3] Run the markdown check on every document this feature wrote or edited: `pre-commit run markdownlint --files` followed by those files. This is a separate command from the gate and is asked of the owner. **Expected: passes.** Then check SC-005 and SC-006 by hand: each of the seven FR-006 statements can be pointed to in the amendment; each of the five files carries the line.

---

## Phase 5: Evidence (US3)

**Goal**: the measurement can be recomputed from the repository, with nothing private in it.

Inputs, all in the session scratch folder (a temporary folder outside the repository): `probe_out1.jsonl` and `probe_out2.jsonl` (the per-call results) and `probe_plan1.json` (the ten cases, with the private input texts). That folder can be cleared by a reboot; if the files are gone, stop and tell the owner, because the evidence cannot then be committed.

- [ ] T023 [US3] Write the redaction script in the scratch folder (not committed). It reads the three inputs and writes `results.jsonl` and `plan-redacted.json` per `data-model.md` **into the scratch folder**. It stops with an error if any failed call's error text is not the provider's billing error, if any case id is not one of `c00` to `c09`, or if any variant name does not match one of the forms `sameN`, `orderN`, `dropN`, `orderN#N`, `dropN#N`. (FR-010)
- [ ] T024 [US3] Run it. Search both outputs for any string value outside the known vocabulary (the ids, the variant forms, the three methodology names, the word "billing"). **Expected: none.** Show the owner the first lines of both files and the search result, and **ask before anything is copied into the repository**. On a yes, copy the two files into `claudedocs/experiments/decide-stability/`. (FR-010)
- [ ] T025 [US3] Write `claudedocs/experiments/decide-stability/table.py` (standard library only) to print, from those two files alone: the call counts (attempted, completed, failed, used in the table, not used) and every row of the specification's measurement table. Run it; save its output as `table-output.txt`; compare each line with `spec.md`, including the counts 281, 179, 221 and 60. **Any mismatch is a finding: stop and ask.** (FR-010, SC-004)
- [ ] T026 [US3] Write the folder's `README.md` per `data-model.md`, including the dates on the result files. Run the markdown check on it as in T022. (FR-010)

---

## Phase 6: Review and delivery

- [ ] T027 Run the gate. Then independent review of the whole diff by a fresh reviewer given the diff and told to trace every caller and to look for any behaviour change. The owner is shown the brief and asked first. Findings go to the owner one at a time, most severe first.
- [ ] T028 Ask the owner whether one live `decide` call is made before the pull request, stating: the project's note says a model-facing change must pass a live example; no live `decide` example exists; the call is one model request; and its price, computed from the project's price table for the model in use.
- [ ] T029 Commit, each asked. Commit messages name what moved and that no computed value did.
- [ ] T030 Tick every completed task box in this file.
- [ ] T031 `CLAUDE.md`'s Active feature section. **Read `the_active_feature_block_links_and_task_counts_are_real` in `src/main.rs` first**: it takes the first "N tasks closed" figure in that section and applies it to every specification directory the section links, so the wording must be chosen with that test in view. The owner is shown the exact before and after and asked. After the edit, run the gate, re-run the stem search of T019 over `CLAUDE.md`, and run the markdown check. Commit when asked.
- [ ] T032 Push, then open the pull request, each asked separately. The pull request states: nothing computed changed; what pins that (the confidence values, the ranking, the built prompt with and without the retry wording, the sanitized schema) and what rests on review of the diff (the request body outside the prompt and schema); the reviews of the specification, plan and task list and what each changed; what is deferred.

---

## Dependencies

- Phase 0 first, so no gate result depends on the probe compiling.
- Phase 1 before Phase 2: the tests must be seen in their before-state.
- T007 before T008, and T009 before T010: those orders are what make T006 and the second half of T005 fail once for their stated reasons.
- Phase 3 after Phase 2: T019's search assumes the description is already corrected.
- Phase 5 depends on T001 only (the folder exists) and on the scratch files still being present.
- T030 before T031: the Active feature test counts ticked boxes.

## Requirement coverage

| Requirement | Tasks |
|---|---|
| FR-001, FR-002 | T004, T007, T008 |
| FR-003 | T005, T009, T010 |
| FR-004 | T004, T005, T006, T007, T009 |
| FR-005 | T003, T012 |
| FR-006 | T020, T022 |
| FR-007 | T010, T011, T013 to T017, T019 |
| FR-008 | T021 |
| FR-009 | T018, T021 |
| FR-010 | T001, T023 to T026 |
| FR-011 | T002 |
| SC-001 | T019, T031 |
| SC-002 | T004, T006 |
| SC-003 | T012 |
| SC-004 | T025 |
| SC-005, SC-006 | T022 |
| US2 scenario 2 | T012 |
