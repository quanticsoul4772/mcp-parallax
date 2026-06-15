# Tasks: Decide — Methodology-Driven Choice

**Feature**: `013-decide-methodology` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

Tests included (Constitution IV). Because the pick and confidence are deterministic server
math over the model's scores, **SC-001/002/003/005 are offline-testable** (mocked score
vectors); only **SC-004** (the model picks the *fitting* methodology) is a live dogfood
(T010). One new single-pass mode file plus server wiring.

## Phase 1: Setup

No new dependencies, no new module beyond the mode file. No setup tasks.

## Phase 2: Foundational (blocking both stories)

The mode, its single pass, and the catalog wiring.

- [X] T001 Create `src/modes/decide.rs`: `DECIDE_ID` (`"decide"`), `DECIDE_DESCRIPTION` (the routing text from contracts/decide.md), a scalar `Methodology` enum (`Weigh | Causal | Probabilistic`, lowercase serde), and `PROMPT_TEMPLATE` with `<<decision>>`, `<<options>>`, `<<context>>` slots (the only subject inputs — stance-blind; the prompt lists the options in order and states the 0–100 score scale). Define `DecideParams { decision: String, options: Vec<String>, context: Option<String> }` and the flat+closed `DecidePass { methodology: Methodology, option_scores: Vec<i64>, option_rationales: Vec<String>, deciding_factors: Vec<String> }`, and a `register(registry)` that enforces flat+closed. Add `pub mod decide;` to `src/modes/mod.rs`.
- [X] T002 In `src/modes/decide.rs`, add `check_input` (reject empty/whitespace or oversize `decision`, and `options.len() < 2`, before any model call — FR-008) and `run(client, mode, params, max_chars)`: build the prompt (inject `decision`, the ordered `options`, `context`), run **one** pass, and `one_pass` (constrained completion → `validate` schema → typed `DecidePass` → **well-formedness check**: `option_scores.len() == option_rationales.len() == options.len()`, `deciding_factors` non-empty, and **every score within 0–100** — else a failed pass, loud, never clamped/normalized; analyze M1). No ensemble, no quorum; a failed pass propagates.
- [X] T003 Register and expose the tool in `src/server.rs`: `decide::register(&mut registry)?` in the catalog build (always on, no gate), the `#[tool(name = "decide", ...)]` entry, and `decide_with_ct` through `run_recorded` (one record), returning `Json<DecideResult>`. Update the catalog assertions: `"decide"` sorts after `checkpoint_turn` and before `diverge` in the name lists (`src/server.rs` + `tests/integration.rs`), and bump the stdio-smoke tool count 7 → 8.

## Phase 3: User Story 1 — a justified, calibrated recommendation (P1)

**Goal**: the server ranks the model's per-option scores, recommends the top, names the runner-up and why it lost, and reports a confidence derived from the score margin.

**Independent test**: a dominant score vector → the top option at high confidence with the runner-up reason; a near-tie vector → the same shape at low confidence (~0.5); a tie resolves by input order.

- [X] T004 [US1] In `src/modes/decide.rs`, add `OptionAssessment { option, score, rationale }` and the pure aggregation: **zip** the parallel arrays with `params.options` by index (scores are already validated 0–100 by T002 — no clamping here), **stable-sort** descending by score (input-order tiebreak), take top = `recommended` / next = `runner_up`, compute `margin = top.score − runner_up.score`, `confidence = 0.5 + 0.5 * min(margin, 100) / 100` (clamped `[0.5, 1.0]`), compose `runner_up_reason`, and assemble `DecideResult { recommended, runner_up, runner_up_reason, confidence, methodology, deciding_factors, assessments }`. Server-assembled; no verdict, no next_step.
- [X] T005 [P] [US1] Unit tests in `src/modes/decide.rs`: a dominant vector (e.g. `[85, 40]`) → `recommended` is the 85 option, `confidence` ≈ 0.725, runner-up named; a near-tie (`[60, 55]`) → lower confidence (≈ 0.525); an exact tie (`[70, 70]`) → input-order winner, confidence 0.5; the margin→confidence map at margins 0/50/100 → 0.5/0.75/1.0; the per-pass schema registers flat + closed (scalar enum + arrays of scalars).
- [X] T006 [US1] Tests (mocked model) in `src/modes/decide.rs` or `tests/integration.rs`: a full `decide` run returns `recommended` + `runner_up` + `runner_up_reason` + `deciding_factors` + `methodology` + `assessments` (one per option) and **no** `verdict`/`next_step` field (FR-007); an arity mismatch (scores vs options) is a failed pass; an **out-of-range score** (e.g. `105` or `-5`) is a failed pass (loud, not clamped — analyze M1); `< 2` options is rejected before any model call (FR-008/SC-005).

