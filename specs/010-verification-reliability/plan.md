# Implementation Plan: Verification Reliability

**Branch**: `010-verification-reliability` | **Date**: 2026-06-14 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/010-verification-reliability/spec.md`

## Summary

Two reliability fixes to the verify family, both surfaced by dogfooding:

- **US1 — `verify` lens-diversity.** Today `verify` hands one identical prompt to
  all *k* passes, so they converge and agreement-derived confidence is near-binary
  (0/8 graduated in live calls). Give each pass a **distinct critical lens** so
  genuinely contestable claims scatter and confidence spans its range. The
  aggregation math (majority, tie→refuted, dedup, confidence = majority/completed,
  quorum) is unchanged — only the per-pass *prompt* diversifies.
- **US2 — `grounded_verify` abstain + `inconclusive`.** When a claim's truth is a
  computable property of the source (a count, a numeric comparison), or when the
  passes self-report that the decisive evidence is missing, `grounded_verify` must
  not emit a confident supported/refuted verdict. It returns a new server-assembled
  **`inconclusive`** verdict (routing computable claims to `check`). v1 **detects and
  abstains**; actually computing the property via `check` is a named follow-up.

## Technical Context

**Language/Version**: Rust 1.94 (edition 2021). **No new dependencies.**

**Primary Dependencies**: the existing `verify` mode/ensemble, the `grounded_verify`
mode (008/009), the shared `aggregate_core`, the constrained-output contract.

**Storage**: unchanged (one invocation record per call).

**Testing**: `cargo test`. The aggregation branches (US1 FR-004/SC-005) are tested
deterministically with constructed vote vectors. The `inconclusive` mapping (US2) is
tested with a mocked model. **Note:** US1's headline SC-001 — that real contestable
claims actually *scatter* across lenses to yield graduated confidence — is a
property of the **live model** (a wiremock returns canned responses and cannot
disagree with itself). Offline tests cover the *mechanism* (distinct lens prompts
per pass; aggregation yields ≈0.67 on a 2:1 vote vector); confirming SC-001 itself is
a **live dogfood**, unlike 008/009 which were fully offline.

**Target Platform**: stdio MCP server (Linux / Windows / macOS).

**Project Type**: single Rust project, extended in place.

**Performance Goals**: unchanged — *k* passes as today; lens injection is a prompt
change, not extra calls.

**Constraints**: stance-blindness preserved (a lens is a *critical perspective*, not
the caller's stance); the `verify` verdict set and aggregation are unchanged; the
`inconclusive` verdict is server-assembled.

**Scale/Scope**: one lens set, a `<<lens>>` prompt slot, one new per-pass detection
flag on the grounded pass, one new server-assembled output verdict value, and the
mapping logic. No new module.

## Constitution Check

*Evaluated against `.specify/memory/constitution.md`.*

- **I. Design-Corpus Fidelity** — ✅ **corpus-restoring.** `NEW_SERVER_DESIGN.md`
  §"Designing real independence" already mandates *diverse lenses, not N identical
  critics* (the `research` layer honors it; `verify` did not). US1 brings `verify`
  into compliance — not a deviation, a fix. No amendment needed.
- **II. Constrained-Output Contract** — ✅ the grounded pass schema gains one **flat**
  boolean (`needs_computation`); it stays flat + closed. `verify`'s pass schema is
  untouched. The `inconclusive` verdict is **server-assembled** (not in any model-pass
  schema).
- **III. Compiler-Enforced Discipline** — ✅ no `unwrap`/`expect` in production; lints
  unchanged.
- **IV. Seams, Composition, Tests** — ✅ no new seam; lens assignment and the
  inconclusive mapping are pure functions, unit-tested. `aggregate_core` reused.
- **V. Deterministic Over Probabilistic** — ✅ US2 routes a computable claim *away*
  from probabilistic judgment toward the deterministic `check` layer — a direct
  application of the principle the bug violated.
- **VI. Capabilities Off By Default** — ✅ no new capability or gate.
- **VII. Simplicity and Scope Discipline** — ✅ both fixes are bounded: a lens array +
  a prompt slot; a detection flag + a server-assembled verdict + mapping. Actual
  computation of the property is explicitly deferred.

**Gate result**: PASS, no deviation requiring amendment. Two **plan-discovered
refinements** of the clarification are documented in research.md (D4) and flagged at
report time — they tighten the mechanism, not the scope.

## Project Structure

### Documentation (this feature)

```text
specs/010-verification-reliability/
├── plan.md          # This file
├── research.md      # Phase 0 — lens set, lens↔k assignment, detection mechanism, inconclusive home
├── data-model.md    # Phase 1 — Lens, the pass detection flag, GroundedVerdictKind
├── quickstart.md    # Phase 1 — what graduated confidence and inconclusive look like
├── contracts/
│   └── verification-reliability.md   # the changed verify/grounded_verify outputs
└── tasks.md         # Phase 2 — /speckit-tasks
```

### Source Code (repository root)

```text
src/modes/
├── verify.rs            # MODIFIED — LENSES const; PROMPT_TEMPLATE gains <<lens>>; run() assigns lens[i % len] per pass; aggregation unchanged; deterministic vote-vector tests
└── grounded_verify.rs   # MODIFIED — GroundedPass gains `needs_computation: bool`; GroundedVerdict.verdict becomes a 3-value GroundedVerdictKind {supported, refuted, inconclusive}; server maps majority needs_computation → inconclusive (the only abstain trigger), routing computable claims to check; missing_evidence stays advisory (no over-abstention)

tests/integration.rs     # MODIFIED — 010 block: inconclusive on the server.rs reproduction (needs_computation); confident verdict still stands when only advisory missing_evidence is listed (no over-abstention); judgment path unchanged
examples/                # MODIFIED/NEW — acceptance: the borderline battery (live) + the reproduction case
```

**Structure Decision**: no new module — both fixes live in the two existing mode
files. US1 is a prompt/orchestration change in `verify.rs`; US2 adds a detection flag
and a server-assembled verdict value in `grounded_verify.rs`. The per-pass
`VerdictKind` (shared with `verify`) stays `{supported, refuted}`; only
`grounded_verify`'s *output* verdict gains `inconclusive`, so `verify` is untouched.

## Complexity Tracking

*No constitution violation requiring justification.* The two plan-discovered
refinements (research.md D4) are recorded there, not here: (1) the `check` classifier
does not detect countable-property-of-source claims, so detection uses a per-pass
self-report flag; (2) that flag is a one-boolean addition to the grounded pass schema,
consistent with the clarification's intent (inconclusive stays server-assembled; the
passes still emit supported/refuted).
