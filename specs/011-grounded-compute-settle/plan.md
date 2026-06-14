# Implementation Plan: Grounded Compute-Settle

**Branch**: `011-grounded-compute-settle` | **Date**: 2026-06-14 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/011-grounded-compute-settle/spec.md`

## Summary

The named 010 FR-005 follow-up. When a majority of `grounded_verify` passes flag a
claim computable (`needs_computation`, 010) **and** they converge on an in-class,
single-source compute spec — a line/byte/literal-match count of one named source
compared to a numeric threshold — the server counts the property over the verbatim
bytes it already read and settles the claim with the existing deterministic engine
(`arithmetic::evaluate`), returning `supported`/`refuted` with the executed form and the
engine's raw result. Everything outside that narrow class still abstains with 010's
`inconclusive` verdict, so the no-confidently-wrong guarantee is never weakened. The
model identifies *what* to count; the server counts and the engine decides.

## Technical Context

**Language/Version**: Rust 1.94 (edition 2021). **No new dependencies** (reuses
`evalexpr` via `arithmetic::evaluate`).

**Primary Dependencies**: the 010 `grounded_verify` mode (per-pass `needs_computation`,
the server-assembled `inconclusive` verdict), the assembler (`grounded::assemble`), and
the deterministic arithmetic engine (`deterministic::arithmetic::evaluate`, 005).

**Storage**: unchanged (one invocation record per call).

**Testing**: `cargo test`. The compute path is fully offline-testable with a mocked
model (the passes return canned compute fields) and a mocked `SourceReader` (returns
known content), because the value is **server-counted**, not model-produced — so unlike
010's SC-001, there is no live-model-only property here. The reproduction (a 1224-line
file) is a deterministic fixture.

**Target Platform**: stdio MCP server (Linux / Windows / macOS).

**Project Type**: single Rust project, extended in place.

**Performance Goals**: unchanged — the count is a single linear scan of bytes already in
memory; no extra model hop (the compute fields ride the existing passes).

**Constraints**: the model never produces the value or the verdict (FR-003); the
supported class is exactly line/byte/match counts of a single source vs a numeric
threshold (clarification); anything else abstains (FR-005); the `check` engine is reused
not reimplemented (FR-007).

**Scale/Scope**: four flat nullable fields on the grounded pass schema, raw-content
capture in the assembler, a pure compute-spec aggregation + count + settle function, two
optional output fields. No new module, no new trait seam.

## Constitution Check

*Evaluated against `.specify/memory/constitution.md`.*

- **I. Design-Corpus Fidelity** — ✅ **corpus-applying.** This is the
  deterministic-over-probabilistic principle (`NEW_SERVER_DESIGN.md` §4) applied one
  step further than 010: a computable property is settled by the solver, not abstained
  on. It is the explicitly-named 010 follow-up (FR-005), not a new corrective — no tool
  is added, `grounded_verify`'s role is unchanged. No amendment needed.
- **II. Constrained-Output Contract** — ✅ the pass schema gains four **flat** nullable
  scalars/enums (`compute_property`, `compute_match_literal`, `compute_operator`,
  `compute_threshold`); the `assert_flat` gate already admits exactly these shapes. The
  settled output's two new fields (`executed_form`, `engine_result`) are
  **server-assembled**, not in any model schema.
- **III. Compiler-Enforced Discipline** — ✅ no `unwrap`/`expect` in production; the
  count and the comparison construction are total functions over validated inputs;
  lints unchanged.
- **IV. Seams, Composition, Tests** — ✅ no new seam. The compute-spec aggregation, the
  count, and the settle are pure functions, unit-tested; `arithmetic::evaluate` and the
  existing `SourceReader`/assembler seams are reused. Fully mockable, no disk/network.
- **V. Deterministic Over Probabilistic** — ✅ the heart of the feature: the value is
  counted deterministically and the verdict is the engine's, never a judge's. Directly
  realizes the principle 010 began applying here.
- **VI. Capabilities Off By Default** — ✅ no new capability or gate; `grounded_verify`
  is already gated on `GROUNDED_VERIFY_ROOT`. The compute path is an internal branch of
  an existing, already-gated tool.
- **VII. Simplicity and Scope Discipline** — ✅ bounded to one narrow class
  (line/byte/match counts, single source, numeric threshold); every broadening
  (aggregates, ranges, predicates, parsing) is explicitly deferred and routed to the
  existing abstain path.

**Gate result**: PASS, no deviation requiring amendment.

## Project Structure

### Documentation (this feature)

```text
specs/011-grounded-compute-settle/
├── plan.md          # This file
├── research.md      # Phase 0 — the five mechanism decisions (D1–D5)
├── data-model.md    # Phase 1 — pass fields, ComputeSpec, settle flow, verdict fields
├── quickstart.md    # Phase 1 — what a settled compute verdict looks like
├── contracts/
│   └── compute-settle.md   # the changed grounded_verify pass + output
└── tasks.md         # Phase 2 — /speckit-tasks
```

### Source Code (repository root)

```text
src/modes/grounded_verify.rs   # MODIFIED — GroundedPass gains compute_property/_match_literal/_operator/_threshold (flat nullable); GroundedVerdict gains optional executed_form + engine_result; aggregation: majority-agreed in-class single-source spec → count + arithmetic::evaluate → supported/refuted; else 010 inconclusive
src/grounded/assemble.rs        # MODIFIED — AssembledEvidence also surfaces the raw per-unit content (text+bytes) so the count runs over verbatim source, not the header-framed block; single-unit detection
tests/integration.rs            # MODIFIED — 011 block: server.rs>1000 → supported (1224>1000); >5000 → refuted; out-of-class/multi-source computable → inconclusive; judgment path unchanged
examples/acceptance_grounded_verify.rs  # MODIFIED — the compute-settle reproduction
```

**Structure Decision**: no new module. The feature is an internal branch added to
`grounded_verify`'s aggregation between the 010 abstain decision and the `inconclusive`
return, plus a small assembler change to expose raw content, plus the reused arithmetic
engine. The per-pass `VerdictKind` (shared with `verify`) is untouched; `verify` is
untouched.

## Complexity Tracking

*No constitution violation requiring justification.* One plan-discovered note recorded in
research.md (D2): the assembler currently frames-and-discards the raw per-unit content,
so it must surface it for the count — a small additive change to `AssembledEvidence`,
not a new structure or seam.
