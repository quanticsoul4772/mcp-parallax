# Implementation Plan: Memory Consolidation and Auto-Capture

**Branch**: `017-memory-consolidation` | **Date**: 2026-07-23 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/017-memory-consolidation/spec.md`

## Summary

The write-path half of the memory layer, on the three clarify-decided rails:
**supersession and merge evaluated on admission** (a deterministic cosine
screen gates one new budgeted, decline-biased model judgment — the
screen-gates-judge pattern the checkpoint layer proved), **decay as
ranking-only** (a reinforcement-refreshed recency term; nothing is ever
removed or hidden), and **auto-capture harness-triggered at end of turn**
(the existing `checkpoint_turn` hop gains a third judgment — capture
proposal — so the boundary keeps its single model pass; candidates enter
untrusted, which the shipped push layer already refuses to surface).
Memories gain a `status` dimension (active | superseded | merged) — the
project's **first additive column migration** — every retrieval path filters
to active, inspection lists everything, stored content is never modified,
and a new `consolidation_records` audit table records every action. No new
tools, no new env vars; candidate promotion reuses existing paths
(re-admission merges the trusted save as canonical) rather than adding
surface.

## Technical Context

**Language/Version**: Rust, pinned stable, MSRV 1.94

**Primary Dependencies**: existing only — rmcp, tokio, serde/schemars, sqlx, mockall. No additions.

**Storage**: SQLite — `memories` gains `status`, `replaced_by`, `last_reinforced_at` via pragma-guarded `ALTER TABLE` (first column migration; the `CREATE TABLE IF NOT EXISTS` block alone no longer suffices); new additive `consolidation_records` table

**Testing**: seam mocks + wiremock integration as established; migration tests against a pre-017 database file; live dogfood per quickstart

**Target Platform**: existing server binary; capture rides the installed Stop-hook sensor plane (no integration change — `checkpoint_turn`'s input is unchanged)

**Project Type**: single-crate extension — `src/memory/` + `src/checkpoint/` + storage

**Performance Goals**: admission consolidation adds at most one screened model pass to `save` (which already runs verify-at-save optionally); capture adds zero model passes (extends the existing turn hop); reinforcement writes are fire-and-forget

**Constraints**: decline bias on every judgment (uncertain ⇒ no action); byte-identical survivors (FR-004); ranking-only decay (FR-005); candidates never auto-trusted (FR-007); content never modified in place (FR-010); fail-open capture (FR-008); memory-off unchanged (FR-012)

**Scale/Scope**: pairwise screens over the small brute-force store; ~6 source files touched + migration + tests; 1 new mode schema, 1 extended hop schema

## Constitution Check

*GATE: evaluated against Constitution 1.0.0 before Phase 0; re-checked after Phase 1.*

| Principle | Verdict | Notes |
|---|---|---|
| I. Design-Corpus Fidelity | PASS | Delivers `MEMORY_LAYER.md`'s write path (capture → consolidate) with its two traps encoded as requirements; the doc is amended in-change (research D10). All three clarify decisions trace to corpus lessons (manual-dependence, no-scheduler architecture, eviction-as-compliance). |
| II. Constrained-Output Contract | PASS | The new consolidation judgment is a registered mode with a flat+closed schema; the turn hop's capture extension keeps `ReviewOut` flat+closed. |
| III. Compiler-Enforced Discipline | PASS | No new lint exceptions; migration errors are loud; fail-open only where the spec contracts it (capture), never to hide errors. |
| IV. Seams, Composition, Tests | PASS | All effects behind existing seams; migration tested against a fixture DB; every story has fail-first seam tests. |
| V. Deterministic Over Probabilistic | PASS | Cosine screens, status filtering, decay math, caps, and audit are pure/deterministic; the two semantic judgments (update-vs-context / same-assertion, capture-worthiness) are named, budgeted, decline-biased model passes invoked only when a deterministic screen fires. |
| VI. Capabilities Off By Default | PASS | No new capability: consolidation rides admission (memory-gated already); capture rides the installed opt-in sensor plane and is inactive with memory off. |
| VII. Simplicity & Scope | PASS with named splits | New `src/memory/consolidate.rs` (pure screens + apply) and the judgment mode; capture lands in the existing checkpoint hop rather than new machinery; no promote tool (re-admission covers it). Scope cuts named in spec Assumptions. |

**Post-Phase-1 re-check**: PASS — no new violations; the first-migration risk is handled as a tested pattern, not complexity.

## Project Structure

### Documentation (this feature)

```text
specs/017-memory-consolidation/
├── plan.md              # This file
├── research.md          # Phase 0 (D1–D10)
├── data-model.md        # Phase 1 — status lifecycle, tables, schemas
├── quickstart.md        # Phase 1 — gate, migration check, live dogfood
├── contracts/
│   ├── consolidation.hop.json   # the admission judgment's constrained output
│   └── review.hop.json          # the turn hop extended with capture fields (supersedes 016's copy of 015's)
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── memory/
│   ├── mod.rs           # + Status enum; Memory gains status, replaced_by, last_reinforced_at
│   ├── consolidate.rs   # NEW: cosine screens (MERGE_TAU / SUPERSEDE_SCREEN_TAU), judgment mode
│   │                    #      registration, pure apply (supersede/merge), audit assembly
│   ├── tools.rs         # save path: screen → judgment → apply → audit (after admission)
│   ├── push.rs          # active-only filtering + reinforcement on surfacing
│   └── ranking.rs       # recency term reads last_reinforced_at (reinforcement-refreshed decay)
├── checkpoint/
│   ├── review.rs        # ReviewOut + prompt gain the capture judgment (third judgment, same hop)
│   └── run.rs           # run_turn: store capped, quarantined candidates; audit; memory-gated
├── traits/storage.rs    # + record_consolidation, list-side helpers, captures_in_session
├── storage/sqlite.rs    # pragma-guarded ALTER TABLE migration + consolidation_records + impls
├── observability.rs     # + emit_consolidation (007 dual-sink)
docs/design/MEMORY_LAYER.md  # same-change amendment: write path shipped (research D10)
tests/integration.rs     # + admission-consolidation, capture, migration-compat scenarios
```

**Structure Decision**: consolidation is memory-family (`consolidate.rs`);
capture is delivered by the checkpoint boundary but stores through the memory
seams — the hop extension mirrors exactly how 015 added its second judgment.

## Complexity Tracking

No Constitution violations to justify. Two named risks handled structurally:
the **first ALTER TABLE migration** (pragma-guarded, tested against a
pre-017 fixture DB, loud on failure) and the **turn hop's third judgment**
(schema stays flat+closed; if the hop's prompt shows measured quality
degradation from crowding, splitting the boundary into two passes is the
named fallback — a deviation that would need its own justification then).
