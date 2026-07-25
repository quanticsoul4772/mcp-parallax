---

description: "Task list for 021 research confidence aggregation"
---

# Tasks: Research Confidence Aggregation

**Input**: Design documents from `/specs/021-research-confidence-aggregation/`

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md), [research.md](research.md), [data-model.md](data-model.md), [contracts/output-contract.md](contracts/output-contract.md), [quickstart.md](quickstart.md)

**Tests**: REQUIRED (Constitution Principle IV). Every story's tests run through the existing `ModelClient` / `Fetcher` / `SearchProvider` / `TimeProvider` seams — no network, no disk, no credentials.

**Organization**: Grouped by user story so each is independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on incomplete work)
- **[Story]**: US1 / US2 / US3, mapping to the spec's prioritised stories
- Exact file paths are given in every task

## Path Conventions

Single Rust project: `src/` and `tests/` at repository root. No new module — see
[plan.md](plan.md) Structure Decision.

---

## Phase 1: Setup

**Purpose**: Establish the measurement baseline the change is judged against.

- [X] T001 Run the full gate and record the pre-change test count as the baseline: `cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`

---

## Phase 2: Foundational (Blocking Prerequisites)

**None.**

This is a deliberate finding, not an omission. The three stories touch overlapping
files but share no blocking prerequisite: US1 needs only that `overall_confidence`
stop multiplying, US2 needs the synthesis hop to key its gaps, US3 needs US2's
published statuses. Inventing a shared "add all the fields first" task would create
exactly the stub state Principle VII and the no-partial-features rule forbid — fields
present but unpopulated. Each story therefore adds and populates its own contract
fields together.

**Checkpoint**: User story work can begin immediately after T001.

---

## Phase 3: User Story 1 - A correct answer reports a confidence that reflects its support (P1) 🎯 MVP

**Goal**: `confidence` stops being annihilated by the coverage term and carries the
support of the claims the answer asserts. The companion `refutation_rate` ships with
it so the caller can tell a run whose evidence held up from one whose evidence fell
apart.

**Independent Test**: Run a research question whose claims all verify as supported and
whose scope produced sub-questions; confirm confidence is non-zero and tracks the
per-claim support.

### Tests for User Story 1 (REQUIRED) ⚠️

> Write these first and confirm they FAIL before implementing.

- [X] T002 [P] [US1] Unit tests for `overall_confidence` without a coverage factor in `src/research/verdict.rs`: mean of the finding confidences; empty findings yield 0.0 (FR-008 / SC-004)
- [X] T003 [P] [US1] Unit tests for `refutation_rate` in `src/research/verdict.rs`: zero verified claims yields 0.0 with no division by zero; all refuted yields 1.0; half refuted yields 0.5
- [X] T004 [P] [US1] Regression test for the observed collapse in `src/research/pipeline_tests.rs`: every sub-question claimed by a gap, all claims supported at ~0.78, asserting confidence > 0 (SC-001). This is the defect — it MUST fail before T007
- [X] T005 [P] [US1] Test in `src/research/pipeline_tests.rs` that two runs with identical surviving support but different refuted proportions report equal confidence and differing refutation rates (SC-006)

### Implementation for User Story 1

- [X] T006 [US1] Remove the coverage parameter from `overall_confidence` in `src/research/verdict.rs`, leaving the mean of the finding confidences (FR-001)
- [X] T007 [US1] Add the pure `refutation_rate(refuted_count, verified_count) -> f32` to `src/research/verdict.rs`, returning 0.0 when no claim was verified (FR-009a)
- [X] T008 [US1] Add `refutation_rate: f32` to `ResearchResult` in `src/research/contract.rs` and redefine the `confidence` doc comment per [contracts/output-contract.md](contracts/output-contract.md)
- [X] T009 [US1] Replace the confidence computation at the assembly site in `src/research/pipeline.rs`: delete the `let settled = plan.sub_questions.len().saturating_sub(gaps.len());` binding at :355 together with the arguments it fed, rewrite the now-false `// Confidence: coverage-weighted (FR-005).` comment above it, and populate `refutation_rate` from the refuted/verified partition already computed there. The binding must go in **this** task, not a later one — left behind it is an unused variable, and `-D warnings` would make the US1 checkpoint unable to pass the mandated gate
- [X] T010 [US1] Amend `specs/004-research-layer/contracts/research.tool.json`: add `refutation_rate` to properties and `required`, and replace the `confidence` description (FR-011)
- [X] T011 [US1] End-to-end test in `tests/integration.rs` asserting a research run returns non-zero confidence with `refutation_rate` present on the wire

