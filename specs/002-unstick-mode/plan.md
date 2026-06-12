# Implementation Plan: Unstick Mode — Second Corrective on the Registry

**Branch**: `002-unstick-mode` | **Date**: 2026-06-12 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/002-unstick-mode/spec.md`

## Summary

Add the second corrective, `unstick`, as a registry entry: one new mode module
(types + calibrated prompt + single-pass run), one new `#[tool]` method on the
existing server handler, one new contract file. The guarded-invocation pattern
(single-exit recording, ct-select cancellation) is extracted from the verify
tool into a shared helper so both tools use it without duplication. No new
dependencies, no new environment variables, no changes to the schema pipeline,
client, storage, or error taxonomy — that absence is the feature's
architecture proof (FR-008/SC-006).

## Technical Context

**Language/Version**: Rust, edition 2021, MSRV 1.94 (unchanged)

**Primary Dependencies**: none added — rmcp 1.7.0, schemars, jsonschema, sqlx,
reqwest, tokio-util all already in place

**Storage**: existing `invocation_records` table; `tool` column now takes the
value `unstick` as well (schema unchanged)

**Testing**: same stack — mockall `ModelClient` for unit tests, in-process
rmcp client + wiremock for integration, all without network/disk state

**Target Platform / Project Type**: unchanged (single Rust crate, MCP stdio)

**Performance Goals**: single Unstick call < 15 s at defaults (SC-004) —
single pass, so roughly one-third of verify's budget

**Constraints**: output schema flat + closed (registry boot assertion); one
generation pass per invocation (FR-007); existing test suite must pass with
zero modified assertions (SC-006)

**Scale/Scope**: one new mode module (~150 lines + tests), one tool method,
one small server refactor (extract the record-guard wrapper), one contract
file, one acceptance example

## Constitution Check

| Principle | Gate | Status |
|---|---|---|
| I. Design-corpus fidelity | Unstick is the Step primitive (`NEW_SERVER_DESIGN.md` §4: "stuck/looping → externalized structured step"); layer-1 self-invoked corrective; the #1 organic tool in the usage data; no stack changes | ✅ PASS |
| II. Constrained-output contract | Same pipeline: schemars-derived per-mode schema → sanitizer → grammar; local validator + code-level semantic checks (non-empty step, no repeat of tried items); flat + closed enforced at boot | ✅ PASS |
| III. Compiler-enforced discipline | No new lint exceptions; no stdout; `Result`-based throughout | ✅ PASS |
| IV. Seams, composition, tests | Same three seams, no new ones needed; every story has test tasks; suite runs without network or disk | ✅ PASS |
| V. Deterministic over probabilistic | Step quality is not mechanically checkable (stays with the model); structural rules (shape, repeats) checked deterministically in code, not by a judge | ✅ PASS |
| VI. Capabilities off by default | No new capabilities, no new egress, no new env vars | ✅ PASS |
| VII. Simplicity and scope | The whole point: mode #2 as a data addition. The only refactor is extracting the existing guard pattern to avoid duplicating it | ✅ PASS |

**Post-Phase-1 re-check**: PASS — design artifacts add one mode + one contract;
nothing else moves.

## Project Structure

### Documentation (this feature)

```text
specs/002-unstick-mode/
├── plan.md              # This file
├── research.md          # Phase 0 — three decisions, no unknowns
├── data-model.md        # Phase 1 — UnstickParams / NextStep / registry entry
├── quickstart.md        # Phase 1 — invoke + acceptance
├── contracts/
│   └── unstick.tool.json
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── modes/
│   ├── mod.rs           # unchanged (registry already generic)
│   ├── verify.rs        # unchanged behavior; uses the extracted wrapper via server.rs
│   └── unstick.rs       # NEW: UnstickParams, NextStep, prompt template, single-pass run
├── server.rs            # + #[tool] unstick method; extract run_recorded() guard wrapper
└── (everything else)    # untouched — that's the point

tests/integration.rs     # + catalog lists both tools; unstick round-trip; failure class parity
examples/
└── acceptance_unstick.rs  # T-last: 10-scenario live acceptance (manual-run)
```

**Structure Decision**: `unstick.rs` mirrors `verify.rs`'s layout (constants,
params, output type, register, run, tests) minus the ensemble machinery. The
`run_recorded` extraction in `server.rs` is the one shared-code change; it is
behavior-preserving for verify and covered by SC-006 (existing tests unchanged).

## Complexity Tracking

> No Constitution Check violations — table intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| — | — | — |
