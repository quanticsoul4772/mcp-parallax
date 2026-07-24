# Implementation Plan: Per-Hop Model Routing

**Branch**: `018-model-routing` | **Date**: 2026-07-24 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/018-model-routing/spec.md`

## Summary

Let each of the server's twelve model call sites run on a model chosen for the work
it does, instead of all twelve sharing one `ANTHROPIC_MODEL`. The lever is cost:
research's per-source claim extraction runs once per fetched page and dominates a
run's tokens while doing mechanical transcription, and it is the one call site where
a cheaper model changes the bill materially.

The approach falls out of two facts already true in the tree. The model is a property
of the client instance, not a parameter of `ModelClient::complete` — so routing is a
construction-time concern and the trait never changes. And `run_recorded` already
threads a hand-picked attributed model per tool, because `surface` and
`checkpoint_action` already attribute to Voyage while everything else attributes to
Anthropic — so multi-model invocations exist today and this feature generalizes an
existing hand-rolled pattern rather than inventing one.

Three pieces: a pure resolution layer over a reserved `PARALLAX_MODEL_*` namespace
(two tiers, per-site overrides, most-specific-first); a per-model usage accumulator
replacing the `(u64, u64)` token pair so cost can be summed at each model's own rate;
and additive record and telemetry surfaces that stay byte-identical when nothing is
routed.

## Technical Context

**Language/Version**: Rust, MSRV 1.94 (pinned in `rust-toolchain.toml`)

**Primary Dependencies**: no new crates. Existing `rmcp`, `reqwest`, `sqlx`,
`opentelemetry`, `tracing`, `serde_json`.

**Storage**: SQLite. Two nullable `TEXT` columns added to `invocation_records` via
the pragma-guarded `ALTER TABLE` pattern established by 017.

**Testing**: `cargo test` through the existing trait seams (`MockModelClient`,
`MockTimeProvider`, `MockStorage`); no network, no disk.

**Target Platform**: MCP server over stdio; Windows and Linux.

**Project Type**: single Rust binary + library.

**Performance Goals**: no request-path cost. Route resolution and client-pool
construction happen once at startup; per call the change is one `Arc` deref that
already existed.

**Constraints**: stdout is the JSON-RPC channel — diagnostics go to stderr only.
Routing must be invisible to callers (no tool schema changes). Unrouted behavior must
be byte-identical, including recorded costs.

**Scale/Scope**: 12 call sites, 2 tiers, 14 environment variables, 2 new modules
(`src/routing.rs`, `src/client/pool.rs`), 9 modified (`config.rs`, `server.rs`,
`telemetry.rs`, `observability.rs`, `storage/sqlite.rs`, `client/anthropic.rs`,
`checkpoint/run.rs`, `research/mod.rs`, `research/pipeline.rs`).

## Constitution Check

*GATE: must pass before Phase 0. Re-checked after Phase 1 — see below.*

| Principle | Assessment |
|---|---|
| **I. Design-Corpus Fidelity** | Routing is not in the corpus today. `SDK_LANDSCAPE.md` §core describes a single `ModelClient` behind one configured model. **The corpus MUST be amended in this change** — a task, not a follow-up. One named deferral is recorded in research D7 (per-family `thinking` suppression), justified and scoped, not slipped in. |
| **II. Constrained-Output Contract** | No mode schema changes; no new model hop. Every existing schema stays flat and closed. Untouched. |
| **III. Compiler-Enforced Discipline** | The startup routing table is `tracing` to stderr. No new `unwrap`/`expect` on production paths; config validation returns `ConfigError`. Loud failure, no fallback that hides a misconfiguration. |
| **IV. Seams, Composition, Tests** | `ModelClient` keeps its exact signature (research D2), so no mock or seam changes. Routing resolution is a pure function testable without a client at all. Tests required for every FR. |
| **V. Deterministic Over Probabilistic** | Route resolution, dominance, and cost are pure computation. Dominance is deliberately computed from measured tokens rather than estimated cost (D5) so attribution never depends on an estimate. |
| **VI. Capabilities Off By Default** | Unset means today's behavior exactly. Routing adds no egress the server did not already have — it redirects existing calls. |
| **VII. Simplicity and Scope Discipline** | Two tiers, not three. No trait change. Additive columns, not a new table. One new module, well under 500 lines. |

**Result: PASS, with one obligation** — the corpus amendment under Principle I is a
task in this feature, not deferred work.

**Post-Phase-1 re-check**: still PASS. The design added no dependency, no trait
change, and no tool-surface change. The only contract touched is 007's telemetry
document, amended in-change per Principle I, and the amendment is additive: every
instrument is byte-identical in the single-model case.

## Project Structure

### Documentation (this feature)

```text
specs/018-model-routing/
├── plan.md              # This file
├── spec.md              # Feature specification (clarified)
├── research.md          # Phase 0 — D1..D9
├── data-model.md        # Phase 1 — call sites, usage, record, telemetry
├── quickstart.md        # Phase 1 — operator walkthrough
├── contracts/
│   └── config.md        # Phase 1 — the PARALLAX_MODEL_* namespace
├── checklists/
│   └── requirements.md  # Spec quality checklist
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
src/
├── routing.rs           # NEW — CallSite, Tier, resolution, RoutingTable, validation
├── client/pool.rs       # NEW — one client per distinct resolved model (D10)
├── config.rs            # MODIFIED — parse + validate the PARALLAX_MODEL_* namespace;
│                        #   REQUEST_TIMEOUT_MS default raised with the budget (D7)
├── server.rs            # MODIFIED — call the pool; per-site Arcs into the deps
│                        #   structs; run_recorded takes ModelUsage; startup table
├── telemetry.rs         # MODIFIED — ModelUsage, per-model cost, pricing rows,
│                        #   attributed model, cost_estimated
├── observability.rs     # MODIFIED — parallax.models, parallax.cost_estimated,
│                        #   per-model cost + token metrics
├── storage/sqlite.rs    # MODIFIED — two nullable columns + pragma-guarded migration
├── client/anthropic.rs  # MODIFIED — output budget raised (D7)
├── checkpoint/run.rs    # MODIFIED — cost call updated for pricing_known
└── research/
    ├── mod.rs           # MODIFIED — RunMeter accumulates per model
    └── pipeline.rs      # MODIFIED — four per-call-site clients in ResearchDeps