**Checkpoint**: US1's mechanism is in and offline-tested. SC-001/SC-002 (calibration) are proven offline (server math); only methodology-fit remains live.

## Phase 4: User Story 2 — the methodology is surfaced (P1)

**Goal**: the chosen methodology (weigh / causal / probabilistic) is surfaced in the output so the caller sees how the choice was reached; the rationale is in that methodology's terms.

**Independent test**: a mocked pass declaring each methodology surfaces it unchanged in `DecideResult.methodology`.

- [X] T007 [US2] Unit/integration test in `src/modes/decide.rs` (or `tests/integration.rs`): for a mocked pass returning `methodology: "causal"` (and `weigh`, `probabilistic`), `DecideResult.methodology` echoes it lowercased; the enum registers with exactly the three values; `deciding_factors` is carried through. (SC-004 — that the model picks the *fitting* methodology for the decision's shape — is the live dogfood T010, not offline.)

**Checkpoint**: both stories complete and offline-tested.

## Phase 5: Polish & Cross-Cutting Concerns

- [X] T008 [P] Docs + acceptance: add the `decide` row to the README Tools table and update the intro count (twelve → thirteen) and the always-on/cognitive-correctives lists; note `decide` in `CLAUDE.md` (tool-serving paragraph + repo layout); create `examples/acceptance_decide.rs` (offline shape: a dominant and a near-tie score vector → the expected recommendation + calibrated confidence).
- [X] T009 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.
- [X] T010 **Live SC-004 dogfood** (after merge + rebuild + restart, the one step needing the running binary): run `decide` on a multi-criteria decision, a downstream-effects decision, and an uncertainty-dominated decision, and confirm the surfaced `methodology` is `weigh` / `causal` / `probabilistic` respectively (it fits the decision's shape) **and the `deciding_factors`/rationales read in that methodology's terms** (a `causal` decision lists effects, not criteria — FR-003 second half, analyze L1); spot-check that real recommendations and confidences are sensible. Record the result. (Not a `cargo test` — methodology-fit is a live-model property; the calibration math is already proven offline.)
  - **Result (2026-06-14, live, `claude-opus-4-8`):** PASS. Multi-criteria (storage engine; criteria = p95 latency / op cost / familiarity) → `methodology: weigh`, `deciding_factors` are exactly the named criteria, recommends ClickHouse (78 vs DuckDB 70 vs Postgres 58), confidence 0.54. Downstream-effects (deprecate v1 now vs keep a year) → `methodology: causal`, factors read as **effects** not criteria ("enterprise migration strain", "churn risk", "support team load", "compounding technical debt"), recommends keep-a-year, confidence 0.55 (close call). Uncertainty-dominated (launch inventory, no demand history) → `methodology: probabilistic`, factors include "value of information from pre-sales data", recommends "wait for pre-sales data" (the value-of-information answer), confidence 0.58. All three methodologies fit the decision shape and rationales read in their methodology's terms (FR-003 second half holds). **Note:** the first `weigh` attempt errored — the server loudly rejected a pass with `3 options but 3 scores / 5 rationales` (model arity slip); since `decide` is single-pass (k=1) with no retry, one malformed pass kills the call. The loud rejection is correct (Constitution III), but `decide` has no violation-fed retry, so a model arity slip surfaces as a hard error to the caller. Re-running succeeded. Worth tracking if it recurs.

## Dependencies & Execution Order

- **Foundational (T001–T003)** blocks everything (the mode, the pass, the wiring).
- **US1 (T004–T006)** then **US2 (T007)**: both live in `decide.rs`; US1 is the rank/calibrate
  core, US2 the methodology surfacing (largely the enum + assembly already in place).
- **Polish (T008–T009)** after both stories; **T010** after merge + rebuild (the only
  non-offline step).

## Parallel Execution Examples

- After T001–T003: T005 (unit) parallel with T007 (methodology test) once T004 lands.
- Polish: T008 parallel with nothing blocking; T009 last; T010 post-merge.

## Implementation Strategy

US1 (the justified, calibrated recommendation) is the headline — it turns a gut pick into
a server-derived choice with calibrated confidence. US2 (methodology surfacing) makes the
choice auditable. Everything except **T010** is offline-testable (mocked score vectors),
because the pick and confidence are deterministic server math; T010 is the single live
confirmation that the model picks the *fitting* methodology, which a mock cannot judge.
