# Tasks: Preference Enforcement at the Checkpoint

**Input**: Design documents from `/specs/015-preference-enforcement/`

**Prerequisites**: plan.md, spec.md (3 user stories), research.md (D1–D10), data-model.md, contracts/

**Tests**: REQUIRED (Constitution Principle IV). All test tasks run through the trait seams (`MockModelClient`/`MockEmbedder`/`MockStorage`/`MockTrajectoryReader`/`MockTimeProvider`) — no network, no disk. Write each story's tests first and watch them fail before implementing.

**Organization**: Tasks grouped by user story; each story is an independently testable increment.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

No setup tasks — this feature extends the existing crate: no new dependencies, no new configuration, no storage migration (plan.md Technical Context).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the shared types every story builds on.

- [X] T001 Add `SignalKind::PreferenceViolation` (wire form `"preference_violation"`) to `src/checkpoint/mod.rs` — `as_str` arm + serde snake_case; extend the existing wire-enum serialization test to pin the new value.
- [X] T002 Create `src/checkpoint/preference.rs` (pure, per research D2/D4 and data-model §2/§4): `PreferenceCandidate { memory_id, content, trust, score }`; `mine_preference_candidates(recall) -> Vec<PreferenceCandidate>` filtering `score >= REVIEW_RECALL_FLOOR && gate::is_constraint(memory)` with a most-relevant-first cap; `violation_signal(candidate, basis) -> (Signal, String)` building the cooldown identity from `memory_id` and the fixed flag template (data-model §4). Co-located unit tests: untrusted excluded, sub-floor excluded, cap order, identity stable across differing basis wording, template quotes content + id + trust. Register the module in `src/checkpoint/mod.rs`. Depends on T001.
- [X] T003 [P] Extend `ReviewOut` in `src/checkpoint/review.rs` with `violates` / `violated_preference` / `violation_basis` (data-model §3) and the prompt template with a decline-biased preference section (placeholder `<<preferences>>` alongside `<<candidates>>`, one-pass substitution per the 005 template-injection rule); registration must still pass the flat+closed boot invariant — extend the register test to assert the three new properties are in the schema.

**Checkpoint**: `cargo test checkpoint` green; foundation ready.

---

## Phase 3: User Story 1 — Violation flagged at end of turn (Priority: P1) 🎯 MVP

**Goal**: seeded trusted preference + violating turn ⇒ one flag quoting the preference verbatim with memory id + trust provenance (spec SC-001).

**Independent Test**: `cargo test` scenarios below pass on mock seams alone; live half deferred to T016.

### Tests for User Story 1 (REQUIRED — write first, watch fail) ⚠️

- [X] T004 [P] [US1] Failing unit tests in `src/checkpoint/review.rs`: with mocked `ModelClient` returning `violates=true` + an echo of one of two mined preference candidates, `review_once` (extended signature) returns a `preference_violation` signal whose identity derives from the *matched candidate's* memory id (echo map-back, research D5) and a flag message from the fixed template — never the model's wording; with `violates=false` and `contradicts=false`, returns no flag.
- [X] T005 [P] [US1] Failing scenario tests in `src/checkpoint/run.rs` tests AND `tests/integration.rs` (plan tree: integration carries the end-to-end violation-flag scenario): `run_turn` with embedder + one seeded trusted `Fact` memory + a plainly violating `final_message` ⇒ `Verdict::Flag` — never `Hold` and no `hookSpecificOutput` mapping (FR-003) — message contains the memory content and id, `signals_fired[0].kind == PreferenceViolation`, `review_ran`, `cost_usd > 0`, with the mock client pinned to `expect_complete().times(1)` (FR-010: one hop, both judgments); and the both-fire case (contradiction + violation mocked true together) ⇒ one flag whose message contains both templates, two signals, two delivered keys (research D6), still `times(1)`.

### Implementation for User Story 1

- [X] T006 [US1] Implement the hop extension in `src/checkpoint/review.rs`: build the numbered preference listing (content verbatim) + bounded final-message excerpt (fixed char-cap constant) + existing window activity summary (research D3); substitute `<<preferences>>`; parse/validate the extended `ReviewOut`; map the echo back to the mined candidate by best overlap and assemble the violation flag via `preference::violation_signal`. Depends on T002, T003, T004.
- [X] T007 [US1] Wire `src/checkpoint/run.rs::run_turn`: mine preference candidates from the existing `turn_recall` output when the embedder is present; pass them to the extended hop; deliver contradiction/violation/both per research D6 (one message, all signals, per-key cooldown via existing `unsuppressed`). Depends on T005, T006.

**Checkpoint**: US1 tests green — the MVP enforcement path works end-to-end on mocks.

---

## Phase 4: User Story 2 — Enforcement never degrades a session (Priority: P2)

**Goal**: memory-off byte-identical behavior; fail-open on recall failure; continuation skip; untrusted never fires; compliant turns silent (spec SC-002/SC-003/SC-004).

**Independent Test**: the scenarios below pass regardless of US3's tasks.

### Tests for User Story 2 (REQUIRED — write first, watch fail) ⚠️

- [X] T008 [P] [US2] Failing tests in `src/checkpoint/run.rs` tests + `tests/integration.rs`: (a) embedder `None` ⇒ `signals_evaluated == [SelfContradiction]` exactly, no preference mining, verdicts identical to pre-015 expectations (SC-003); (b) embedder errors mid-recall ⇒ `fail_open` silence, no error to caller, record still written (SC-004); (c) `continuation=true` ⇒ no evaluation at all (unchanged assert extended to enforcement); (d) seeded `Untrusted` memory + violating message ⇒ silence, nothing fired (FR-005); (e) mocked hop `violates=false` on a compliant message ⇒ silence with `review_ran` (SC-002).

