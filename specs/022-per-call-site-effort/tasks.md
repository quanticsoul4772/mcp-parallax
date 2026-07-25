---

description: "Task list for 022 per-call-site reasoning effort"
---

# Tasks: Per-Call-Site Reasoning Effort

**Input**: Design documents from `/specs/022-per-call-site-effort/`

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md), [contracts/config.md](contracts/config.md)

**Tests**: REQUIRED (Constitution Principle IV).

**Retrofitted.** These tasks describe work already done; they are marked complete
because they are, not because they were planned and then executed. The ordering
below reconstructs what happened rather than what was scheduled. See the spec's
*Process deviation* section.

## Format: `[ID] [P?] [Story] Description`

---

## Phase 1: Setup

- [X] T001 Confirm the gate is green before touching anything: `cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test` (532 at the time)

---

## Phase 2: Foundational (Blocking Prerequisites)

**None.** The three stories share the resolution code but no blocking
prerequisite: US1 needs precedence, US2 needs the absent state to send nothing,
US3 needs validation. All three land in the same function.

---

## Phase 3: User Story 1 - Deep reasoning only where it earns its keep (P1) 🎯 MVP

### Tests for User Story 1 (REQUIRED)

- [X] T002 [P] [US1] Test in `src/routing.rs` that model and effort resolve most-specific-first and **independently** — a site takes its model from a tier and its effort from its own variable (FR-002, SC-002, SC-004)
- [X] T003 [P] [US1] Test in `src/routing.rs` that a per-site effort overrides its tier's, and that a judgment-tier setting never reaches a bulk-tier site (FR-002, SC-003)
- [X] T004 [P] [US1] Test in `src/client/anthropic.rs` that a routed effort reaches `output_config.effort` in its wire spelling alongside `format` (FR-007)

### Implementation for User Story 1

- [X] T005 [US1] Add the `Effort` enum with its five levels, wire spellings, and case-insensitive parse to `src/routing.rs` (FR-001)
- [X] T006 [US1] Add `effort` and `effort_source` to `ResolvedRoute`, and resolve both namespaces independently in `RoutingTable::resolve` (FR-002)
- [X] T007 [US1] Add the optional effort field and `for_model_and_effort` to `src/client/anthropic.rs`, emitting `output_config.effort` when set (FR-007)

**Checkpoint**: An operator can set an effort and it reaches the call site.

---

## Phase 4: User Story 2 - An upgrade changes nothing until asked (P2)

### Tests for User Story 2 (REQUIRED)

- [X] T008 [P] [US2] Test in `src/routing.rs` that an empty namespace leaves every route without an effort and the client count unchanged (FR-003, SC-001)
- [X] T009 [P] [US2] Test in `src/client/anthropic.rs` that with no routed effort the request carries **no** `effort` key and `output_config` has exactly one key (FR-003, SC-001)

### Implementation for User Story 2

- [X] T010 [US2] Make absent a distinct state from `High`: `Option<Effort>`, with the field omitted from the body entirely when `None` (FR-003)
- [X] T011 [US2] Reuse the injected client in `src/server.rs` only when the site is on the default model **and** at no explicit effort — an effort changes the body, so it needs its own client

---

## Phase 5: User Story 3 - A misspelled setting is refused (P3)

### Tests for User Story 3 (REQUIRED)

- [X] T012 [P] [US3] Test in `src/routing.rs` that an unknown suffix and an unparseable level are each startup errors naming the variable, and that the level error lists the accepted values (FR-004, FR-005, SC-006)

### Implementation for User Story 3

- [X] T013 [US3] Add `validate_effort_namespace` as a free function in `src/routing.rs` — a free function rather than inline because `resolve` would otherwise cross the 100-line clippy limit (FR-004, FR-005)

---

## Phase 6: Pooling

- [X] T014 [P] Test in `src/routing.rs` that `distinct_clients` keys on model **and** effort: one model at three effort states yields three clients, and two sites agreeing on both yield one (FR-006, SC-005)
- [X] T015 Key `ClientPool` on `(model, effort)` in `src/client/pool.rs`, so two sites on one model at different efforts do not share a client (FR-006)

---

## Phase 7: Polish & Cross-Cutting Concerns

- [X] T016 [P] Amend `docs/design/SDK_LANDSCAPE.md`: the thinking-suppression mechanism named in 018 research D7 has changed — `thinking: disabled` 400s on newer families; `output_config.effort` is the supported control (Principle I, FR-008)
- [X] T017 [P] Discharge the deferral in `specs/018-model-routing/research.md` D7, recording that it is delivered by 022 and that the mechanism differs from the one deferred
- [X] T018 [P] Add the `[Unreleased]` **Added** entry to `CHANGELOG.md`
- [X] T019 Run the full gate and confirm the count rose: 540 against the 532 baseline
- [X] T020 Prove the wire test catches the defect: remove the effort insertion, confirm `a_routed_effort_reaches_the_request_body` fails, revert
- [X] T021 Write these Spec Kit artifacts, naming the process deviation in the spec and plan rather than omitting it (Principle I)

---

## Dependencies & Execution Order

- Setup → US1 → US2 → US3 → Pooling → Polish. US2's "absent is distinct" shaped
  US1's type (`Option<Effort>` rather than a defaulted `Effort`), so in practice
  they were not independent — recorded here rather than presented as a clean
  three-way split that did not happen.

## Task Summary

| Phase | Tasks | Count |
|---|---|---|
| Setup | T001 | 1 |
| Foundational | — | 0 |
| US1 (P1) | T002–T007 | 6 |
| US2 (P2) | T008–T011 | 4 |
| US3 (P3) | T012–T013 | 2 |
| Pooling | T014–T015 | 2 |
| Polish | T016–T021 | 6 |
| **Total** | | **21** |

Tests: 8 of 21.