**Checkpoint**: The reported confidence no longer collapses. Shippable on its own —
though breadth of resolution is unpublished until US2, which is why US2 follows
immediately rather than being optional.

---

## Phase 4: User Story 2 - A caller can see breadth of resolution separately from support (P2)

**Goal**: Coverage is measured from gaps keyed to the sub-questions they concern, and
published alongside the per-sub-question statuses it is derived from.

**Independent Test**: Run a question whose scope produces sub-questions of which only
some are settled; confirm the settled proportion is reported as its own value,
independent of the claims' support, and reconcilable against the published statuses.

### Tests for User Story 2 (REQUIRED) ⚠️

- [X] T012 [P] [US2] Unit tests for the pure `coverage` in `src/research/verdict.rs` covering the boundary table in [data-model.md](data-model.md): no sub-questions yields 1.0 (FR-007); no target yields 1.0; every sub-question targeted yields 0.0; several gaps targeting one sub-question count it unsettled once (FR-004); an out-of-range target is discarded (FR-006); a `0` target is ignored (FR-009)
- [X] T013 [P] [US2] Test in `src/research/pipeline_tests.rs` that `sub_question_status` carries one entry per scoped sub-question, verbatim and in scope order, with `settled` matching the returned targets (FR-005)
- [X] T014 [P] [US2] Test in `src/research/pipeline_tests.rs` that two runs with identical per-claim support but different settled counts report equal confidence and differing coverage (SC-002)
- [X] T015 [P] [US2] Test in `src/research/pipeline_tests.rs` that reported coverage equals the fraction of published statuses marked settled, for several target shapes (SC-003), and that `confidence * coverage` reproduces the pre-change formula's value for a fixture with known per-claim support and settled count (SC-005) — the property that justified splitting the field rather than blending it
- [X] T016 [P] [US2] Test in `src/research/pipeline_tests.rs` that a `gap_targets` length differing from `gaps` triggers the synthesis retry, and that a second mismatch takes the existing demotion path rather than being silently accepted ([research.md](research.md) D2)
- [X] T017 [P] [US2] Test in `src/research/pipeline_tests.rs` that the grounding-demotion path reports every sub-question unsettled, so a demoted run reports coverage 0.0 with confidence still drawn from its findings
- [X] T018 [P] [US2] Test in `src/research/pipeline_tests.rs` that a run tripping its token budget or deadline still reports defined coverage and a complete `sub_question_status` list — an early stop is when breadth of resolution matters most to the caller, so it is the worst case to leave unpinned

### Implementation for User Story 2

- [X] T019 [US2] Add `gap_targets: Vec<u32>` to `SynthOut` in `src/research/prompts.rs`, keeping the schema flat and closed, and extend `SYNTH_PROMPT_TEMPLATE` to state that entry *i* is the 1-based sub-question gap *i* concerns with 0 meaning none (FR-003)
- [X] T020 [US2] Validate arity and range in `src/research/synthesis.rs`, raising a `ValidationFailure` on a length mismatch into the existing retry, and carry the validated targets out alongside the gaps
- [X] T021 [US2] Populate targets on the grounding-demotion branch of `src/research/synthesis.rs`, keying each appended sub-question gap to its own sub-question and the demotion notice to 0
- [X] T022 [US2] Add the pure `coverage(sub_question_count, targets) -> f32` to `src/research/verdict.rs`, discarding out-of-range and zero targets and counting each sub-question at most once
- [X] T023 [US2] Add `coverage: f32`, `sub_question_status: Vec<SubQuestionStatus>` and the `SubQuestionStatus` type to `src/research/contract.rs`
- [X] T024 [US2] Assemble the statuses and coverage in `src/research/pipeline.rs` at the site the subtraction occupied (removed in T009), deriving both from the validated targets (FR-002)
- [X] T025 [US2] Amend `specs/004-research-layer/contracts/research.tool.json` with `coverage` and `sub_question_status` per [contracts/output-contract.md](contracts/output-contract.md)
- [X] T026 [US2] End-to-end test in `tests/integration.rs` asserting coverage and the statuses reach the wire and reconcile with each other, and that `gaps` is still an array of plain strings (FR-005a) — the shape the clarification decision preserved deliberately, so a regression here would silently undo it

**Checkpoint**: Support and breadth are separately legible, and coverage is auditable
from the output.

---

## Phase 5: User Story 3 - The penalty matches what the caller can see (P3)

**Goal**: A caller can reconcile the reported coverage against the run's own output
even when the gap list is capped.

**Independent Test**: Force a run to produce more gaps than the output publishes and
confirm the reported coverage stays consistent with the published statuses.

### Tests for User Story 3 (REQUIRED) ⚠️

