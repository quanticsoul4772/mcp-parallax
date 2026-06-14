# Tasks: Preference Elicitation — the Wrong-Objective Corrective

**Feature**: `014-preference-elicitation` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

Tests included (Constitution IV). The mechanism + the recall integration are
offline-testable (mocked model + mock embedder + in-memory storage); only the inference
*quality* (SC-001 right objective, SC-002 catching a seeded conflict) is a live dogfood
(T012). One new single-pass mode with an optional memory dependency, plus server wiring.

## Phase 1: Setup

No new dependencies, no new module beyond the mode file. No setup tasks.

## Phase 2: Foundational (blocking all stories)

The mode, its run+assembly, the recall integration, and the catalog wiring.

- [ ] T001 Create `src/modes/elicit.rs`: `ELICIT_ID` (`"elicit"`), `ELICIT_DESCRIPTION` (the routing text from contracts/elicit.md), a scalar `SignalLevel` enum (`Low | Medium | High`, lowercase serde), and `PROMPT_TEMPLATE` with `<<task>>`, `<<context>>`, `<<preferences>>` slots (task/context are the only caller-prose inputs; `<<preferences>>` is server-filled — stance-blind). Define `ElicitParams { task: String, context: Option<String> }` and the flat+closed `ElicitPass { assumed_objective: String, preference_texts: Vec<String>, preference_signals: Vec<String>, preference_strengths: Vec<String>, divergence_questions: Vec<String>, divergence_signals: Vec<String>, signal_level: SignalLevel }`, plus `GoverningPreference`, `DivergencePoint`, `ElicitResult`, and `register(registry)` (single pass, flat+closed). Add `pub mod elicit;` to `src/modes/mod.rs`.
- [ ] T002 In `src/modes/elicit.rs`, add `check_input` (reject empty/whitespace or oversize `task` before any model call — FR-008) and the single-pass `run(client, mode, memory, params, max)` core *without* recall yet: build the prompt (inject `task`, `context`, and a `<<preferences>>` placeholder `"(no stored preferences — memory not configured)"`), one pass → `validate` schema → typed `ElicitPass` → **well-formedness validation** (the three `preference_*` arrays equal length, the two `divergence_*` arrays equal length, every `preference_strengths` is `"revealed"`/`"stated"` — else a loud failed pass, never normalized; 013 convention; empty arrays valid) → **zip** into `GoverningPreference`/`DivergencePoint` → assemble `ElicitResult { assumed_objective, governing_preferences, divergence_points, signal_level, memory_consulted: false }`. No `aggregate_core`, no quorum.
- [ ] T003 In `src/modes/elicit.rs`, add the **recall integration** (FR-003/SC-004): `run` takes `memory: Option<&MemoryDeps>`; when `Some`, call `memory::tools::recall(deps, &RecallParams { query: params.task.clone(), kind: None, limit: Some(RECALL_LIMIT) })`, keep the **trusted** recalled memories, format them into the `<<preferences>>` slot as "stored verified preferences (revealed signal — outrank merely stated ones)", and set `memory_consulted = true`. `RECALL_LIMIT` is a named constant (5). When `None`, keep the placeholder + `memory_consulted = false`. A recall failure surfaces, not hidden.
- [ ] T004 Register and expose the tool in `src/server.rs`: `elicit::register(&mut registry)?` in the catalog build (always on, no gate), the `#[tool(name = "elicit", ...)]` entry, and `elicit_with_ct` through `run_recorded` (one record) passing `self.memory.as_deref()` as the optional `MemoryDeps`, returning `Json<ElicitResult>`. Update the catalog assertions: `"elicit"` sorts after `diverge` and before `forget`/`unstick` in the name lists (`src/server.rs` + `tests/integration.rs`), and bump the stdio-smoke tool count 8 → 9.

## Phase 3: User Story 1 — surface the assumed objective and what should govern it (P1)

**Goal**: the tool returns the assumed objective and the governing preferences, each traced to its signal, with stored verified preferences (when present) marked as the stronger signal.

**Independent test**: a mocked inference → the assembled output carries the objective + zipped preferences with signal+strength; with a seeded trusted memory the recall reaches the prompt and `memory_consulted` is true.

