# Implementation Plan: Decide — Methodology-Driven Choice

**Branch**: `013-decide-methodology` | **Date**: 2026-06-14 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/013-decide-methodology/spec.md`

## Summary

`Decide` is the choice corrective: given a decision and ≥2 options, a single stance-blind
model pass selects the fitting **methodology** (weigh / causal / probabilistic) and emits
a **numeric score per option** (plus per-option rationales and the deciding factors) as
flat scalar arrays. The server zips those with the option labels, ranks by score, and
**deterministically** assembles the recommendation: the top option, the runner-up and why
it lost, the surfaced methodology, the deciding factors, and a confidence derived from the
score **margin**. The model scores; the server chooses and calibrates — so the choice
traces to the factors, never an unexamined preference. New always-on mode, no gate.

## Technical Context

**Language/Version**: Rust 1.94 (edition 2021). **No new dependencies.**

**Primary Dependencies**: the mode registry + `CorrectiveMode` (`src/modes/mod.rs`), the
single-pass mode pattern (`src/modes/unstick.rs`), the constrained-output contract
(`ModelClient` + flat schema), and the schema validator.

**Storage**: unchanged (one invocation record per call).

**Testing**: `cargo test`. Because the pick and confidence are **deterministic server math
over the model's scores**, SC-001 (dominant → high conf), SC-002 (close → lower conf),
SC-003 (output completeness), and SC-005 (< 2 options rejected) are **fully offline** with
a mocked model returning chosen score vectors. Only **SC-004** (the model picks the
*fitting* methodology) is a live property — confirmed by a small live dogfood.

**Target Platform**: stdio MCP server (Linux / Windows / macOS).

**Project Type**: single Rust project, new mode added in place.

**Performance Goals**: one model call per invocation (single pass); the zip + rank +
margin math is O(n) over ≤ a handful of options. No ensemble.

**Constraints**: stance-blindness is structural (decision + options + optional context the
only subject slots); the per-pass schema is flat+closed (scalar enum + arrays of scalars,
research D1); the recommendation and confidence are server-derived from the scores
(FR-004/FR-005); output is a recommendation, never a verdict (`verify`) or a next step
(`unstick`).

**Scale/Scope**: one new mode file, a methodology enum, a flat per-pass schema (parallel
scalar arrays), a pure zip+rank+margin-confidence function, a server-assembled output, and
registration. No new trait seam, no new gate.

## Constitution Check

*Evaluated against `.specify/memory/constitution.md`.*

- **I. Design-Corpus Fidelity** — ✅ **corpus-applying.** `Decide` is a named entry in the
  failure-mode catalog (`NEW_SERVER_DESIGN.md` — *indecision / miscalibration → methodology
  (weigh / causal / probabilistic)*). This implements a catalog corrective, not a new
  invention. No amendment required.
- **II. Constrained-Output Contract** — ✅ the per-pass schema is **flat + closed**: a
  scalar enum (`methodology`) and three arrays of scalars (`option_scores` integer,
  `option_rationales`/`deciding_factors` string). Per-option data is encoded as parallel
  scalar arrays (research D1) because arrays of objects are illegal. The `DecideResult` is
  **server-assembled** (may nest, like `grounded_verify`'s manifest).
- **III. Compiler-Enforced Discipline** — ✅ no `unwrap`/`expect` in production; the
  zip/rank/margin functions are total over validated inputs; lints unchanged.
- **IV. Seams, Composition, Tests** — ✅ no new seam. The zip, the argmax+tiebreak, and the
  margin→confidence map are pure functions, unit-tested; the `ModelClient` seam and
  single-pass pattern are reused. Mockable, no disk/network.
- **V. Deterministic Over Probabilistic** — ✅ the heart of the feature: the pick is
  `argmax(scores)` and the confidence is a fixed function of the margin — both deterministic
  server math over the model's structured scores, never a model gut call.
- **VI. Capabilities Off By Default** — ✅ `Decide` adds no network egress or code
  execution, so it is always in the catalog like `verify`/`unstick`/`diverge` (the
  off-by-default rule governs capabilities, not correctives; FR-009).
- **VII. Simplicity and Scope Discipline** — ✅ one mode, one enum, one flat schema, one
  pure ranking+calibration function; single pass, no ensemble; the score scale and the
  margin→confidence constants are named constants, not config vars.

**Gate result**: PASS, no deviation requiring amendment.

## Project Structure

### Documentation (this feature)

```text
specs/013-decide-methodology/
├── plan.md          # This file
├── research.md      # Phase 0 — parallel-arrays schema, argmax pick, margin confidence, single pass, testing
├── data-model.md    # Phase 1 — DecideParams, DecidePass (flat), OptionAssessment, DecideResult, the rank/calibrate rule
├── quickstart.md    # Phase 1 — what a Decide call returns
├── contracts/
│   └── decide.md    # the decide tool input + per-pass schema + output
└── tasks.md         # Phase 2 — /speckit-tasks
```

### Source Code (repository root)

```text
src/modes/decide.rs    # NEW — DECIDE_ID/DESCRIPTION; methodology enum; PROMPT_TEMPLATE (<<decision>>/<<options>>/<<context>>); DecideParams {decision, options: Vec<String>, context}; flat DecidePass {methodology, option_scores[], option_rationales[], deciding_factors[]}; run() = single pass; aggregate = validate-arity + zip + argmax(+input-order tiebreak) + margin→confidence → DecideResult
src/modes/mod.rs       # MODIFIED — `pub mod decide;`
src/server.rs          # MODIFIED — register decide (always on) + #[tool] entry + decide_with_ct through run_recorded; catalog assertions updated
tests/integration.rs   # MODIFIED — 013 block: dominant→high conf, close→low conf, output completeness, < 2 options rejected, one record
examples/acceptance_decide.rs   # NEW — offline shape (dominant/close score vectors) + the live-dogfood scaffold
```

**Structure Decision**: a new single-pass mode file, closest to `unstick.rs` (one pass,
no ensemble) but with a richer flat schema and a server-side rank/calibrate step. It does
**not** use `verify::aggregate_core` (no votes, no quorum). `verify`/`diverge` are
untouched.

## Complexity Tracking

*No constitution violation requiring justification.* One note: per-option data is carried
as **parallel scalar arrays** rather than the natural array-of-objects, because the
flat-schema gate forbids the latter (research D1). The server re-associates by index and
validates arity — a deliberate encoding for Constitution II, recorded so it is not mistaken
for an oversight.
