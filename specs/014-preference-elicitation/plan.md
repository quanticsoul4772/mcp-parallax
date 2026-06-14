# Implementation Plan: Preference Elicitation — the Wrong-Objective Corrective

**Branch**: `014-preference-elicitation` | **Date**: 2026-06-14 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/014-preference-elicitation/spec.md`

## Summary

`elicit` is the wrong-objective corrective, run *before* the model commits. Given a task
and optional context, a single stance-blind pass surfaces the **assumed objective**, the
**governing preferences** (each traced to its signal, revealed/verified > stated), the
**divergence points** where the assumed objective likely departs from the user's real one,
and a self-reported **signal level**. When memory is configured, the **server** recalls
relevant **trusted** stored preferences (reusing `memory::tools::recall`) and injects them
into the prompt as the revealed signal, so the model can weight them above stated ones and
flag stated-vs-revealed conflicts. The model produces the structured inference (flat
parallel arrays); the server validates, zips, and assembles. It **only surfaces** —
enforcement stays `checkpoint_action`'s job. Always-on, no new gate.

## Technical Context

**Language/Version**: Rust 1.94 (edition 2021). **No new dependencies.**

**Primary Dependencies**: the mode registry + `CorrectiveMode`, the single-pass pattern
(`src/modes/unstick.rs`), the constrained-output contract, and — when memory is configured
— the existing recall seam `memory::tools::recall` over `Arc<MemoryDeps>` (the server's
`memory: Option<Arc<MemoryDeps>>`).

**Storage**: unchanged (one invocation record per call). The recall reads memories; it
writes nothing.

**Testing**: `cargo test` with a mocked model + mock embedder + in-memory storage. The
**mechanism** (schema flat+closed, arity/strength validation, zip/assembly, low-signal →
empty, `memory_consulted`) and the **recall integration** (a seeded trusted memory reaches
the prompt; the mock model captures it) are offline. Only the **inference quality**
(SC-001 right objective, SC-002 catching a seeded conflict) is a live dogfood.

**Target Platform**: stdio MCP server (Linux / Windows / macOS).

**Project Type**: single Rust project, new mode added in place.

**Performance Goals**: one model call per invocation; one cosine recall (when memory
present) over the stored memories — the existing `recall` cost, nothing new.

**Constraints**: stance-blindness is structural (task + context the only caller-prose
slots; recalled prefs are server-fetched, not caller-asserted); per-pass schema flat+closed
(parallel scalar arrays + a scalar enum); output server-assembled; **surfacing only**, no
action/hold/modify (FR-006); a malformed assessment is a loud failed pass (013 convention).

**Scale/Scope**: one new mode file, an optional memory wiring, a flat per-pass schema, a
pure validate+zip+assemble function, a server-assembled output, and registration. No new
trait seam, no new gate.

## Constitution Check

*Evaluated against `.specify/memory/constitution.md`.*

- **I. Design-Corpus Fidelity** — ✅ **corpus-applying.** `Preference elicitation` is a
  named catalog entry (`NEW_SERVER_DESIGN.md` — *wrong objective → preference elicitation +
  enforcement*; `PREFERENCE_ELICITATION.md`). This implements the **elicitation half**; the
  enforcement half (already `checkpoint_action` over memory) is explicitly **not rebuilt** —
  a named scope boundary, not a deviation.
- **II. Constrained-Output Contract** — ✅ the per-pass schema is **flat + closed**: a
  string, a scalar enum (`signal_level`), and five arrays of scalars (per-item data as
  parallel arrays). `preference_strengths` is a server-validated string, not an
  array-of-enums (011 H1 caution). The `ElicitResult` is **server-assembled** (nested, like
  `decide`/`grounded_verify`).
- **III. Compiler-Enforced Discipline** — ✅ no `unwrap`/`expect` in production; validation
  and zip are total over checked inputs; lints unchanged.
- **IV. Seams, Composition, Tests** — ✅ no new seam. The recall reuses `MemoryDeps` /
  `memory::tools::recall`; validation, zip, and prompt-building are pure, unit-tested;
  mockable model + embedder + storage, no real network/disk.
- **V. Deterministic Over Probabilistic** — ✅ the recall, validation, and assembly are
  deterministic; the irreducibly-probabilistic part (the inference) is the model's, and the
  output is server-assembled from its structured fields.
- **VI. Capabilities Off By Default** — ✅ `elicit` adds **no new** network egress or code
  execution and **no new gate**. The only egress is the embedder call **when memory is
  already configured** — the existing memory capability, already gated on `VOYAGE_API_KEY`.
  Always in the catalog (FR-009).
- **VII. Simplicity and Scope Discipline** — ✅ one mode, one flat schema, one
  validate+zip+assemble function, one reused recall; enforcement and preference *storage*
  are explicitly out of scope; the recall limit is a named constant.

**Gate result**: PASS, no deviation requiring amendment. The scope boundary (no
enforcement) is named, per Principle I.

## Project Structure

### Documentation (this feature)

```text
specs/014-preference-elicitation/
├── plan.md          # This file
├── research.md      # Phase 0 — always-on + optional memory, server recall, flat parallel-array schema, assembly, testing
├── data-model.md    # Phase 1 — ElicitParams, ElicitPass (flat), GoverningPreference, DivergencePoint, ElicitResult, validate+zip
├── quickstart.md    # Phase 1 — what an elicit call returns (with and without memory)
├── contracts/
│   └── elicit.md    # the elicit tool input + per-pass schema + output
└── tasks.md         # Phase 2 — /speckit-tasks
```

### Source Code (repository root)

```text
src/modes/elicit.rs    # NEW — ELICIT_ID/DESCRIPTION; SignalLevel enum; PROMPT_TEMPLATE (<<task>>/<<context>>/<<preferences>>); ElicitParams {task, context}; flat ElicitPass {assumed_objective, preference_texts[], preference_signals[], preference_strengths[], divergence_questions[], divergence_signals[], signal_level}; run(client, mode, memory: Option<&MemoryDeps>, params, max) = optional recall → single pass → validate+zip+assemble → ElicitResult
src/modes/mod.rs       # MODIFIED — `pub mod elicit;`
src/server.rs          # MODIFIED — register elicit (always on) + #[tool] entry + elicit_with_ct passing self.memory.as_deref() through run_recorded; catalog assertions updated (elicit sorts after decide, before forget/grounded; count bump)
tests/integration.rs   # MODIFIED — 014 block: surfaces objective + traced prefs + divergence (mocked inference); recall reaches the prompt with a seeded memory; low-signal → empty; no enforcement field; one record
examples/acceptance_elicit.rs   # NEW — offline shape (with/without memory) + the live-dogfood scaffold
```

**Structure Decision**: a new single-pass mode, like `unstick`/`decide`, but with an
**optional** `MemoryDeps` argument threaded from the server's `memory` field — when present
the server recalls trusted preferences before the pass. It does **not** use
`verify::aggregate_core`. `verify`/`decide`/the memory tools are untouched; the recall path
reuses `memory::tools::recall` as-is.

## Complexity Tracking

*No constitution violation requiring justification.* Two notes: (1) per-item data is
carried as **parallel scalar arrays** (flat-schema requirement, as in `decide`); (2) the
mode takes an **optional** `MemoryDeps` — the first corrective mode to read memory — but it
reuses the existing `recall` function rather than introducing a new seam, so the
composition stays within the established pattern (recorded so the optional dependency is a
conscious design point, not drift).
