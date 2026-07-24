# Tasks: Memory Consolidation and Auto-Capture

**Input**: Design documents from `/specs/017-memory-consolidation/`

**Prerequisites**: plan.md, spec.md (4 user stories, 3 decided clarifications), research.md (D1–D10), data-model.md, contracts/

**Tests**: REQUIRED (Constitution Principle IV). All test tasks run through the seams; the migration test runs against a pre-017 fixture database. Write each story's tests first and watch them fail before implementing.

**Organization**: Tasks grouped by user story. Note the foundational phase carries unusual mechanical blast radius: adding three `Memory` fields touches every `Memory { … }` construction site in the crate's tests.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

No setup tasks — existing crate; additive storage plus the project's first column migration (handled as a foundational task, not setup).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the status dimension, the migration, and active-only retrieval — everything every story assumes.

- [X] T001 Extend `src/memory/mod.rs`: `Status` enum (`Active | Superseded | Merged`, stable strings + parse, `is_active()`); `Memory` gains `status: Status`, `replaced_by: Option<String>`, `last_reinforced_at: DateTime<Utc>` (data-model §1). Update every `Memory { … }` construction site across the crate (ranking/gate/review/run/push/tools/sqlite tests) with `status: Status::Active`, `replaced_by: None`, `last_reinforced_at: created_at` — mechanical, but it is the blast radius task. Round-trip test for the enum strings.
- [X] T002 Storage (research D2/D8, data-model §1/§4) in `src/storage/sqlite.rs` + `src/traits/storage.rs`: pragma-guarded column migration (`PRAGMA table_info(memories)` → `ALTER TABLE ADD COLUMN` for `status`/`replaced_by`/`last_reinforced_at`, backfill `last_reinforced_at = created_at`, loud on failure) run at `connect` after the CREATE block; new `consolidation_records` table; seam methods `record_consolidation`, `captures_in_session(session_id) -> u32`, `update_memory_status(id, status, replaced_by)` (status columns only — content columns are never written, FR-010), `touch_reinforcement(ids)`; inherent `list_consolidations`. Tests: **pre-017 fixture DB** (create via embedded old-schema SQL) migrates with rows byte-identical and correct defaults; migration idempotent on re-connect; consolidation round-trip; captures_in_session counts only `capture_proposed`.
- [X] T003 Active-only retrieval (research D1, FR-011): every path that feeds memories into reasoning filters `status.is_active()` — recall ranking (`src/memory/tools.rs`), push selection (`src/memory/push.rs`), gate constraints (`src/checkpoint/gate.rs` / `run.rs` load site), review/elicit recall (`src/checkpoint/review.rs` `rank_recall` input, `src/modes/elicit.rs` recall seam). One test per path proving a superseded memory is excluded; recall's response continues to expose non-active records only via explicit inspection semantics (verify recall contract: ranked results = active only). Depends on T001.

**Checkpoint**: full suite green with the new fields inert; foundation ready.

---

## Phase 3: User Story 1 — Stale information stops competing (Priority: P1) 🎯 MVP

**Goal**: an update supersedes; context coexists; uncertainty keeps both; superseded stays inspectable with attribution (spec SC-001/SC-002).

### Tests for User Story 1 (REQUIRED — write first, watch fail) ⚠️

- [X] T004 [P] [US1] Failing unit tests in `src/memory/consolidate.rs`: screen selects the best same-kind ACTIVE pair and fires only at `SUPERSEDE_SCREEN_TAU = 0.75`; apply rules — `updates` ⇒ older superseded with `replaced_by`; `context_specific`/`distinct` ⇒ no action; judgment error/timeout ⇒ no action (decline bias); at most one judgment per admission; audit row assembly per action.
- [X] T005 [P] [US1] Failing integration tests in `tests/integration.rs` (wiremock messages endpoint returns the judgment): save fact → save reworded update ⇒ recall and push return only the update, the superseded original is present in the store with status + `replaced_by`, one `supersede` audit row exists; the Berlin/Lisbon scenario (judgment returns `context_specific`) ⇒ both active; judgment returning `distinct` on a screened pair ⇒ both active.

### Implementation for User Story 1