docs/design/             # MODIFIED — corpus amendment (Principle I obligation)
specs/007-observability-layer/contracts/telemetry.md   # MODIFIED — amendment
```

**Structure Decision**: single project, existing layout, **two** new modules and nine
modified. Routing gets its own module because it is a pure, self-contained concern with
a wide test surface. The client pool gets a second one (D10) rather than living in
either neighbour: putting it in `routing.rs` would drag a concrete client into the
module whose value is not having one, and putting it in `server.rs` would add to a file
already at 1397 lines — which is what `/speckit-analyze` flagged. Everything else is a
modification to the module that already owns the responsibility. `ResearchDeps` gains
per-call-site client fields rather than a resolver, keeping the dependency explicit at
each use.

## Implementation Sequence

Ordered so each step is independently testable and the risky work lands last.

1. **Routing module** (`src/routing.rs`) — call sites, tiers, resolution,
   namespace validation. Pure; fully unit-testable with no client, no I/O.
2. **Config integration** — parse and validate at startup, loud on unknown suffix or
   empty value. Covers FR-006, FR-006a, SC-005.
3. **Client pool + wiring** — one client per distinct model, per-site `Arc`s into the
   deps structs, startup routing table. Covers FR-001, FR-004, FR-005.
4. **Per-model usage** — `ModelUsage`, `RunMeter` accumulation, `run_recorded`
   signature. Covers FR-007. The single-model helper keeps eleven call sites one-line.
5. **Record and cost** — pricing rows, `pricing_known`, attributed model, summed cost,
   the two columns and their migration. Covers FR-008..FR-012, FR-009a.
6. **Telemetry amendment** — span attributes, per-model metric recording, and the 007
   contract edit. Covers FR-010.
7. **Request shape** — output budget and timeout defaults. Covers FR-013, FR-014.
8. **Corpus amendment** — Principle I obligation.

Steps 1–3 deliver User Story 1 (P1) end to end and are shippable alone. Steps 4–6 are
User Story 2 (P2). Step 7 is User Story 3 (P3).

## Risks

| Risk | Mitigation |
|---|---|
| The `run_recorded` signature change touches all twelve call sites at once | Eleven are mechanical (`ModelUsage::single`). Land step 4 as its own commit with the suite green before step 5 changes any semantics. |
| A byte-identical-when-unrouted claim is easy to assert and hard to prove | SC-004 is tested directly: existing record and telemetry tests keep their current expected values, unmodified. If an expectation needs editing, the invariant broke. |
| Raising the output budget without measuring | D7's measurement procedure gives the budget a deterministic floor (the largest mode schema bounds its own output at ~3.5k tokens) and an empirical acceptance test (zero truncation and zero timeout outcomes across the T050 family sweep). Provisional values are set, then validated by the sweep and raised if it fails — an iteration, not a guess. |
| Fable 5 rejects `thinking: disabled`, Opus 5 accepts it | Resolved by sending one universally-accepted shape (omit the field). The cheaper per-family suppression is a **named deferral** (D7), to be decided on measured cost. |
| Provider price drift silently under-reporting | `pricing_known` surfaces the fallback in both the record and the span, so "correct by lookup" and "correct by coincidence" stop looking alike. |

## Complexity Tracking

No Constitution Check violations. Table intentionally empty.
