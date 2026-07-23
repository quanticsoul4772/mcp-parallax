# Tasks: Push Memory

**Input**: Design documents from `/specs/016-push-memory/`

**Prerequisites**: plan.md, spec.md (3 user stories, 3 decided clarifications), research.md (D1–D11), data-model.md, contracts/surface.tool.json

**Tests**: REQUIRED (Constitution Principle IV). All test tasks run through the seams (`MockEmbedder`/`MockStorage`/`MockTimeProvider`, wiremock for integration) — no network, no disk. Write each story's tests first and watch them fail before implementing.

**Organization**: Tasks grouped by user story; the S2 spike is the one task needing a live harness session (local build, pre-merge).

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

No setup tasks — existing crate: no new dependencies, no new configuration, additive storage only (plan.md Technical Context).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the pure core and the storage seam every story builds on.

- [X] T001 Create `src/memory/push.rs` (pure half, data-model §1/§6): constants `PUSH_RELEVANCE_TAU = 0.55`, `PUSH_CAP = 3`, `PUSH_BUDGET_MS = 500`, `PUSH_PROMPT_CHARS = 2000` (doc comments citing provenance per research D9); types `PushRecord` (data-model §4) and `SurfacedMemory {id, kind, trust, score, content}`; pure `select(ranked, already_pushed) -> Vec<SurfacedMemory>` implementing trusted-filter → raw-cosine floor → suppression subtract → cap most-relevant-first; pure `advisory_context(&[SurfacedMemory]) -> String` building the fixed template (research D7 — label, kind/trust/id, verbatim content, `forget(<id>)` pointer, no instruction phrasing). Co-located unit tests: floor excludes 0.54 and admits 0.55; untrusted excluded at any score; cap and ordering; suppression subtract; empty ⇒ empty; template wording pins (label + advisory + forget hint present, no imperative "apply/use" phrasing). Register `pub mod push` in `src/memory/mod.rs`.
- [X] T002 Extend the storage seam: `record_push(&PushRecord)` and `pushed_memory_ids(session_id) -> Vec<String>` on `src/traits/storage.rs` (mockall-mocked); `push_records` table (additive `CREATE TABLE IF NOT EXISTS`, data-model §4) + both impls in `src/storage/sqlite.rs`; sqlite tests: record → read-back round trip, `pushed_memory_ids` returns the union across a session's rows and nothing across sessions, empty-array silence rows round-trip. Depends on T001 (`PushRecord`).

**Checkpoint**: `cargo test push && cargo test storage` green; foundation ready.

---

## Phase 3: User Story 1 — Relevant prior knowledge arrives without asking (Priority: P1) 🎯 MVP

**Goal**: seeded trusted memory + related prompt ⇒ surfaced advisory block with content, id, and trust (spec SC-001).

**Independent Test**: mock-seam and wiremock scenarios below; live half deferred to T015.

### Tests for User Story 1 (REQUIRED — write first, watch fail) ⚠️

- [X] T003 [P] [US1] Failing orchestration tests in `src/memory/push.rs`: `run()` with `MockEmbedder` (fixed vector) + `MockStorage` (seeded trusted memories, empty pushed-ids) ⇒ result with `surfaced` populated most-relevant-first, `fail_open == false`, embed `input_tokens` attributed, and exactly one `record_push` call whose `surfaced_ids` match; nothing above the floor ⇒ empty `surfaced`, still exactly one record (silence row).
- [X] T004 [P] [US1] Failing integration tests in `tests/integration.rs`: (a) `serve_with_memory` + `save` an "alpha"-keyed fact + `surface` call with a related prompt ⇒ `surfaced[0].id` matches the saved id, `hookSpecificOutput.hookEventName == "UserPromptSubmit"`, `additionalContext` contains the label, the id, `first_hand`, and the verbatim content; (b) the served `surface` tool's description and param/result property sets match `specs/016-push-memory/contracts/surface.tool.json` (the checkpoint contract-equality pattern); (c) with plain `serve` (no memory), the catalog does NOT list `surface`.

### Implementation for User Story 1

