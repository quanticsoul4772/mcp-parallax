# Implementation Plan: Per-Call Reasoning Effort

**Branch**: `028-per-call-effort-argument` | **Date**: 2026-07-26 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/028-per-call-effort-argument/spec.md`

## Summary

Make three operator-only settings reachable by the caller for one invocation:
reasoning **effort** (on the seven correctives), **pass count** (on `verify`,
`diverge`, `grounded_verify`), and research **concurrency** (in `research`'s
existing `constraints`). A fourth candidate, `MEMORY_RECALL_LIMIT`, is already
per-call and becomes a confirming test plus a corpus citation.

The central design question — how a per-call effort reaches the HTTP client when
the client pool keys on `(model, effort)` and is built once at startup — is
resolved in research D1 by a **third option the spec did not name**: pre-build
every `(routed model, level)` combination eagerly at startup over one shared HTTP
client. The trait seam is untouched (018 D2 upheld), nothing is allocated per
call, and no lock or interior mutability is introduced.

## Technical Context

**Language/Version**: Rust, stable pinned via `rust-toolchain.toml`; MSRV 1.94

**Primary Dependencies**: `rmcp` (MCP stdio + tool schemas), `reqwest`,
`serde`/`schemars`, `sqlx` (SQLite), `tokio`, `mockall` (seam mocks),
`wiremock` (HTTP-level tests)

**Storage**: SQLite — `invocation_records` gains one nullable `effort` column via
the additive, pragma-guarded `ALTER TABLE` pattern already used for `depth`
(019), `src/storage/sqlite.rs:275`

**Testing**: `cargo test` through the trait seams; wiremock for wire-shape
assertions; no network, no live credentials

**Target Platform**: stdio MCP server, Windows and Linux

**Project Type**: single Rust binary + library

**Performance Goals**: no per-call allocation on either the default or the
override path; the eager client set is bounded at (distinct routed models) × 6
effort states — at most 72, realistically under 12

**Constraints**: stdout is the JSON-RPC channel (tracing to stderr only); mode
schemas flat and closed; no `unwrap`/`expect` in production paths

**Scale/Scope**: 12 call sites, 7 tools gaining an effort argument, 3 gaining a
pass-count argument, 1 gaining a concurrency constraint

## Constitution Check

*GATE: evaluated before Phase 0 research; re-evaluated after Phase 1 design.*

| Principle | Assessment | Verdict |
| --- | --- | --- |
| **I. Design-Corpus Fidelity** | This feature touches a corpus decision: 018 research D2 decided against giving the completion seam a per-call parameter. D1 upholds that decision rather than overturning it, so no deviation is taken. Separately the corpus must gain the operator-owned vs caller-owned test (FR-009) and cite `recall`'s existing `limit` as prior art (FR-016) — both additions made in the same change. | PASS |
| **II. Constrained-Output Contract** | The effort argument is an *input* property; no output schema changes for it, since FR-007 puts it on the record instead. The pass count is added to three *output* schemas and stays flat and closed — a scalar, no nesting. | PASS |
| **III. Compiler-Enforced Discipline** | An unrecognised caller value is a typed rejection (FR-006), never a silent fallback. The ceilings (FR-012a, FR-015) cap a value the caller is explicitly permitted to lower; they hide no failure. D3 records why this is a specified bound rather than graceful degradation, because a clamp can be mistaken for one. | PASS |
| **IV. Seams, Composition, Tests** | The `ModelClient` seam is untouched by D1, so every existing mock keeps compiling. Tests are required per FR and enumerated in Phase 1. | PASS |
| **V. Deterministic Over Probabilistic** | Resolution order, ceiling enforcement, and record-writing are pure functions over inputs. No model judgment is involved in any of them. | PASS |
| **VI. Capabilities Off By Default** | Adds no capability and no egress. Every new argument is optional; omitting all of them leaves outbound requests byte-identical (FR-003, SC-002). The one capability-adjacent surface — research concurrency — is bounded so a caller can only *reduce* egress. | PASS |
| **VII. Simplicity and Scope Discipline** | Scope is the four settings the spec names and nothing else. D1 is chosen partly for being the smallest of the three candidates. `src/client/pool.rs` grows and stays well under the 500-line target. | PASS |

No entry requires a Complexity Tracking justification.

## Project Structure

### Documentation (this feature)

```text
specs/028-per-call-effort-argument/
├── plan.md              # This file
├── spec.md              # Feature specification
├── research.md          # Phase 0 — the four resolved decisions
├── data-model.md        # Phase 1 — entities, resolution, migration
├── quickstart.md        # Phase 1 — how a caller uses it
├── contracts/
│   └── tool-surface.md  # Phase 1 — the input/output contract per tool
├── checklists/
│   └── requirements.md  # Spec quality checklist (all passing)
└── tasks.md             # Phase 2 — NOT created by /speckit-plan
```

### Source Code (repository root)

```text
src/
├── client/
│   ├── pool.rs          # D1: eager (model, level) set; for_site_with_effort()
│   └── anthropic.rs     # shared reqwest client constructor
├── routing.rs           # Effort already lives here; gains the override resolver
├── modes/
│   ├── mod.rs           # ensemble_k becomes a per-run override
│   ├── verify.rs        # pass count in, count-used out
│   ├── diverge.rs       # same
│   └── grounded_verify.rs  # same
├── research/
│   └── pipeline.rs      # concurrency from constraints, clamped
├── server.rs            # tool params -> overrides; record the effort used
├── storage/sqlite.rs    # additive `effort` column (depth pattern)
└── telemetry.rs         # InvocationRecord.effort
tests/
└── integration.rs       # T013 re-grounded (FR-008)
```

**Structure Decision**: no new modules. Every change lands in a file that already
owns the concern, which is why the ≤500-line target is not threatened.

## Phase 0: Research

Complete — see [research.md](research.md). Four decisions:

- **D1**: how a per-call effort reaches the client — **eager pre-build over a
  shared HTTP client**, rejecting both options the spec named.
- **D2**: how a per-call pass count reaches a mode — override at run, not at
  registration.
- **D3**: how the concurrency ceiling is enforced — clamp at the boundary, with
  the clamp made visible.
- **D4**: how the effort reaches the record — additive column, `depth` pattern.

No NEEDS CLARIFICATION items remain: the spec's two were resolved in
`/speckit-clarify`, and the plan-level fork is resolved by D1.

## Phase 1: Design & Contracts

Complete — [data-model.md](data-model.md),
[contracts/tool-surface.md](contracts/tool-surface.md),
[quickstart.md](quickstart.md).

### Constitution re-check after design

Re-evaluated against the Phase 1 artifacts: all seven principles still PASS. The
design adds one output field to three schemas (Principle II — scalar, flat,
closed) and one nullable column (Principle IV — additive, backward-readable).
Neither introduces a new gate concern.

One risk is named rather than designed away: **D3's clamp is the only place in
this feature where a caller-supplied value is altered rather than honoured or
rejected.** It is surfaced on the record so the caller's value and the effective
value are both recoverable. If review judges the clamp too close to Principle
III's prohibition, the alternative is a typed rejection; D3 states the tradeoff
rather than hiding the choice.

## Complexity Tracking

No principle violations requiring justification.
