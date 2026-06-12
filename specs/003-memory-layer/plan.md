# Implementation Plan: Memory Layer — Recall Corrective with Verified-Before-Stored Memory

**Branch**: `003-memory-layer` | **Date**: 2026-06-12 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/003-memory-layer/spec.md`

## Summary

Three new tools (`save`, `recall`, `forget`) backed by a `memories` table in
the existing SQLite store, with embeddings from a thin Voyage client behind a
new `Embedder` seam. Recall ranks by cosine relevance + recency, with trust as
a tier (first-hand/verified ≥ untrusted). The poisoning defense: external
provenance is admitted as trusted only through the existing verify ensemble at
save time. The whole capability is gated on `VOYAGE_API_KEY` — absent it, the
tools are not in the catalog and no Voyage connection ever happens.

**One named stack deviation (Constitution I)**: v1 scores similarity by
**brute-force in-process cosine over BLOB embeddings**, not sqlite-vec.
`SDK_LANDSCAPE.md` picked sqlite-vec, with the sqlx-loading caveat to spike
first; the spike (research.md S1) shows registration requires either an
`unsafe` FFI call (the crate forbids unsafe; isolating it means a workspace
split) or shipping per-platform loadable-extension binaries — while at v1
scale (≤ 10k memories × 1024 dims) brute force is single-digit milliseconds.
sqlite-vec remains the named scale path; the landscape doc is amended in this
change.

## Technical Context

**Language/Version**: Rust, edition 2021, MSRV 1.94 (unchanged)

**Primary Dependencies**: one addition — none. The Voyage client reuses
`reqwest`; embeddings are stored as BLOBs via existing `sqlx`; cosine is plain
Rust. (sqlite-vec deliberately not added — see deviation above.)

**Storage**: new `memories` table (id, content, kind, origin, external, trust,
tags, created_at, embedding BLOB) in the existing SQLite file; idempotent
migration extends the existing one

**Testing**: same stack — a `MockEmbedder` via mockall for unit tests,
wiremock for the Voyage client, in-process rmcp + wiremock for integration;
no network/disk state

**New seam**: `Embedder` trait (`embed_document`, `embed_query` → `Vec<f32>` +
usage) — Voyage distinguishes document vs query input types and retrieval
quality depends on using them correctly

**Target Platform / Project Type**: unchanged

**Performance Goals**: recall < 5 s (SC-004; one query embedding + in-process
scoring), save without verification < 10 s (one document embedding); scoring
itself < 50 ms at 5k memories (spike-validated)

**Constraints**: capability off without `VOYAGE_API_KEY` (FR-007 — catalog
filtering is the implementation risk; see research.md D5); memory tool output
schemas are MCP-side only (no model hop → the grammar subset and the
registry's flat invariant do not apply; recall output nests a memory array);
existing suite must pass unchanged with the key absent (SC-005)

**Scale/Scope**: single-operator store, thousands of memories; new modules
`src/memory/` (store, ranking, tools logic), `src/client/voyage.rs`,
`src/traits/embedder.rs`; config gains `VOYAGE_API_KEY`, `VOYAGE_MODEL`,
`MEMORY_RECALL_LIMIT`, and the generic `INPUT_MAX_CHARS` (paying the named
naming debt from 002 — `VERIFY_MAX_CLAIM_CHARS` honored as a fallback alias)

## Constitution Check

| Principle | Gate | Status |
|---|---|---|
| I. Design-corpus fidelity | Memory is the Recall corrective + §F/§G of the landscape; verified-before-stored is `MEMORY_LAYER.md`'s central move; pull-only and no-consolidation are named deferrals grounded in the corpus (push needs the watchdog; importance/merge need per-write LLM passes). The sqlite-vec deviation is named, justified, spiked, and the landscape doc is amended in the same change | ✅ PASS |
| II. Constrained-output contract | No new model-hop schemas except verify-at-save, which reuses the existing verify mode unchanged. MCP-side output schemas declared in contracts/ and validated in tests; the grammar subset governs model hops only — documented in research.md D6 | ✅ PASS |
| III. Compiler-enforced discipline | No unsafe (the brute-force decision exists partly to keep `forbid(unsafe_code)` intact); no new lint exceptions | ✅ PASS |
| IV. Seams, composition, tests | New `Embedder` seam, mocked; Voyage client wiremock-tested; storage tests on in-memory SQLite; every story has test tasks | ✅ PASS |
| V. Deterministic over probabilistic | Ranking is deterministic math over embeddings; trust tiering is deterministic; the only LLM judgment is the existing verify ensemble at save time (exactly where the corpus puts it) | ✅ PASS |
| VI. Capabilities off by default | New egress (Voyage) enabled only by its credential; absent → no tools, no connections (FR-007/SC-005) | ✅ PASS |
| VII. Simplicity and scope | Brute-force over a vector extension at v1 scale is the YAGNI call; no consolidation machinery; forget is a DELETE | ✅ PASS |

**Post-Phase-1 re-check**: PASS.

## Project Structure

### Documentation (this feature)

```text
specs/003-memory-layer/
├── plan.md, research.md, data-model.md, quickstart.md, tasks.md
└── contracts/
    ├── save.tool.json
    ├── recall.tool.json
    └── forget.tool.json
```

### Source Code (repository root)

```text
src/
├── traits/embedder.rs    # NEW seam: embed_document / embed_query
├── client/voyage.rs      # NEW thin Voyage client (reqwest, wiremock-tested)
├── memory/
│   ├── mod.rs            # types: Memory, Kind, Trust, Provenance
│   ├── store.rs          # memories table CRUD on the existing SqliteStorage pool
│   ├── ranking.rs        # cosine + recency + trust tiering (pure, heavily tested)
│   └── tools.rs          # save/recall/forget logic incl. verify-at-save
├── config.rs             # + VOYAGE_API_KEY/VOYAGE_MODEL/MEMORY_RECALL_LIMIT/INPUT_MAX_CHARS
├── server.rs             # + three #[tool] methods via run_recorded; catalog gating
└── error.rs              # + EmbeddingProvider failure class

examples/
├── spike_bruteforce.rs   # S1: blob round-trip + scoring timing at 5k×1024 (no key)
└── acceptance_memory.rs  # live acceptance (key required)
tests/integration.rs      # + gating, round-trip, trust, forget, parity tests
```

**Structure Decision**: memory is not a registry mode (no prompt template, no
model hop) — it gets its own module tree beside `modes/`, sharing the seams
and `run_recorded`. Verify-at-save calls `verify::run` through the existing
registry entry.

## Complexity Tracking

> No Constitution Check violations — table intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| — | — | — |