### Implementation for User Story 2

- [X] T009 [US2] Implement conditional `signals_evaluated` in `src/checkpoint/run.rs` (`PreferenceViolation` listed iff embedder present — research D7); confirm the existing `recover`/fail-open path covers the extended hop and mining (no new error surface); add the final-message prompt-cap constant where T006 consumes it. Depends on T007, T008.

**Checkpoint**: US1 + US2 green — enforcement provably cannot hurt an unconfigured or failing session.

---

## Phase 5: User Story 3 — Every enforcement evaluation is auditable (Priority: P3)

**Goal**: one audit row per evaluation; enforcement-evaluated and what-fired readable from the row; violation cooldown keyed by memory id (spec SC-005).

**Independent Test**: record-shape assertions below, independent of live telemetry.

### Tests for User Story 3 (REQUIRED — write first, watch fail) ⚠️

- [X] T010 [P] [US3] Failing tests in `src/checkpoint/run.rs` tests: across flag / silence / fail-open / memory-off scenarios, `record_checkpoint` is called exactly once each, with `signals_evaluated` containing `preference_violation` iff memory was configured, fired evidence naming the memory id; and the cooldown scenario — same memory's violation delivered within `COOLDOWN_WINDOW_MS` ⇒ suppressed record (`suppressed=true`, empty `delivered_keys`), a *different* memory's violation ⇒ delivered (per-key independence).

### Implementation for User Story 3

- [X] T011 [US3] Satisfy T010 in `src/checkpoint/run.rs` — expected to be verification-only (the record path and `emit_checkpoint` already serialize kinds by value; data-model §6/§7): fix anything T010 surfaces, and grep the storage/observability read paths for kind-filtering that would need the new variant (quickstart rollback note). Depends on T007, T010.

**Checkpoint**: all three stories independently green.

---

## Phase 6: Polish & Cross-Cutting

- [X] T012 [P] Update the `checkpoint_turn` tool description in `src/server.rs` to the 015 contract wording (`contracts/checkpoint_turn.tool.json`) so the served surface matches the contract.
- [X] T013 [P] Amend `docs/design/PREFERENCE_ELICITATION.md` (research D10): dated 2026-07-21 amendment — enforce half shipped at the end-of-turn checkpoint, flag-and-revise authority chosen, hold tier deferred pending SC-005 audit data (Constitution I same-change rule).
- [X] T014 [P] Append the feature to `CHANGELOG.md` `## [Unreleased]` (Keep a Changelog 1.1.0, per the CLAUDE.md convention).
- [X] T015 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.
- [X] T016 **Live SC-001/SC-002/SC-005 dogfood** (after merge + rebuild + restart, the one step needing the running binary + memory + hooks): follow `quickstart.md` — seed the "delve" preference, produce a violating turn (expect the flag with memory id, revise in the forced continuation), a compliant turn (expect silence), inspect the two `checkpoint_records` rows, `forget` the seed. Record the result here. (Not a `cargo test` — whether the live model's judgment fires on a real violation and declines on a compliant turn is a live-model property; the mechanics are proven on mocks by T004–T011.)
  - **Result (2026-07-21, live, `claude-opus-4-8` hop + `voyage-4` recall, rebuilt post-#44 binary):** PASS with one named finding. Seeded "final messages must never contain the word 'delve'" (fact, first-hand, id `e7cf7aac`), direct `checkpoint_turn` calls against a scratch transcript. (a) SC-001 — an on-topic violating final message (about final-message wording, containing "delve") → **flag**: preference quoted verbatim, memory id + `first_hand` provenance named, basis "The final message contains the word 'delve', which the stored preference forbids", `decision: block` hook mapping present, delivered key `preference_violation:07f6a55d30ff5efe`, hop cost $0.0108. (b) SC-002 — same topic without the word → hop ran (`review_ran`, $0.0097) and **declined** to silence: the decline bias holds live. (c) SC-005 — three `checkpoint_records` rows, one per evaluation, `signals_evaluated` listing `preference_violation` in all three, fired/delivered readable. Seed `forget`-cleaned. **Finding (recall-floor topicality):** the *first* violating attempt — a final message about retry-loop code that merely used the word "delve" — produced silence with `review_ran: 0`: recall ranks by topic (cosine ≥ 0.45), and a wording-ban preference is not topically near an arbitrary message that violates it. Wording bans reach the hop only when the message is topically close to the preference; lexical candidate mining (substring/token match alongside cosine) is the natural follow-up if live precision data shows this class mattering.

---

## Dependencies & Execution Order

- **Phase 2 blocks everything**: T001 → T002; T003 parallel to T002.
- **US1 (MVP)**: T004 ∥ T005 (test-first) → T006 → T007.
- **US2**: T008 (test-first) → T009. Depends on US1's wiring (T007) because it asserts the *absence* of US1 behavior under degraded configs — still independently testable via its own scenarios.
- **US3**: T010 (test-first) → T011. Depends on T007 only.
- **Polish**: T012–T014 parallel, any time after US1; T015 after all code tasks; T016 after merge.

## Parallel Example: User Story 1

```text
# After Phase 2, launch test-writing in parallel:
T004: hop-extension unit tests in src/checkpoint/review.rs
T005: run_turn scenario tests in src/checkpoint/run.rs
# Then sequentially: T006 (review.rs) → T007 (run.rs)
```

## Implementation Strategy

MVP is US1 alone: after T007 the enforcement loop works end-to-end on mocks and
is demonstrable. US2 hardens the degraded configurations, US3 pins the audit
surface; each checkpoint is a stable stopping point. T016 is the only task that
cannot run pre-merge (needs the rebuilt binary in a live hooked session), same
pattern as 012 T013 / 013 T010 / 014 T012.
