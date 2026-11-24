# Tasks: Screen research pages for prompt injection

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md)

Tests come first in each story and are seen failing before the code.

## Phase 1: Foundational

- [X] T001 Failing unit tests in `src/research/screen.rs` (new module, `pub mod screen` in `src/research/mod.rs`): one string per family is flagged with its name (`instruction_override`, `role_marker`, `hidden_text`); case and extra whitespace are tolerated; each name appears once however often it matches; clean text returns nothing; the two narrowed phrases do not fire on "you are now ready" or "an article about the system prompt"
- [X] T002 Implement `screen(text) -> Vec<&'static str>` per research.md D1–D2; T001 passes; commit

## Phase 2: User Story 1 — The caller sees which sources carry injection attempts (P1)

- [X] T003 [US1] Failing pipeline tests in `src/research/pipeline_tests.rs`: a run with one page containing an injection phrase marks exactly that source and counts 1 flagged page; a run with clean pages has empty lists and a count of 0
- [X] T004 [US1] Add `SourceRecord.injection_patterns` (`src/research/mod.rs`), screen the extracted text in `extract_source` (`src/research/pipeline.rs`), add `SourceRef.injection_patterns` and `Stats.pages_flagged` (`src/research/contract.rs`), copy and count in `src/research/outcome.rs`, and update `specs/004-research-layer/contracts/research.tool.json` (exact text approved first); T003 passes; commit

## Phase 3: User Story 2 — Warn, never block (P1)

- [X] T005 [US2] Pipeline test: a flagged page's text reaches the extraction prompt unchanged and its claims are cited as any other page's; commit (passes on T004's code; a failure is a defect and is asked about)

## Phase 4: Polish

- [X] T006 `CHANGELOG.md` `[Unreleased]` entry (exact text approved first)
- [X] T007 Independent review: a fresh subagent given only the diff, told to trace every caller (brief approved first); findings asked one by one
- [x] T008 Live check (SC-002): after the rebuild, run the 7 acceptance questions through `examples/acceptance_research.rs` (it gains a printed flagged-page count, change shown first), check every flag by hand, record results in `specs/063-research-injection-screen/quickstart.md` (text approved first)
- [ ] T009 CLAUDE.md active-feature block (exact text approved first)

## Dependencies

T001–T002 → T003–T004 → T005 → T006–T009. T007 before T008; T009 last.
