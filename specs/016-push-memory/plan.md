# Implementation Plan: Push Memory

**Branch**: `016-push-memory` | **Date**: 2026-07-23 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/016-push-memory/spec.md`

## Summary

Add the push half of the memory layer: a new harness-triggered, memory-gated
tool **`surface`** that the installable integration's `UserPromptSubmit` hook
calls at each turn start with the session id and the user's prompt. The server
embeds a bounded prompt excerpt, ranks stored memories with the existing pull
ranking (cosine + recency + trust), filters to trusted, applies a relevance
floor and a small cap, subtracts memories already surfaced this session
(derived from the feature's own audit rows — no new in-process state), and
returns a fixed-template advisory context block (content verbatim plus memory
id and trust) through the hook's `additionalContext` mapping. Deterministic
end-to-end — zero model passes (spec FR-010) — under a hard 500 ms budget
with fail-open silence. One new audit table mirrored to OTLP at the same exit
point (007 pattern). Delivery precondition: an **S2 spike** live-verifies the
`UserPromptSubmit` mcp_tool payload and `additionalContext` mapping, which S1
never covered.

## Technical Context

**Language/Version**: Rust, pinned stable via `rust-toolchain.toml`, MSRV 1.94

**Primary Dependencies**: existing only — rmcp, tokio (timeout budget), serde/schemars, mockall. No additions; no model client involvement.

**Storage**: SQLite — one additive table (`push_records`); no changes to existing tables

**Testing**: `cargo test` through the seams (`MockEmbedder`, `MockStorage`, `MockTimeProvider`); integration scenarios in `tests/integration.rs` (serve_with_memory + wiremock embeddings); live dogfood per quickstart after the S2 spike

**Target Platform**: existing server binary, stdio; sensor plane is the Claude Code hooks integration (client-specific, like 006)

**Project Type**: single-crate extension — `src/memory/` + `server.rs` + `integrations/claude-code/`

**Performance Goals**: hard `PUSH_BUDGET_MS = 500` (clarification Q2) — one embed call + in-process ranking, the gate boundary's proven workload

**Constraints**: fail-open (FR-007); deterministic selection, no model pass (FR-010); trusted-only (FR-004); once-per-session suppression (FR-005, clarification Q3); advisory labeling, never instruction-phrased (FR-002); memory-off/integration-absent byte-identical (FR-006)

**Scale/Scope**: ranking over the full store (existing brute-force cosine, budget-bounded); ~2 new source files + seam/storage/server/integration touches + tests + 1 contract

## Constitution Check

*GATE: evaluated against Constitution 1.0.0 before Phase 0; re-checked after Phase 1.*

| Principle | Verdict | Notes |
|---|---|---|
| I. Design-Corpus Fidelity | PASS | Direct delivery of `MEMORY_LAYER.md`'s push contract ("effortless, not manual" — the layer's named fix for dead manual recall). `MEMORY_LAYER.md` amended in-change to record the push half shipped (research D11). Auto-capture exclusion traces to the corpus's own write-path warning, decided via the clarify protocol. |
| II. Constrained-Output Contract | PASS (vacuous for hops) | No model pass exists (FR-010), so no hop schema; the tool's structured result is a typed `Json<T>` like every tool. |
| III. Compiler-Enforced Discipline | PASS | No new lint exceptions; stderr-only tracing; fail-open is the specced contract, not error-hiding. |
| IV. Seams, Composition, Tests | PASS | `Embedder`/`Storage`/`TimeProvider` seams only; two new `Storage` methods mocked like the rest; tests per user story, no network/disk. |
| V. Deterministic Over Probabilistic | PASS | The whole feature is the deterministic path: ranking, threshold, cap, suppression, and template assembly are pure; no judge anywhere. |
| VI. Capabilities Off By Default | PASS | No new env var and no new egress: embeddings are already gated on `VOYAGE_API_KEY`, and context injection activates only when the operator installs the hooks integration — the same explicit opt-in posture as 006. |
| VII. Simplicity & Scope | PASS | Push-only scope (clarify Q1); new pure module `src/memory/push.rs` well under the 500-line target; constants over config knobs (research D9). |

**Post-Phase-1 re-check**: PASS — no violations introduced; no Complexity Tracking entries.

## Project Structure

### Documentation (this feature)

```text
specs/016-push-memory/
├── plan.md              # This file
├── research.md          # Phase 0 output (D1–D11)
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output (incl. S2 spike protocol)
├── contracts/
│   └── surface.tool.json    # the new tool's contract (input, result, hook mapping)
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
src/
├── memory/
│   ├── push.rs          # NEW: selection (rank → trusted → floor → cap → suppression-subtract),
│   │                    #      fixed advisory template, run() orchestration under the budget
│   └── mod.rs           # + pub mod push
├── traits/storage.rs    # + record_push, + pushed_memory_ids(session_id) (mocked)
├── storage/sqlite.rs    # + push_records table (additive CREATE TABLE) + the two impls
├── server.rs            # + `surface` tool (memory-gated catalog, run_recorded, hook mapping on result)
├── observability.rs     # + emit_push — records mirrored to OTLP at the same exit point (007)
integrations/claude-code/
└── hooks.json           # + UserPromptSubmit → surface entry (final shape set by the S2 spike)
docs/design/
└── MEMORY_LAYER.md      # same-change amendment: push half shipped (research D11)
tests/integration.rs     # + surface end-to-end scenarios (serve_with_memory)
```

**Structure Decision**: memory-family placement (`src/memory/push.rs`), not
checkpoint — push delivers stored knowledge, it judges nothing. The harness
triggering is a delivery property recorded in the tool description, mirroring
the checkpoint tools' wording.

## Complexity Tracking

No Constitution violations to justify. The one external risk — the unverified
`UserPromptSubmit` hook mapping — is handled as a named spike precondition
(S2), not absorbed as complexity.
