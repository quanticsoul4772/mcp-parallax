# Implementation Plan: Diverge — Independent Perspectives

**Branch**: `012-diverge-perspectives` | **Date**: 2026-06-14 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/012-diverge-perspectives/spec.md`

## Summary

`Diverge` is the divergence corrective: when the caller is anchored on one framing,
it runs `k` stance-blind passes under **distinct generative lenses** (invert the goal,
change the actor, shift the horizon, deny the load-bearing assumption, reframe the
problem class) and returns a **deterministically deduplicated** set of materially
distinct framings — each a one-line reframing plus its implication, labeled with the
lens that produced it. It reuses `verify`'s ensemble orchestration, lens-array pattern,
and constrained-output contract, but **not** its verdict aggregation: Diverge scatters
(collect + dedup), it does not converge (majority + confidence). New mode, no new gate.

## Technical Context

**Language/Version**: Rust 1.94 (edition 2021). **No new dependencies.**

**Primary Dependencies**: the mode registry + `CorrectiveMode` (`src/modes/mod.rs`), the
`k`-pass ensemble orchestration and `LENSES`/`<<lens>>` pattern from `verify`
(`src/modes/verify.rs`), the constrained-output contract (`ModelClient` + flat schema),
and `verify::dominant_failure` (or an equivalent) for the zero-completion case.

**Storage**: unchanged (one invocation record per call).

**Testing**: `cargo test` for the mechanism (distinct lens prompts, flat+closed schema,
deterministic dedup over constructed perspective sets, stance-blind prompt structure,
zero-completion → dominant failure). **SC-001 / SC-003** (real problems scatter into ≥3
distinct framings; a stated stance does not narrow the set) are **live-model** properties
— a wiremock cannot diverge — confirmed by a **live dogfood**, as `verify`'s SC-001 (010).

**Target Platform**: stdio MCP server (Linux / Windows / macOS).

**Project Type**: single Rust project, new mode added in place.

**Performance Goals**: unchanged — `k` parallel passes as today; dedup is a single
O(n²) pass over ≤ `k` short strings (negligible).

**Constraints**: stance-blindness is structural (problem + optional context the only
subject slots); dedup is deterministic server-side (clarification); each pass emits one
perspective (clarification); output is framings only — never a verdict (`verify`) or a
single step (`unstick`).

**Scale/Scope**: one new mode file (`src/modes/diverge.rs`), a `LENSES` array + `<<lens>>`
prompt, a flat `{framing, implication}` per-pass schema, a pure deterministic dedup
function, a server-assembled `{perspectives, passes}` output, and registration. No new
trait seam, no new gate.

## Constitution Check

*Evaluated against `.specify/memory/constitution.md`.*

- **I. Design-Corpus Fidelity** — ✅ **corpus-applying.** `Diverge` is an existing entry
  in the failure-mode catalog (`NEW_SERVER_DESIGN.md` — *anchoring/tunnel-vision →
  independent perspectives*; `THEORY_OF_MIND.md` perspectives half). This implements a
  named catalog corrective, not a new invention — no amendment required. It realizes the
  corpus's "diverse lenses, not N identical critics" on the *generative* side, the
  counterpart to `verify`'s critical side.
- **II. Constrained-Output Contract** — ✅ the per-pass schema is **flat + closed**: two
  strings (`framing`, `implication`). The lens is server-assigned (not a model field); the
  returned `{perspectives, passes}` set is **server-assembled**, not grammar-constrained.
- **III. Compiler-Enforced Discipline** — ✅ no `unwrap`/`expect` in production; the dedup
  and assignment are total functions; lints unchanged.
- **IV. Seams, Composition, Tests** — ✅ no new seam. Lens assignment and the dedup are
  pure functions, unit-tested; the ensemble orchestration and `ModelClient` seam are
  reused. Mockable, no disk/network.
- **V. Deterministic Over Probabilistic** — ✅ dedup is a deterministic token-Jaccard
  rule (clarification), no embedder/model hop — the verdict-free analogue of the principle.
- **VI. Capabilities Off By Default** — ✅ `Diverge` adds **no** network egress or code
  execution, so it is always in the catalog like `verify`/`unstick` (the off-by-default
  rule governs *capabilities*, not correctives; FR-009).
- **VII. Simplicity and Scope Discipline** — ✅ one mode, one lens array, one flat schema,
  one dedup function; no voting math reused where it does not belong; the dedup threshold
  is a named constant, not a config var.

**Gate result**: PASS, no deviation requiring amendment.

## Project Structure

### Documentation (this feature)

```text
specs/012-diverge-perspectives/
├── plan.md          # This file
├── research.md      # Phase 0 — lens set, assignment, per-pass schema, dedup, aggregation, testing
├── data-model.md    # Phase 1 — Lens, DivergePass, Perspective, DivergeResult, dedup rule
├── quickstart.md    # Phase 1 — what a Diverge call returns
├── contracts/
│   └── diverge.md   # the diverge tool input + per-pass schema + output
└── tasks.md         # Phase 2 — /speckit-tasks
```

### Source Code (repository root)

```text
src/modes/diverge.rs   # NEW — DIVERGE_ID/DESCRIPTION; LENSES (invert/actor/horizon/assumption/class) + <<lens>>; DivergePass {framing, implication} flat schema; run() = k lensed passes; aggregate = collect + deterministic Jaccard dedup → DivergeResult {perspectives, passes}; zero-completion → dominant failure
src/modes/mod.rs       # MODIFIED — `pub mod diverge;`
src/server.rs          # MODIFIED — register diverge in the catalog (always on) + the #[tool] entry + run_recorded wiring
tests/integration.rs   # MODIFIED — 012 block: distinct framings returned + labeled; dedup collapses duplicates; stance-blind structure; one record
examples/acceptance_diverge.rs   # NEW — the live dogfood scaffold (SC-001/SC-003), mocked offline shape
```

**Structure Decision**: a new mode file, parallel to `verify.rs`/`unstick.rs`. It reuses
the registry and ensemble pattern but defines its own aggregation (collect + dedup, no
vote). `verify` and `grounded_verify` are untouched; the shared `aggregate_core` is **not**
used by Diverge (research D5).

## Complexity Tracking

*No constitution violation requiring justification.* One note: Diverge deliberately does
**not** reuse `aggregate_core` (the verdict/majority/confidence math) — reusing it would
force convergence semantics onto a divergence tool. The only shared machinery is the
k-pass orchestration and the constrained-output contract (research D5).