- [X] T027 [P] [US3] Test in `src/research/pipeline_tests.rs` that when the gap list exceeds the published cap, coverage and `sub_question_status` remain mutually consistent and a sub-question may be reported unsettled while its explanatory gap text was dropped

### Implementation for User Story 3

- [X] T028 [US3] Confirm in `src/research/pipeline.rs` that coverage is derived from every returned target while `gaps.truncate` acts only on the published text, and that the two statements can no longer disagree; add the ordering note at the truncation site recording that the original defect was invisibility rather than order ([research.md](research.md) D6)

**Checkpoint**: The figure is auditable in every case, including under the cap.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [X] T029 [P] Amend `docs/design/RESEARCH_PRIMITIVE.md` to describe confidence, coverage and refutation rate as three published figures and how each is derived (Principle I, FR-012)
- [X] T030 [P] Amend `specs/004-research-layer/research.md` and `specs/004-research-layer/data-model.md` where they state the old combined formula, recording that the coverage multiplier was removed and why (Principle I, FR-012)
- [X] T031 [P] Add `CHANGELOG.md` entries under `[Unreleased]`: **Fixed** for the collapse, **Changed** for the `confidence` redefinition, noting the prior value stays recoverable as `confidence * coverage`
- [X] T032 Run the full gate: `cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`, and confirm the test count rose against the T001 baseline
- [X] T033 Prove the T004 regression test catches the defect: temporarily reinstate the coverage multiplier in `src/research/verdict.rs`, confirm the test in `src/research/pipeline_tests.rs` fails, then revert. A test that has never failed is not known to test anything
- [ ] T034 Review against the design corpus with the `design-reviewer` agent and against Rust conventions with `code-reviewer` — required before merge for changes touching the tool surface and schemas (Constitution, Development Workflow)
- [ ] T035 Live-verify after rebuild and restart using the question in [quickstart.md](quickstart.md), confirming non-zero confidence, published coverage, and statuses that reconcile — where the same question previously reported confidence 0

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies
- **Foundational (Phase 2)**: empty — see the phase note
- **US1 (Phase 3)**: after T001. Independent of US2 and US3
- **US2 (Phase 4)**: after T001. Touches `verdict.rs`, `contract.rs` and `pipeline.rs` in the same regions as US1, so **sequence after US1** when one person is working, even though the two are logically independent
- **US3 (Phase 5)**: after US2 — it asserts a property of the statuses US2 publishes
- **Polish (Phase 6)**: after all stories

### Within Each User Story

- Tests are written and failing before implementation
- Pure functions in `verdict.rs` before their call sites in `pipeline.rs`
- The synthesis schema (T019) before anything consuming targets (T020–T024)
- Contract JSON amended alongside the field it documents, never after

### Parallel Opportunities

- T002–T005 in parallel: two distinct test modules, no shared state
- T012–T018 in parallel: all test-only, no implementation dependency between them
- T029–T031 in parallel: three separate documents
- **US1 and US2 are logically independent** and could run in parallel with two people, but they edit adjacent regions of `verdict.rs`, `contract.rs` and `pipeline.rs`; solo, run them in order

## Parallel Example: User Story 1

```bash
# The four US1 tests, written together before any implementation:
Task: "Unit tests for overall_confidence without a coverage factor in src/research/verdict.rs"
Task: "Unit tests for refutation_rate in src/research/verdict.rs"
Task: "Regression test for the observed collapse in src/research/pipeline_tests.rs"
Task: "Refutation-rate discrimination test in src/research/pipeline_tests.rs"
```

## Implementation Strategy

### MVP (User Story 1 only)

1. T001 baseline
2. Phase 3 — the collapse is fixed and confidence discriminates
3. **Stop and validate**: T004 passes, and fails again when the multiplier is reinstated

US1 alone is a defensible increment: it ends the defect. It does leave breadth of
resolution unpublished, which is why the spec ranks US2 immediately after rather than
as optional — shipping US1 alone would relocate no signal, it would drop one.

### Incremental Delivery

1. Setup → baseline recorded
2. US1 → confidence carries support → validate
3. US2 → breadth published separately → validate
4. US3 → auditable under the gap cap → validate
5. Polish → corpus, changelog, gate, live check

### Task Summary

| Phase | Tasks | Count |
|---|---|---|
| Setup | T001 | 1 |
| Foundational | — | 0 |
| US1 (P1) | T002–T011 | 10 |
| US2 (P2) | T012–T026 | 15 |
| US3 (P3) | T027–T028 | 2 |
| Polish | T029–T035 | 7 |
| **Total** | | **35** |

Tests: 13 of 35 (T002–T005, T012–T018, T027, plus the proof-of-catch T033).
