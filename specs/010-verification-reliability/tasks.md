# Tasks: Verification Reliability

**Feature**: `010-verification-reliability` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

Tests included (Constitution IV). US1 and US2 touch different files and are
independent — either can ship alone.

## Phase 1: Setup

No new dependencies, no new module — both fixes live in two existing mode files.
No setup tasks; proceed to the user stories.

## Phase 2: User Story 1 — `verify` confidence is a meaningful signal (P1)

**Goal**: the *k* passes run under distinct critical lenses so confidence becomes graduated, not near-binary.

**Independent test**: the *k* per-pass prompts differ; a 2:1 vote vector aggregates to ≈0.67; the regression set keeps its verdicts.

- [X] T001 [US1] Add a fixed `LENSES` array (literal, counterexample, definitional, evidential, scope — each a directive paragraph) and a `<<lens>>` placeholder in `PROMPT_TEMPLATE`, in `src/modes/verify.rs`. The lens occupies the critical-instruction slot only; `claim`/`context` stay the only subject inputs (stance-blindness, research D3).
- [X] T002 [US1] In `verify::run` (`src/modes/verify.rs`), build a per-pass prompt assigning `LENSES[i % LENSES.len()]` to pass *i*; leave `aggregate_core` and the quorum/confidence math unchanged (only the inputs diversify).
- [X] T003 [P] [US1] Unit tests in `src/modes/verify.rs`: the *k* built prompts are pairwise distinct (lenses injected); `build_prompt` still contains only the lens + claim + context slots (no stance); and the aggregation vote-vector tests (2:1 → ≈0.67, even-`k` 2:2 tie → refuted, sub-quorum → dominant failure) have direct coverage (FR-004 / SC-005). No live model.
- [X] T004 [US1] Verify the regression set (FR-003) with the mocked model: a clear-error claim still converges to `refuted` with the named finding; a clearly-true claim returns `supported` with no manufactured findings; an authority-vouching context still refutes on the merits — in `src/modes/verify.rs` tests or `tests/integration.rs`.

**Checkpoint**: US1's mechanism is in and offline-tested. SC-001 (real claims scatter → graduated confidence) is confirmed live in Polish (a mock cannot disagree with itself).

## Phase 3: User Story 2 — `grounded_verify` does not confidently judge a computable property (P2)

**Goal**: a computable claim, or one whose decisive evidence the passes self-report as missing, returns `inconclusive` — never a confident wrong verdict.

**Independent test**: the `server.rs > 1000 lines` reproduction returns `inconclusive` (not `refuted` at 1.0); the judgment path is unchanged.

- [X] T005 [US2] Add `needs_computation: bool` to the `GroundedPass` schema and instruct the prompt to set it when the claim's truth hinges on an exact computation of the source the pass cannot perform by reading (a precise count/measure), in `src/modes/grounded_verify.rs`. Keep the pass schema flat + closed.
- [X] T006 [US2] Add a server-assembled `GroundedVerdictKind { Supported, Refuted, Inconclusive }` and a `reason` field on `GroundedVerdict`; the per-pass `VerdictKind` (shared with `verify`) stays `{ Supported, Refuted }`, so `verify` is untouched (FR-009) — in `src/modes/grounded_verify.rs`.
- [X] T007 [US2] In grounded aggregation (`src/modes/grounded_verify.rs`), after the existing pass aggregation: if a majority of completed passes set `needs_computation` → `Inconclusive` (reason: computable property — route to `check`); else the majority `Supported`/`Refuted` (008 behavior). `needs_computation` is the **only** abstain trigger; a non-empty aggregated `missing_evidence` is carried through as the advisory completeness signal and MUST NOT force `Inconclusive` (no over-abstention).
- [X] T008 [P] [US2] Unit tests in `src/modes/grounded_verify.rs`: majority `needs_computation` → `inconclusive` (route reason); **no over-abstention** — a confident `supported`/`refuted` verdict that lists non-empty advisory `missing_evidence` (no pass set `needs_computation`) stays `supported`/`refuted` and carries the missing_evidence through; no flag + empty missing → `supported`/`refuted` unchanged; the pass schema validates flat + closed with the new boolean.
- [X] T009 [P] [US2] Integration tests in `tests/integration.rs` (010 block): the `src/server.rs > 1000 lines` reproduction (mock passes return `needs_computation=true`) → the tool returns `inconclusive`, never `refuted` at 1.0 (FR-008 / SC-003); a genuine judgment claim (no flag) returns `supported`/`refuted` unchanged (FR-007).

**Checkpoint**: both stories complete and offline-tested.

## Phase 4: Polish & Cross-Cutting Concerns

- [X] T010 [P] Docs: note the `inconclusive` verdict in the `grounded_verify` README Tools row and `CLAUDE.md`; note that `verify` runs diverse lenses.
- [X] T011 [P] Extend `examples/acceptance_grounded_verify.rs` with the `inconclusive` reproduction (server.rs line-count → inconclusive), and add the verify regression assertions.
- [X] T012 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.
- [ ] T013 **Live SC-001 dogfood** (after merge + rebuild + restart, the one step needing the running binary): re-run the borderline `verify` battery that previously returned 0/8 graduated and confirm a confidence spread; confirm the `grounded_verify` reproduction returns `inconclusive`. Record the result. (Not a `cargo test` — a live model property.)

## Dependencies & Execution Order

- **US1 (T001–T004)** and **US2 (T005–T009)** are independent (different files); either order, or in parallel.
- **Polish (T010–T012)** after both stories; **T013** after merge + rebuild (the only non-offline step).

## Parallel Execution Examples

- Within US1: T003 (unit) parallel with T004 (regression) once T001/T002 land.
- Within US2: T008 and T009 parallel once T005–T007 land.
- US1 and US2 whole phases can proceed in parallel (disjoint files).
- Polish: T010, T011 parallel; T012 last; T013 post-merge.

## Implementation Strategy

Either story is independently shippable. US1 (the lens diversity) is the higher-value
P1 — it restores the corpus's "diverse lenses" mandate and makes confidence a real
signal. US2 (the `inconclusive` verdict) removes the confidently-wrong-verdict class.
Everything except **T013** is implementable and testable **offline** (wiremock-mocked
model); T013 is the single live confirmation of SC-001, which a mock cannot produce.