- [X] T006 [US1] Implement `src/memory/consolidate.rs` (research D3): consts (`SUPERSEDE_SCREEN_TAU`, `MERGE_SCREEN_TAU`, `CONSOLIDATION_BUDGET_MS` with provenance docs), pure screen over active same-kind memories, mode registration (flat+closed per `contracts/consolidation.hop.json`), budgeted judgment invocation, pure apply (all four relations + trust guard) returning status updates + audit rows. Depends on T001, T002, T004.
- [X] T007 [US1] Wire the admission path in `src/memory/tools.rs`: after a successful save/admission, run consolidation (screen → judgment → apply via `update_memory_status` → `record_consolidation`), fail-open to keep-both on any error; `observability::emit_consolidation` at the record exit point (`src/observability.rs`, 007 dual-sink, ids/counts only). Depends on T006; makes T005 green.

**Checkpoint**: US1 green — supersession works end-to-end.

---

## Phase 4: User Story 2 — The store stays canonical (Priority: P2)

**Goal**: near-duplicates merge to a byte-identical survivor; the trust guard holds; uncertain pairs never merge (spec SC-003).

### Tests for User Story 2 (REQUIRED — write first, watch fail) ⚠️

- [X] T008 [P] [US2] Failing tests (unit in `src/memory/consolidate.rs`, scenario in `tests/integration.rs`): `same_assertion` at cosine ≥ `MERGE_SCREEN_TAU = 0.90` ⇒ older becomes `merged`, survivor content byte-identical to the new admission, one `merge` audit row; `same_assertion` in the 0.75–0.90 band ⇒ no merge (keep both); **trust guard** — untrusted admission vs trusted existing ⇒ keep both regardless of relation; **promotion direction** (research D7 / FR-007's promotion path) — trusted first-hand admission vs existing untrusted candidate with `same_assertion` ≥ merge tau ⇒ candidate becomes `merged`, the trusted admission survives as canonical; dissimilar pairs never reach the judgment.

### Implementation for User Story 2

- [X] T009 [US2] Close whatever T008 surfaces in `src/memory/consolidate.rs` — the rules land in T006; this task pins the merge band, the byte-identity property, and the guard, and fixes gaps the tests find. Depends on T006–T008.

**Checkpoint**: US1 + US2 green — the store converges without losing distinctions.

---

## Phase 5: User Story 3 — The store grows from ordinary use (Priority: P3)

**Goal**: end-of-turn capture proposes ≤ cap quarantined candidates; uneventful turns propose nothing; candidates never surface via push (spec SC-005).

### Tests for User Story 3 (REQUIRED — write first, watch fail) ⚠️

- [X] T010 [P] [US3] Failing tests in `src/checkpoint/review.rs` + `src/checkpoint/run.rs`: `ReviewOut` carries the four capture fields (register test pins schema + decline-bias prompt wording per `contracts/review.hop.json`); `run_turn` with a capture-worthy hop result + memory configured ⇒ one new memory stored with judgment kind, origin naming the session, `external = true` ⇒ trust `Untrusted`, plus one `capture_proposed` audit row; cap reached (`captures_in_session` ≥ `CAPTURE_SESSION_CAP = 2`) ⇒ `capture_dropped` row, nothing stored; memory off ⇒ capture judgment not requested and not in evaluated kinds; capture storage failure ⇒ turn verdict unaffected (fail-open); `capture_worthy=false` ⇒ nothing. Update every existing mock `ReviewOut` JSON fixture (run.rs, review.rs, integration) with the new fields — the 015-pattern churn.
- [X] T011 [P] [US3] Failing integration test in `tests/integration.rs`: end-to-end `checkpoint_turn` with the wiremock hop returning a capture proposal ⇒ candidate present in the store untrusted with the auto-capture origin; a subsequent `surface` call on a related prompt never surfaces it (quarantine ∘ 016 FR-004); `recall` returns it labeled untrusted; `forget` deletes it.

### Implementation for User Story 3

- [X] T012 [US3] Implement the hop extension (`src/checkpoint/review.rs`: fields + prompt third-judgment section, one-pass substitution rules preserved) and the `run_turn` capture path (`src/checkpoint/run.rs`: cap check via `captures_in_session`, store via memory seams, audit, fail-open, conditional on embedder). Depends on T001, T002, T010.

**Checkpoint**: all growth paths live, quarantine airtight.

---

## Phase 6: User Story 4 — Nothing is ever silently lost or rewritten (Priority: P4)

**Goal**: absolute no-silent-loss — audit completeness, byte-identity, universal forget, ranking-only decay with reinforcement (spec SC-004, FR-005).

### Tests for User Story 4 (REQUIRED — write first, watch fail) ⚠️

- [X] T013 [P] [US4] Failing tests: across every consolidation/capture scenario exactly one audit row per action (extend T004/T008/T010 assertions into a sweep test over `list_consolidations`); post-consolidation content byte-identity for every involved memory (unit, `src/memory/consolidate.rs` + sqlite read-back); `forget` deletes records of every status (`src/memory/tools.rs` tests); ranking reads `last_reinforced_at` (band-edge tests updated, `src/memory/ranking.rs`); recall-return and push-surfacing refresh reinforcement (`touch_reinforcement` called with exactly the returned/surfaced ids; failures warn-logged not raised); raw-cosine floors unaffected by decay (push floor test with an old-but-relevant memory).

### Implementation for User Story 4

- [X] T014 [US4] Implement reinforcement (`src/memory/ranking.rs` recency term reads `last_reinforced_at`; `src/memory/tools.rs` recall + `src/memory/push.rs` fire-and-forget `touch_reinforcement` after response assembly) and close T013 gaps. Depends on T002, T003.

**Checkpoint**: all four stories independently green.

---

## Phase 7: Polish & Cross-Cutting

- [X] T015 [P] Sync the served `checkpoint_turn` tool description (`src/server.rs`) and the canonical contract copy (`specs/006-checkpoint-layer/contracts/checkpoint_turn.tool.json`) to mention the third judgment (capture proposal, quarantined) — the contract-equality integration test enforces the pairing.
- [X] T016 [P] Amend `docs/design/MEMORY_LAYER.md` (research D10): dated amendment — write path shipped (admission-time levers, ranking-only decay, quarantined harness capture, promotion-by-re-admission), traps encoded as rules, three clarify decisions recorded (Constitution I same-change rule).
- [X] T017 [P] Append the feature to `CHANGELOG.md` `## [Unreleased]`.
- [X] T018 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.
- [ ] T019 **Live dogfood** (after merge + rebuild + restart): follow `quickstart.md` — supersession (update wins, original inspectable), Berlin/Lisbon (both stay), merge (byte-identical canonical), capture (candidate proposed, quarantined, never pushed, forget-cleaned), plus a look at `consolidation_records`. Record the result here; clean all seeds. (Live-model properties: whether real judgments classify real pairs correctly; the mechanics are proven by T004–T014.)

---

## Dependencies & Execution Order

- **Phase 2 blocks everything**: T001 → T002 → T003 (T003 needs the field; T002 needs the type).
- **US1 (MVP)**: T004 ∥ T005 (test-first) → T006 → T007.
- **US2**: T008 → T009. Depends on T006 (same module/rules).
- **US3**: T010 ∥ T011 (test-first) → T012. Independent of US1/US2 after Phase 2 (different modules) — parallelizable with them.
- **US4**: T013 → T014. Depends on Phase 2 only; parallelizable with US1–US3 except the shared-file edits in tools.rs/push.rs (coordinate with T007).
- **Polish**: T015–T017 parallel after US3 (T015 needs the final description); T018 after all code; T019 post-merge.

## Parallel Example: after Phase 2

```text
Track A (memory):   T004/T005 → T006 → T007 → T008 → T009
Track B (capture):  T010/T011 → T012
Track C (decay):    T013 → T014   (coordinate tools.rs/push.rs edits with Track A)
```

## Implementation Strategy

MVP is US1: supersession alone fixes the stale-memory-poisons-push failure
and is live-demoable. US2 rides the same module and lands nearly free after
T006. US3 is the largest independent chunk (hop churn included). US4 makes
the guarantees provable. T019 is the only post-merge step. The blast-radius
task (T001) and the first migration (T002) are deliberately front-loaded so
every later task works against the final shapes.