- [X] T005 [US1] Implement `run()` in `src/memory/push.rs`: excerpt (`PUSH_PROMPT_CHARS`), embed via the `Embedder` seam, `Storage::load_memories` + `ranking::rank`, `pushed_memory_ids` subtract, `select`, `advisory_context`, `record_push` — the whole pipeline under `tokio::time::timeout(PUSH_BUDGET_MS)` with the fail-open recover pattern (checkpoint `run.rs` shape). Depends on T001, T002, T003.
- [X] T006 [US1] Wire `src/server.rs`: `SurfaceParams`/`SurfaceResult` (schemars, contract shapes), memory-gated catalog registration beside `save`/`recall`/`forget`, `run_recorded` invocation, `hookSpecificOutput` present only when `surfaced` is non-empty. Depends on T005; makes T004 green.

**Checkpoint**: US1 green — push works end-to-end on mocks and wiremock.

---

## Phase 4: User Story 2 — Push never degrades a session (Priority: P2)

**Goal**: silence on irrelevant prompts, byte-identical memory-off behavior, fail-open on failure/timeout, once-per-session suppression, untrusted never surfaced (spec SC-002/SC-003/SC-004/SC-006).

**Independent Test**: the scenarios below run standalone against the wired tool.

### Tests for User Story 2 (REQUIRED — write first, watch fail) ⚠️

