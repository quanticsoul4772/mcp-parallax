# Tasks: Research deadlines that fit the claude_cli backend

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md)

Tests come first in each story and are seen failing before the code.

## Phase 1: Foundational (blocks both stories)

- [X] T001 Add `backend: crate::config::Backend` to `ResearchDeps` in `src/research/pipeline.rs`, set from `config.backend` in `src/server.rs` and `examples/acceptance_research.rs`, and to `Backend::Anthropic` in the deps builder of `src/research/pipeline_tests.rs` (no behavior change; gate green)

## Phase 2: User Story 1 — Standard and deep research finish on the subscription (P1)

**Independent test**: on `claude_cli`, the default deadline is 120 / 360 / 720 s by depth, and a standard run is not stopped at 300 s.

- [X] T002 [US1] Failing tests in `src/research/settings.rs`: with `Backend::ClaudeCli` and no caller deadline, `validate_params` gives 120 000 / 360 000 / 720 000 ms for quick / standard / deep
- [X] T003 [US1] Failing test in `src/research/pipeline_tests.rs`: on `claude_cli`, a standard run whose test clock reads 300 s elapsed is not stopped early by the deadline
- [X] T004 [US1] Add the `claude_cli` deadline table (`Depth::claude_cli_deadline_ms`) in `src/research/mod.rs` and choose the default by backend in `src/research/settings.rs`; T002–T003 pass; commit

## Phase 3: User Story 2 — Nothing changes anywhere else (P1)

**Independent test**: other backends keep 120 / 240 / 480 s; a caller deadline wins on every backend.

- [X] T005 [US2] Tests in `src/research/settings.rs`: `Backend::Anthropic` and `Backend::OpenAiCompat` give 120 000 / 240 000 / 480 000 ms; a caller `deadline_ms` is used as given on all three backends; below 5000 ms is still rejected on `claude_cli`; commit (these pass on T004's code; a failure is a defect in T004)

## Phase 4: Polish

- [X] T006 `CHANGELOG.md` `[Unreleased]` entry (FR-004)
- [X] T007 Independent review: a fresh subagent given only the diff, told to trace every caller
- [X] T008 Live check (SC-001/002): after the rebuild, one standard and one deep `research` call on `claude_cli`; record the results in `specs/062-claude-cli-research-deadlines/quickstart.md`
- [X] T009 CLAUDE.md active-feature block

## Dependencies

T001 → T002–T004 → T005 → T006–T009. T007 before T008; T009 last.