- [ ] T005 [P] [US1] Unit tests in `src/modes/elicit.rs`: the per-pass schema registers flat + closed (string + scalar enum + arrays of scalars); `assemble` zips `preference_texts`/`_signals`/`_strengths` into `GoverningPreference` traced to its signal; an arity mismatch (prefs) and an invalid `preference_strengths` value are each a loud failed pass; the prompt template has exactly the three slots (stance-blind).
- [ ] T006 [US1] Integration test in `tests/integration.rs` (014 block): a mocked inference → `elicit` returns `assumed_objective` + `governing_preferences` (with signal+strength) + `signal_level` + `memory_consulted`, **no** `verdict`/enforcement field (FR-006/SC-005), and exactly one record.
- [ ] T009 [US1] Integration test in `tests/integration.rs` (014 block, memory configured via `serve_with_memory`): seed a **trusted** memory + a mock embedder; a `elicit` call recalls it (assert the request body the mock model receives **contains the memory content** — the recall reached the prompt) and `memory_consulted` is true (FR-003/SC-004 mechanism). Without memory (`serve`): the call runs, `memory_consulted` is false, and the prompt notes no stored signal.

## Phase 4: User Story 2 — name the divergence points (P1)

**Goal**: the tool returns the divergence points (zipped question+signal); none when the signals are consistent.

**Independent test**: a mocked inference with divergence arrays → zipped `divergence_points`; an inference with empty divergence arrays → no divergence points.

- [ ] T007 [US2] Tests (mocked model) in `src/modes/elicit.rs` or `tests/integration.rs`: a divergence-bearing inference → `divergence_points` zipped (question + conflicting signal); an arity mismatch (divergence arrays) is a loud failed pass; an inference with empty divergence arrays → `divergence_points: []` (no manufactured doubt, FR-004 scenario 2).

## Phase 5: User Story 3 — inference, not interrogation (P2)

**Goal**: with little/no signal the tool reports it and fabricates nothing.

**Independent test**: a low-signal canned inference → empty preferences and divergence, `signal_level: low`.

- [ ] T008 [US3] Test (mocked model) in `src/modes/elicit.rs` or `tests/integration.rs`: a canned inference with `signal_level: "low"`, empty `preference_*`, empty `divergence_*` → `ElicitResult` with `signal_level: low`, `governing_preferences: []`, `divergence_points: []` (SC-003: 0 fabricated).

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T010 [P] Docs + acceptance: add the `elicit` row to the README Tools table, update the intro count (thirteen → fourteen) and the always-on/cognitive-correctives lists; note `elicit` in `CLAUDE.md` (tool-serving paragraph + repo layout); create `examples/acceptance_elicit.rs` (offline shape: with-memory recall reaching the prompt, and a low-signal no-fabrication case).
- [ ] T011 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.
- [ ] T012 **Live SC-001 / SC-002 dogfood** (after merge + rebuild + restart, the one step needing the running binary): run `elicit` on a task whose surface objective is qualified by context and confirm it surfaces the *right* assumed objective with governing preferences (SC-001); run a task where a stated request conflicts with a stored verified preference and confirm the conflict appears as a divergence point citing the stored signal (SC-002), and that the stored preference is marked `revealed`. Record the result. (Not a `cargo test` — inference quality is a live-model property; the recall + assembly are already proven offline.)

## Dependencies & Execution Order

- **Foundational (T001–T004)** blocks everything (the mode, run+assembly, recall, wiring).
  T002 lands the core, T003 adds recall, T004 wires the server.
- **US1 (T005/T006/T009)**, **US2 (T007)**, **US3 (T008)**: all exercise the same
  assembled output through `run`; independent once foundational lands.
- **Polish (T010–T011)** after the stories; **T012** after merge + rebuild (the only
  non-offline step).

## Parallel Execution Examples

- After T001–T004: T005 (unit) parallel with T007/T008 (mocked-inference tests).
- Polish: T010 parallel with nothing blocking; T011 last; T012 post-merge.

## Implementation Strategy

US1 (surface the objective + governing preferences, including the recalled stored ones) is
the headline. US2 (divergence points) is the actionable half; US3 (inference-not-
interrogation) is the guardrail. Everything except **T012** is offline-testable — the
recall, the validation, and the assembly are deterministic — so only the inference quality
(right objective, real conflict) waits on the live binary.
