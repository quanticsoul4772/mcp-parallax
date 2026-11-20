# Implementation Plan: decide per-option fields

**Branch**: `056-decide-per-option-fields` | **Date**: 2026-11-20 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/056-decide-per-option-fields/spec.md`

## Summary

`decide`'s single pass returns scores and rationales as parallel arrays the
grammar cannot length-constrain, and 28 recorded calls failed on a length
mismatch. Replace them with one required score field and one required rationale
field per option position. Because the schema then depends on the option
count, `decide` registers one schema per accepted count (2 through 10) at
startup; each passes the existing flat-schema gate at boot, and each call sends
the schema for its count. Callers see the same result; the only new caller-
visible rule is the 10-option cap (FR-008), stated in the description.

## Technical Context

**Language/Version**: Rust 1.100.0 (pinned), MSRV 1.94

**Primary Dependencies**: existing only — `schemars`, `serde_json`, the mode registry (`src/modes/mod.rs`), the sanitizer and validator (`src/schema/`)

**Storage**: none changed; invocation records unchanged

**Testing**: `cargo test` with the existing `MockModelClient`; one live check through the rebuilt server

**Target Platform**: the MCP stdio server, both model backends (Anthropic, OpenAI-compatible)

**Project Type**: MCP server (library + binary)

**Performance Goals**: no regression; the provider compiles each distinct output format once and caches it, so at most nine formats are ever compiled for `decide`

**Constraints**: flat + closed schemas (Constitution II); no fallback or repair of malformed output; the schema-per-count must be fixed at startup so the gate runs at boot, not per call

**Scale/Scope**: one mode (`decide`), its contract, its 013 design documents; plus a small registry extension for per-count schemas

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Note |
|---|---|---|
| I. Design-Corpus Fidelity | Pass, with amendment | 013 research D1 chose parallel arrays; this reverses it. A dated amendment to 013 `research.md` and `data-model.md` ships in the same change (FR-007). |
| II. Constrained-Output Contract | Pass | Every per-count schema is flat and closed and goes through `assert_flat` at registration; nothing is generated per call. |
| III. Compiler-Enforced Discipline | Pass | No new lint exceptions; no `unwrap`/`expect` in production paths. |
| IV. Seams, Composition, Tests | Pass | Tests first through `MockModelClient`; no new external dependency. |
| V. Deterministic Over Probabilistic | Pass | Ranking and confidence stay server-side and unchanged. |
| VI. Capabilities Off By Default | N/A | No new capability. |
| VII. Simplicity and Scope Discipline | Pass | Nine precomputed schemas instead of a schema built per call; the registry change is additive. The billing-mislabel defect is out of scope. |

Re-check after Phase 1: unchanged; see research D1–D5.

## Project Structure

### Documentation (this feature)

```text
specs/056-decide-per-option-fields/
├── plan.md              # This file
├── research.md          # Phase 0 decisions D1–D5
├── data-model.md        # The per-count pass schema and what stays the same
├── quickstart.md        # How to verify, including the live check
├── contracts/
│   └── decide-pass.md   # The per-count model-facing pass format
├── checklists/
│   └── requirements.md
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
src/modes/mod.rs          # registry: per-count schema variants, each gated by assert_flat at boot
src/modes/decide.rs       # per-count schema builder, prompt wording, 10-option cap, parse by position
src/server.rs             # decide #[tool] description literal ("two to ten options")
specs/013-decide-methodology/contracts/decide.tool.json   # description copy, options field note
specs/013-decide-methodology/research.md                 # dated amendment
specs/013-decide-methodology/data-model.md               # dated amendment
tests/integration.rs      # existing decide catalog test still binds served = contract
CHANGELOG.md              # Fixed + Changed entries
```

**Structure Decision**: Keep the change inside `decide` plus an additive registry entry point. Other modes are untouched.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Registry gains per-count schema variants | The gate must run at boot for every schema `decide` can send (Constitution II) | Building the schema per call would move the flat check to request time and lose the boot-time guarantee; a second retry only reduces failures |
