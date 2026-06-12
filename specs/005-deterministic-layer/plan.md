# Implementation Plan: Deterministic Layer — Checkable Claims Settled by Execution

**Branch**: `005-deterministic-layer` | **Date**: 2026-06-12 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/005-deterministic-layer/spec.md`

## Summary

One new MCP tool, **`check`** — always in the catalog (no new credential: the
engines are pure and in-process, so Constitution VI requires no gate). One
constrained model call classifies checkability and translates the claim into a
small typed formal target (flat+closed schema covering both engine families);
a deterministic engine executes it — **evalexpr** for boolean-valued
arithmetic comparisons, **Z3** (bundled) for constraint satisfiability with
witnesses; a real engine violation triggers exactly one violation-fed
re-translation; verdict mapping and the plain-language explanation are
**server-assembled** (the model never decides or phrases the verdict — the
004 D7 pattern, applied harder). Engine timeout maps to the existing `timeout`
class; double translation failure to `validation_failure`; "not checkable" is
a successful result with a stated reason and a decline-biased classifier.

## Technical Context

**Language/Version**: Rust (pinned stable via `rust-toolchain.toml`, MSRV 1.94)

**Primary Dependencies**: existing stack + new: `z3` 0.20 (`bundled` —
vendored build, no system install), `evalexpr` 13 — see research.md D1/D2

**Storage**: existing SQLite via the `Storage` seam (invocation records only)

**Testing**: cargo test — MockModelClient for translation, engines tested
directly with ground-truth tables (no mocking a solver), in-process rmcp for
integration; live acceptance example (translation quality is the live
question; engines are deterministic)

**Target Platform**: cross-platform stdio binary (Windows dev, Linux CI) —
the z3 bundled build is the platform risk; spike S1 gates it

**Performance Goals**: SC-007 determinism; solver bounded by a 10 s
in-engine timeout; arithmetic effectively instant (expression length bounded)

**Constraints**: verdicts only from execution (FR-002); auditability — every
response carries the formal form + raw engine result (FR-007); no new env
vars (FR-010); engines have zero effects beyond the process (FR-006)

**Scale/Scope**: two engine families; formal targets deliberately small
(boolean arithmetic comparison; linear-arithmetic SMT-LIB 2 constraints over
int/real/bool); everything else named-deferred (FR-011)

## Constitution Check

*GATE: evaluated against constitution v1.0.0 before Phase 0; re-checked after
Phase 1 design.*

| Principle | Status | Notes |
|---|---|---|
| I. Design-corpus fidelity | PASS | Maps to `DETERMINISTIC_LAYER.md` (translate→execute→feed-back, decline bias, auditability) + `SDK_LANDSCAPE.md` §deterministic (z3 0.20 the named pick; "meval/fend-style" evaluator → evalexpr, named in research.md D2 with fend-core as the unit-aware upgrade path). Named deferrals: PAL code-exec (needs the off-by-default sandbox), CAS, planners, round-trip checking, formalization ensembles (spec FR-011/Assumptions). |
| II. Constrained-output contract | PASS | One model hop with a flat+closed schema (`{checkable, reason?, engine?, arithmetic_expression?, smtlib_constraints?, asserted?}` — nullable scalars, enums; no nesting). The wire result is MCP-side. |
| III. Compiler discipline | PASS | z3/evalexpr are in-process; no stdout; no unsafe in our code (`z3-sys` FFI lives inside the dependency — `forbid(unsafe_code)` governs this crate, dependencies keep their own guarantees, same as sqlx/reqwest). |
| IV. Seams + tests | PASS | Translation through the existing `ModelClient` seam. Engines are deterministic pure functions — tested directly against ground-truth tables, not mocked (mocking a solver would test nothing). |
| V. Deterministic over probabilistic | PASS (the layer IS this principle) | Verdict mapping, witness extraction, and explanation text are pure functions of engine output; the model never emits a verdict. |
| VI. Capabilities off by default | PASS | No new capability: zero effects beyond the process (no network, no filesystem, no exec). The tool is always on, like `verify`/`unstick`. Code execution stays deferred until a sandbox exists. |
| VII. Simplicity / ≤500-line modules | PASS | Module split below; formal targets minimal (YAGNI — expressiveness grows only with evidence). |

**Post-Phase-1 re-check**: PASS — the contract introduces no new violations.

## Project Structure

### Documentation (this feature)

```text
specs/005-deterministic-layer/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── check.tool.json
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── deterministic/
│   ├── mod.rs            # constants (timeouts, length bounds), shared types
│   ├── contract.rs       # wire types: CheckParams/CheckResult (MCP-side)
│   ├── translate.rs      # the classify+translate mode: prompt, flat schema, registration, violation-fed retry
│   ├── arithmetic.rs     # evalexpr engine wrapper (pure: expression → bool | violation)
│   ├── solver.rs         # z3 engine wrapper (SMT-LIB 2 in → sat/unsat/unknown + witness)
│   └── check.rs          # orchestration: validate → translate → execute → server-assembled result
├── server.rs             # + check #[tool] via run_recorded (always in catalog)
└── error.rs              # (no change — timeout/validation_failure classes reused)

tests/integration.rs      # + catalog presence, ground-truth round trip, failure parity
examples/spike_z3.rs      # S1: bundled build + trivial sat/unsat (gates everything)
examples/acceptance_check.rs  # live acceptance (translation quality; ANTHROPIC_API_KEY only)
```

**Structure Decision**: single crate, new `deterministic/` module family
mirroring `research/` (contract split, prompts-in-translate, pure engine
wrappers). No config or error-taxonomy changes: engine timeout reuses the
`timeout` class (message names the solver), double translation failure
reuses `validation_failure` (message names translation) — both messages
distinct per the existing class-naming convention.

## Complexity Tracking

No constitution violations to justify. Named engineering risk: the z3
`bundled` feature vendors a C++ build of Z3 — spike S1 measures build time
on Windows and CI before anything depends on it; CI's rust-cache amortizes
it to cache misses. If the build proves unworkable, the named fallback is
shipping the arithmetic engine first and the solver behind a feature flag —
that would be a recorded deviation, decided at spike time, not silently.
