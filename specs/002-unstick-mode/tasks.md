---

description: "Task list for Unstick Mode — Second Corrective on the Registry"
---

# Tasks: Unstick Mode — Second Corrective on the Registry

**Input**: Design documents from `/specs/002-unstick-mode/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: REQUIRED (Constitution Principle IV) — through the trait seams, no
network or disk state. The acceptance example is manual-run live spend.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

*(none — no new dependencies, no config changes; that absence is FR-008)*

## Phase 2: Foundational

- [X] T001 Extract the guarded-invocation wrapper in src/server.rs: a private `run_recorded(tool_id, ct, future)` helper carrying RecordGuard + ct-select + error mapping, with the verify tool refactored onto it — behavior-preserving, gated by the existing suite passing unchanged (SC-006)

## Phase 3: User Story 1 — one concrete next step (P1) 🎯 MVP

- [X] T002 [P] [US1] Unit tests in src/modes/unstick.rs test module (mockall): single model call per invocation (expect times(1)); empty goal/blocked rejected before any call; combined-length bound; restatement of a tried item → validation_failure; empty next_step → validation_failure; happy path returns step+rationale+watch_for with usage
- [X] T003 [P] [US1] Unstick types in src/modes/unstick.rs: UnstickParams (goal, blocked, tried), NextStep (next_step, rationale, watch_for nullable) with schemars derives; test asserting derived schemas match specs/002-unstick-mode/contracts/unstick.tool.json (property sets, required, description) and pass the registry flat+closed assertion
- [X] T004 [US1] Unstick execution in src/modes/unstick.rs: calibrated one-step prompt (placeholders `<<goal>>`/`<<blocked>>`/`<<tried>>` only — structural blindness test like verify's), single pass via ModelClient, local validation + normalized no-restatement check, register() with ensemble_k=1 (depends on T002, T003)
- [X] T005 [US1] Register unstick in src/server.rs (Parallax::new) and add the `#[tool]` unstick method via run_recorded (depends on T001, T004)

## Phase 4: User Story 2 — guarantee parity (P2)

- [X] T006 [P] [US2] Integration tests in tests/integration.rs: catalog lists BOTH tools with schemas matching their contract files; unstick end-to-end round trip (wiremock) with structured_content validating against the contract outputSchema; one record with tool="unstick"
- [X] T007 [P] [US2] Failure-class parity test in tests/integration.rs: induced refusal on unstick surfaces `[refusal]` and records outcome=refusal — same classes as verify
- [X] T008 [US2] SC-006 gate: run the full pre-existing suite and confirm zero modified assertions (the only allowed test-file changes are additions)

## Phase 5: Polish

- [X] T009 [P] Acceptance example examples/acceptance_unstick.rs: 10 varied stuck scenarios; asserts structural validity (SC-002), one-step shape + zero tried-restatements (SC-003), latency < 15s (SC-004); run live and record results in specs/002-unstick-mode/quickstart.md
- [X] T010 [P] Update README.md and CLAUDE.md status: server now serves verify + unstick
- [X] T011 Full gate + code-reviewer and design-reviewer agent passes over the branch diff

## Dependencies

T001 → T005; T002/T003 → T004 → T005 → T006/T007 → T008 → T009/T010 → T011.
T002 ∥ T003; T006 ∥ T007; T009 ∥ T010.
