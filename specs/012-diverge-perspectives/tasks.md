# Tasks: Diverge — Independent Perspectives

**Feature**: `012-diverge-perspectives` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

Tests included (Constitution IV). The mechanism is offline-testable; **SC-001 / SC-003**
(real problems scatter into ≥3 distinct framings; a stated stance does not narrow the
set) are **live-model** properties confirmed by a single live dogfood (T013), like
`verify`'s SC-001 (010). One new mode file plus server wiring.

## Phase 1: Setup

No new dependencies, no new module beyond the mode file. No setup tasks.

## Phase 2: Foundational (blocking both stories)

The mode, its passes, and the catalog wiring — prerequisites for US1 and US2.

- [ ] T001 Create `src/modes/diverge.rs`: `DIVERGE_ID` (`"diverge"`), `DIVERGE_DESCRIPTION` (the routing text from contracts/diverge.md), a fixed `LENSES: &[Lens]` array (invert / actor / horizon / assumption / class — each a generative directive paragraph), and `PROMPT_TEMPLATE` with `<<lens>>`, `<<problem>>`, `<<context>>` slots (the only subject inputs — stance-blind). Define `DivergeParams { problem: String, context: Option<String> }`, the flat+closed `DivergePass { framing: String, implication: String }`, and a `register(registry, ensemble_k)` that enforces flat+closed. Add `pub mod diverge;` to `src/modes/mod.rs`.
- [ ] T002 In `src/modes/diverge.rs`, add `check_input` (reject empty/whitespace or oversize `problem` before any model call, FR-008) and `run(client, mode, params, max_claim_chars)`: build a per-pass prompt assigning `LENSES[i % LENSES.len()]` to pass *i* (reusing verify's lensing approach), run the `k` passes via `futures::future::join_all`, and call `one_pass` (constrained completion → validate → typed `DivergePass`; an empty/whitespace `framing` is a failed pass).
- [ ] T003 In `src/modes/diverge.rs`, add the **collect** aggregation: label each completed pass with its lens (`LENSES[i % len].name`) into a `Perspective { lens, framing, implication }`, sum token usage, and assemble `DivergeResult { perspectives, passes }`. If **zero** passes complete, return the dominant failure class (reuse `verify::dominant_failure` or an equivalent). No verdict, no quorum (research D5). (Dedup is added in US1/T005.)
- [ ] T004 Register and expose the tool in `src/server.rs`: `diverge::register(&mut registry, config.verify_ensemble_k)?` in the catalog build (always on, no gate), the `#[tool(name = "diverge", ...)]` entry, and `diverge_with_ct` wired through `run_recorded` (one record per call), returning `Json<DivergeResult>`. Mirror the `unstick`/`verify` wiring.

## Phase 3: User Story 1 — distinct framings, deduplicated (P1)

**Goal**: the `k` passes run under distinct generative lenses and the server returns a deterministically deduplicated set of materially distinct, lens-labeled framings.

**Independent test**: the `k` built prompts are pairwise distinct (lenses injected); a constructed set with two near-identical framings collapses to one; distinct framings are kept; each perspective is lens-labeled.

- [ ] T005 [US1] In `src/modes/diverge.rs`, add the deterministic dedup (research D4): a `DEDUP_THRESHOLD` constant (`0.8`), a `normalize(framing) -> token set` (lowercase, strip punctuation, collapse whitespace), a `jaccard(a, b) -> f64`, and `dedup(perspectives) -> Vec<Perspective>` that keeps the lower-index perspective and drops later ones with Jaccard ≥ threshold, in pass order. Wire `dedup` into the T003 aggregation.
- [ ] T006 [P] [US1] Unit tests in `src/modes/diverge.rs`: the `k` built prompts are pairwise distinct (lenses injected); `LENSES` is non-empty with unique names and cycles at `k > len`; `dedup` collapses two near-identical framings to one (keeping the earlier lens) and keeps two distinct framings; `jaccard`/`normalize` behave on constructed inputs; the per-pass schema registers flat + closed (`additionalProperties:false`, two string fields).
- [ ] T007 [US1] Tests (mocked model) in `src/modes/diverge.rs` or `tests/integration.rs`: a `diverge` run returns one `Perspective` per completed pass, each labeled with its assigned lens; an empty-`framing` pass is dropped (not a perspective); zero completed passes returns the dominant failure.

**Checkpoint**: US1's mechanism is in and offline-tested. SC-001 (real problems scatter to ≥3 distinct framings) is confirmed live in Polish (a mock cannot diverge).

## Phase 4: User Story 2 — stance-blind, like the family (P1)

**Goal**: a pass sees only the problem and optional neutral context — never the caller's preferred framing, stance, or history.

**Independent test**: the prompt template exposes only `<<lens>>` / `<<problem>>` / `<<context>>`; there is no slot for stance/history/identity.

- [ ] T008 [US2] Unit test in `src/modes/diverge.rs`: `build_prompt` substitutes only lens + problem + context byte-for-byte, and `PROMPT_TEMPLATE` contains exactly those three placeholders (no stance/identity slot) — stance-blindness is structural (mirrors verify's blindness test).
- [ ] T009 [P] [US2] Integration test in `tests/integration.rs` (012 block): a `diverge` call whose `context` asserts a preferred framing still returns lens-labeled perspectives and writes exactly one invocation record; the caller's stated preference reaches a pass only as `context` (no extra slot). (The SC-003 "does not narrow" property is the live dogfood, T013.)

**Checkpoint**: both stories complete and offline-tested.

## Phase 5: Polish & Cross-Cutting Concerns

- [ ] T010 [P] Integration tests in `tests/integration.rs` (012 block): `diverge` is always in the catalog (no gate); a mocked run returns a deduplicated, lens-labeled `perspectives` set with `passes`, and exactly one record; the output carries no `verdict`/`confidence` field (FR-007).
- [ ] T011 [P] Docs + acceptance: add the `diverge` row to the README Tools table and note it in `CLAUDE.md` (tool list + repo layout); create `examples/acceptance_diverge.rs` (mocked offline shape: distinct lens-labeled framings, dedup collapses duplicates) as the live-dogfood scaffold.
- [ ] T012 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.
- [ ] T013 **Live SC-001 / SC-003 dogfood** (after merge + rebuild + restart, the one step needing the running binary): run `diverge` on a problem with a clear dominant framing and confirm ≥3 materially distinct lens-labeled framings (SC-001); run the same problem with and without a stated preference in `context` and confirm the distinct-framing set is not narrowed (SC-003). Record the result. (Not a `cargo test` — a live model property.)

## Dependencies & Execution Order

- **Foundational (T001–T004)** blocks everything (the mode, passes, aggregation, wiring).
- **US1 (T005–T007)** then **US2 (T008–T009)**: both live in `diverge.rs`; US1 adds dedup
  and its tests, US2 the stance-blindness tests. Largely independent once foundational lands.
- **Polish (T010–T012)** after both stories; **T013** after merge + rebuild (the only
  non-offline step).

## Parallel Execution Examples

- After T001–T004: T006 (unit) parallel with T008 (stance test); T007 and T009 once dedup/wiring land.
- Polish: T010 and T011 parallel; T012 last; T013 post-merge.

## Implementation Strategy

US1 (distinct, deduplicated framings) is the headline value — it makes the tool a real
divergence engine. US2 (stance-blindness) is the structural property that keeps the
divergence honest. Everything except **T013** is implementable and testable **offline**
(wiremock-mocked model); T013 is the single live confirmation of SC-001/SC-003, which a
mock cannot produce — identical to how `verify`'s SC-001 was confirmed in 010.