- [X] T007 [P] [US2] Failing tests in `src/memory/push.rs` tests + `tests/integration.rs`: (a) unrelated prompt (below-floor vectors) ⇒ empty `surfaced` AND no `hookSpecificOutput` key in the serialized result (SC-002 — silence injects nothing); (b) seeded untrusted memory with perfect relevance ⇒ never surfaced (FR-004); (c) embedder error ⇒ `fail_open` silence, no error to caller, record written (SC-004); (d) slow embedder past `PUSH_BUDGET_MS` (the gate's SlowEmbedder pattern) ⇒ fail-open within budget (SC-004); (e) second call, same session, same memory (storage returns its id as pushed) ⇒ suppressed; different session ⇒ surfaced again (FR-005/SC-006); (f) memory-off: full existing suite untouched and `surface` absent from the catalog (SC-003 — extends T004(c)); (g) FR-009 explicit: a `recall` call interleaved with `surface` calls in the same session returns results identical to a pull-only session (push reads never perturb the pull surface).

### Implementation for User Story 2

- [X] T008 [US2] Close whatever T007 surfaces in `src/memory/push.rs` / `src/server.rs` — expected mostly verification: the timeout, recover, suppression-subtract, and absent-when-empty mapping land in T005/T006; this task pins the degraded paths and fixes gaps the tests find. Depends on T005–T007.

**Checkpoint**: US1 + US2 green — push provably costs nothing when it has nothing to say.

---

## Phase 5: User Story 3 — Every push evaluation is auditable (Priority: P3)

**Goal**: one `push_records` row per evaluation; surfacing rate and per-memory counts computable; OTLP mirror without content leakage (spec SC-005).

### Tests for User Story 3 (REQUIRED — write first, watch fail) ⚠️

- [X] T009 [P] [US3] Failing tests: across surfaced / silent / fail-open scenarios, `record_push` is called exactly once each with correct `surfaced_ids`, `latency_ms`, `fail_open`, `input_tokens` (unit, `src/memory/push.rs`); `observability::emit_push` exports a `parallax.push` span whose attributes carry session id, surfaced count, latency, fail_open, and tokens but NO memory content or ids-with-content (the checkpoint no-evidence rule — in-memory exporter assertions in `src/observability.rs` tests).

### Implementation for User Story 3

- [X] T010 [US3] Implement `emit_push` in `src/observability.rs` at the same exit point as the store write (007 one-measurement-two-sinks), wired from `run()`. Depends on T005, T009.

**Checkpoint**: all three stories independently green.

---

## Phase 6: Polish & Cross-Cutting

- [X] T011 **S2 spike** (live, local debug build — pre-merge, gates the integration entry): follow `quickstart.md` — provisional `UserPromptSubmit` mcp_tool hook → `surface`; capture the harness's actual payload field names and substitution behavior; confirm `additionalContext` reaches the model (quote-back test); record verified shapes as an S2 section in `examples/spike_hooks.md`; finalize the `UserPromptSubmit` entry in `integrations/claude-code/hooks.json` and fix `SurfaceParams`/result mapping if the live shapes differ. **Failure branch**: if the harness offers no model-visible context channel for this hook shape at all, STOP — do not merge the tool surface with no consumer; amend the plan's delivery decision (research D2) with the finding and re-plan delivery, per the 006 MCP-reality amendment precedent. Depends on T006.
  - **Result (2026-07-23, live, two rounds):** PASS — no STOP branch. Round 1 (unrelated prompt): hook fired, `${session_id}`/`${prompt}` substituted (loud -32602 would have flagged a mismatch), silence row recorded (227 ms, 0 surfaced, fail_open 0), nothing injected. Round 2 (seeded marker fact + related prompt): full `additionalContext` round-trip — the model received and quoted back the verbatim advisory template; audit row `surfaced_ids: [seed]`, 176 ms. `hooks.json` finalized from the verified shape; S2 section recorded in `examples/spike_hooks.md`; seed forget-cleaned.
- [X] T012 [P] Amend `docs/design/MEMORY_LAYER.md` (research D11): dated 2026-07-23 amendment — the push half of "effortless, not manual" shipped (per-turn, deterministic, trusted-only, once-per-session, 500 ms budget, audited); auto-capture stays open, coupled to the consolidation levers per the 016 clarify record (Constitution I same-change rule).
- [X] T013 [P] Append the feature to `CHANGELOG.md` `## [Unreleased]` (Keep a Changelog 1.1.0, per convention).
- [X] T014 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.
- [X] T015 **Live SC-001/SC-002/SC-006 dogfood** (after merge + rebuild + restart, with the S2-verified hooks installed): follow `quickstart.md` — seed a distinctive fact, new session, related prompt ⇒ labeled memory in context; repeat ⇒ suppressed; unrelated prompt ⇒ nothing; inspect the three `push_records` rows; `forget` the seed. Record the result here. (Not a `cargo test` — whether real embeddings rank a real prompt above the floor is a live property; the mechanics are proven by T003–T010.)
  - **Result (2026-07-23, live, merged code path — the running binary is Rust-identical to `d79c2dd`):** PASS with one named finding. (a) SC-001 — a near-paraphrase prompt surfaced the seeded fact end-to-end: the verbatim advisory template (label, `[fact, first_hand, memory 2d20df84…]`, content) arrived in the model's context and was applied; audit row `["2d20df84…"]`, 115 ms. (b) SC-006 — the immediate same-topic follow-up surfaced nothing: suppression row `[]` directly after the surfacing row (live once-per-session suppression confirmed — the check S2 did not exercise). (c) SC-002 — every unrelated prompt across the live session produced clean silence rows; zero false surfacings in all live evaluations. All rows `fail_open: 0`, all inside the 500 ms budget; seed `forget`-cleaned. **Finding (floor sensitivity, the D9 datum):** the first related-but-not-paraphrase prompt scored **0.406** vs the 0.55 floor — a genuine miss on genuinely related content. The zero-false-surfacing floor buys a real miss rate on differently-worded prompts; this is the first measured data point for tuning `PUSH_RELEVANCE_TAU` from accumulated `push_records`, per research D9 ("moves only with new measurement").

---

## Dependencies & Execution Order

- **Phase 2 blocks everything**: T001 → T002.
- **US1 (MVP)**: T003 ∥ T004 (test-first) → T005 → T006.
- **US2**: T007 (test-first) → T008. Depends on T006 (asserts degraded behavior of the wired tool).
- **US3**: T009 (test-first) → T010. Depends on T005 only; parallel to US2 after that.
- **Polish**: T011 after T006 (needs the working tool, local build); T012 ∥ T013 any time; T014 after all code tasks; T015 after merge + T011.

## Parallel Example: User Story 1

```text
# After Phase 2, write tests in parallel:
T003: run() orchestration tests in src/memory/push.rs
T004: end-to-end + contract-equality tests in tests/integration.rs
# Then sequentially: T005 (push.rs) → T006 (server.rs)
```

## Implementation Strategy

MVP is US1: after T006 the push loop works end-to-end on mocks and wiremock
and is demonstrable. US2 hardens every degraded configuration, US3 pins the
audit surface; each checkpoint is a stable stopping point. T011 (S2 spike) is
deliberately before merge — it runs on a local build and is the only step
that can invalidate the hooks.json entry and the result mapping; T015 is the
only post-merge step, same pattern as 012–015.
