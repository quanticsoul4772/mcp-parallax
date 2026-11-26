# Tasks: decide per-option fields

**Input**: Design documents from `specs/056-decide-per-option-fields/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/decide-pass.md, quickstart.md

**Tests**: required. Test-first for every behaviour change: each test is written and run to fail before the code that makes it pass.

**The gate** is `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests` (run with `-j 2` on this machine), plus `pre-commit run markdownlint` on changed markdown.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependency on an unfinished task)
- **[Story]**: US1, US2, US3 from spec.md

## Phase 1: Setup

- [X] T001 Run the full gate on branch `056-decide-per-option-fields` before any code change and record the passing test counts as the baseline in `specs/056-decide-per-option-fields/quickstart.md`

## Phase 2: Foundational (blocks every story)

- [X] T002 Write a failing unit test in `src/modes/mod.rs` tests: a registry method that registers one mode with one output schema per count in a given range runs `assert_flat` on every variant (an illegal variant fails registration) and returns the variant for a requested count, with none for a count outside the range
- [X] T003 Implement that registry method in `src/modes/mod.rs` (per-count `output_schema` and `sanitized_schema`, each gated by `assert_flat` at registration; existing `register` unchanged for other modes); T002 passes

## Phase 3: User Story 1 — no list-length mismatch is possible (P1, MVP)

**Goal**: `decide` asks for one required score and rationale field per option and parses by position.

**Independent test**: with a mocked client, every count 2..=10 sends a format with exactly those fields and no score or rationale arrays, and a well-formed answer yields one assessment per option in input order.

- [X] T004 [US1] Write failing tests in `src/modes/decide.rs` tests: (a) for each count 2..=10 the registered format has required `option_<i>_score` (integer) and `option_<i>_rationale` (string) for i in 1..=N, `methodology`, `deciding_factors`, and no `option_scores`/`option_rationales`; (b) a call with N options sends the format for N (asserted through `MockModelClient`'s received schema), and one call through the OpenAI-compatible client's test double sends that same per-count format; (c) a well-formed per-field answer yields N assessments in input order
- [X] T005 [US1] Write a failing test in `src/modes/decide.rs` tests: 11 options is rejected with `InvalidInput` naming the count and the limit 10 before any model call; exactly 10 is accepted
- [X] T006 [US1] Implement the per-count pass schema builder and register decide's formats for counts 2..=10 via the T003 method in `src/modes/decide.rs`; replace `DecidePass`'s arrays with position-keyed parsing from the completion value
- [X] T007 [US1] Implement the 10-option cap in `check_input` in `src/modes/decide.rs` (`DECIDE_OPTIONS_MAX = 10`)
- [X] T008 [US1] Update `PROMPT_TEMPLATE` rules 3–4 and the violation clause in `src/modes/decide.rs` to name `option_<i>_score` / `option_<i>_rationale` and drop the now-structural "exactly one score and one rationale per option"; update the pinned-schema test (`PIN_SCHEMA`) to the new per-count format
- [X] T009 [US1] Remove the arity check and its message from `assemble` in `src/modes/decide.rs`, and change these existing tests:
  - `assessment()` helper: rewritten to build the per-field answer; every test that calls it follows unchanged in meaning
  - `arity_mismatch_after_retry_is_a_failed_pass`: rewritten as "a missing per-option field is rejected by the schema validator" (the arity case can no longer be expressed)
  - `a_malformed_assessment_triggers_one_retry_then_succeeds`: rewritten to use an out-of-range score as the malformed first answer instead of an arity mismatch
  - `the_retry_prompt_carries_the_violation_verbatim`: rewritten with an out-of-range-score violation
  - the pinned retry-prompt test (`PIN_PROMPT_RETRY` and its arity violation string): rewritten with a score violation and the new prompt wording
  - `mode_registers_flat_closed_with_single_pass`: rewritten to check the per-count formats instead of `option_scores` items
  - `margin_to_confidence_map`: rewritten to build the new pass shape instead of `option_scores`/`option_rationales`
- [X] T010 [US1] Run the decide tests and the full gate; T004 and T005 pass

## Phase 4: User Story 2 — callers see no change (P2)

**Goal**: identical results and records for identical scores.

**Independent test**: the existing result, confidence, runner-up and record tests pass, rewritten only for the new answer shape.

- [X] T011 [US2] Write a test in `src/modes/decide.rs` tests that feeds the same scores and rationales through the new shape and asserts the `DecideResult` (recommended, runner_up, runner_up_reason, confidence, methodology, deciding_factors, assessments) equals the values the pre-change tests expected
- [X] T012 [US2] Confirm a score outside 0–100 and empty `deciding_factors` still feed the single retry (existing tests, adapted to the new shape) in `src/modes/decide.rs`
- [X] T013a [US2] Change "two or more options" to "two to ten options" in `DECIDE_DESCRIPTION` and note the 2–10 range on the `options` field doc, both in `src/modes/decide.rs`
- [X] T013b [P] [US2] Make the same description change in the `#[tool]` literal in `src/server.rs` and in `specs/013-decide-methodology/contracts/decide.tool.json`; update CLAUDE.md's decide description ("≥2 options" to "2 to 10 options") and README if it states the range; after T013a and T013b the existing served-equals-contract and contract-equals-constant tests must pass
- [X] T014 [US2] Run the decide integration tests in `tests/integration.rs` and the full gate

## Phase 5: User Story 3 — the design record explains the change (P3)

- [X] T015 [P] [US3] Add a dated 2026-11-20 amendment to `specs/013-decide-methodology/research.md` (D1 reversed: per-option fields, the 28 arity failures, the flat gate still holds per count)
- [X] T016 [P] [US3] Add a dated 2026-11-20 amendment to `specs/013-decide-methodology/data-model.md` (the pass format per count; the 2–10 option range)

## Phase 6: Polish & cross-cutting

- [X] T017 Add CHANGELOG entries in `CHANGELOG.md` under `[Unreleased]`: Fixed (arity mismatches impossible) and Changed (10-option cap)
- [X] T018 Independent review: a fresh general-purpose subagent given only the diff, told to trace every caller
- [X] T019 Live check (paid): after the rebuild, call `decide` through the running server with 2, 4 and 6 options; record counts, results and cost in `specs/056-decide-per-option-fields/quickstart.md`
- [X] T020 Update the CLAUDE.md Spec Kit block when the feature closes

## Dependencies

- T001 → T002 → T003 → US1 (T004–T010) → US2 (T011–T014) → US3 (T015–T016) → Polish (T017–T020)
- US3 and T013b touch files no other US2 task edits and can run beside other US2 work once US1 is done; T013a edits `src/modes/decide.rs` with T011 and T012, so it runs in sequence with them.

## Parallel opportunities

- T013b, T015 and T016 edit different files.

## Implementation strategy

MVP is US1: once T010 passes, arity mismatches are impossible. US2 proves nothing else moved; US3 and Polish complete the record, review and live check.
