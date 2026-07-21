# Implementation Plan: Preference Enforcement at the Checkpoint

**Branch**: `015-preference-enforcement` | **Date**: 2026-07-21 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/015-preference-enforcement/spec.md`

## Summary

Close the enforcement gap named in `PREFERENCE_ELICITATION.md`: the end-of-turn
checkpoint (`checkpoint_turn`) already recalls stored memories and runs the
layer's single blind review hop — but that hop judges only *self-contradiction*
("both statements cannot be true as written"), so a turn that violates a stored
preference ("never use word X"; "always run the test gate before claiming
done") passes silently: a banned word does not textually contradict the ban.
This feature teaches the **same single hop** a second judgment — *violation* —
over the same recalled trusted-memory population the gate already treats as
constraints. A confirmed violation returns a **flag** (never hold, never
rewrite) quoting the stored preference verbatim with its provenance (memory id plus
trust standing), under a new `preference_violation` signal kind, cooled down
by memory id. No new tools, no new config, no storage schema change, no new
dependencies; memory-unconfigured behavior is byte-identical to today.

## Technical Context

**Language/Version**: Rust, pinned stable via `rust-toolchain.toml`, MSRV 1.94

**Primary Dependencies**: existing only — rmcp (stdio MCP), tokio, serde/schemars (flat+closed hop schema), mockall (seam tests). No additions.

**Storage**: existing SQLite `checkpoint_records` — no schema change (`signals_evaluated`/`signals_fired` already carry the new kind as JSON)

**Testing**: `cargo test` through the trait seams (`MockModelClient`, `MockEmbedder`, `MockStorage`, `MockTrajectoryReader`); integration tests in `tests/integration.rs`; live dogfood per quickstart

**Target Platform**: the existing server binary (Windows/Linux), stdio transport

**Project Type**: single Rust crate — extension of `src/checkpoint/`

**Performance Goals**: unchanged — end-of-turn runs at Stop-hook time (not the gate's 500 ms critical path); still exactly one embed call + at most one model hop per turn (FR-010)

**Constraints**: fail-open (FR-007); flat+closed hop schema (Constitution II); decline-biased judgment (FR-004); cooldown suppression via existing `delivered_keys` feed (30-min window); bounded transcript window (existing `WINDOW_ENTRIES`/`WINDOW_BYTES`)

**Scale/Scope**: recall over the full memory store (existing brute-force cosine); candidates capped by existing `REVIEW_CANDIDATES_MAX`-style cap; ~4 source files touched + tests + 2 contract files

## Constitution Check

*GATE: evaluated against Constitution 1.0.0 before Phase 0; re-checked after Phase 1.*

| Principle | Verdict | Notes |
|---|---|---|
| I. Design-Corpus Fidelity | PASS | Direct delivery of `PREFERENCE_ELICITATION.md`'s capture→store→recall→**enforce** loop and the watchdog amendment's enforcement role. The corpus's open question (block vs flag-and-revise) is resolved to flag-only; `PREFERENCE_ELICITATION.md` is amended in the same change (research D10). No crate-stack deviation. |
| II. Constrained-Output Contract | PASS | The review hop's output schema is extended, stays flat + closed (`additionalProperties: false`), registered through the same boot-time invariant check. No free-text parsing. |
| III. Compiler-Enforced Discipline | PASS | No new lint exceptions; stderr-only tracing; errors read-and-fixed, fail-open is the layer's specced contract (spec FR-007), not an error-hiding wrapper. |
| IV. Seams, Composition, Tests | PASS | All new logic sits behind the existing seams (`ModelClient`, `Embedder`, `Storage`, `TrajectoryReader`, `TimeProvider`); tests required and planned per user story; no live credentials needed. |
| V. Deterministic Over Probabilistic | PASS | Candidate mining, provenance mapping, cooldown identity, flag wording, and verdict assembly are all pure/deterministic; the LLM judges only the one thing a solver cannot — whether natural-language turn content violates a natural-language preference. |
| VI. Capabilities Off By Default | PASS | No new capability: no new egress, no new env var. Enforcement activates only inside the existing double opt-in (memory configured via `VOYAGE_API_KEY` + checkpoint hooks installed). |
| VII. Simplicity & Scope | PASS with named split | `review.rs` is already 516 lines; the extension lands in a new `src/checkpoint/preference.rs` (mining, identity, flag assembly) so no module crosses further past the 500-line target. Full agreed scope built; capture and hold-tier remain named out-of-scope per spec Assumptions. |

**Post-Phase-1 re-check**: PASS — design artifacts introduce no new violations; the hop schema in `contracts/review.hop.json` is flat and closed; no storage migration.

## Project Structure

### Documentation (this feature)

```text
specs/015-preference-enforcement/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── checkpoint_turn.tool.json   # updated turn contract (new signal kind)
│   └── review.hop.json             # the extended constrained-output hop schema + fixed templates
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
src/
├── checkpoint/
│   ├── mod.rs           # + SignalKind::PreferenceViolation (wire form "preference_violation")
│   ├── preference.rs    # NEW: preference-candidate mining, cooldown identity, violation flag assembly (pure)
│   ├── review.rs        # extended hop: ReviewOut violation fields, prompt section, provenance map-back
│   ├── run.rs           # run_turn: conditional evaluated_kinds, wire preference candidates, combined delivery
│   ├── contract.rs      # unchanged (tool inputs unchanged); result signals enum widens implicitly
│   ├── gate.rs          # unchanged — `is_constraint` reused as the candidate population
│   └── screen.rs        # unchanged
├── server.rs / server/  # unchanged (tool surface identical)
└── storage/             # unchanged (record columns already fit)

tests/
└── integration.rs       # + end-to-end turn scenarios (violation flag, memory-off equivalence)

docs/design/
└── PREFERENCE_ELICITATION.md  # same-change amendment: enforce-half shipped, flag-only authority (D10)
```

**Structure Decision**: single-crate extension of the existing `src/checkpoint/`
module tree; one new file (`preference.rs`) keeps `review.rs` under control per
Principle VII. No new binaries, tools, or layers.

## Complexity Tracking

No Constitution violations to justify. The one borderline item —
`review.rs` at 516 lines pre-change — is resolved by the named
`preference.rs` split rather than justified as an overage.
