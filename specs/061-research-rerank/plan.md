# Implementation Plan: Rerank fetched research pages

**Branch**: `061-research-rerank` | **Date**: 2026-11-23 | **Spec**: [spec.md](spec.md)

## Summary

After research's one contents call returns the kept sources' text, a new
`Reranker` seam scores each page against the question with Voyage
`rerank-3`, and only the top K pages (5 / 15 / 35 by depth) go on to
claim extraction, the step that costs one model call per page. The reranker
is the existing `VoyageClient` (new `POST /v1/rerank` method), present when
`VOYAGE_API_KEY` is set. A failed rerank leaves today's behavior in place and
is recorded in the result's stats (named exception). Rerank
tokens are accounted like any other model's, under `rerank-3`, so they reach
the record's usage and cost.

## Technical Context

**Language/Version**: Rust (toolchain 1.100.0, MSRV 1.94)
**Primary Dependencies**: existing crates only (reqwest, serde, tokio)
**Storage**: none new (usage flows through the existing `usage_by_model`)
**Testing**: `cargo test`; a `MockReranker` seam double; wiremock for the
Voyage endpoint
**Performance Goals**: SC-001, ≥ 30% fewer extraction calls at standard depth
(planned: 15 of 25 kept, 40%)
**Constraints**: the rerank call is bounded by the run's deadline; one
request per run (limits below are far above a deep run)
**Scale/Scope**: one new seam (~60 lines), a client method, one pipeline step,
result-contract fields

## Constitution Check

| Principle | Status | Note |
|---|---|---|
| I. Design-corpus fidelity | Amend | SDK_LANDSCAPE names `rerank-2.5`; the provider now lists `rerank-3` as current (pricing page, 2026-11-23). A dated amendment records the change. |
| II. Constrained output | N/A | No model-facing schema or prompt changes. |
| III. Compiler discipline | Deviation, named | FR-005's fallback is the named exception to "no fallbacks": rerank only trims work, so a skipped rerank costs usage, never correctness, and the skip is recorded, not hidden. |
| IV. Seams and tests | Pass | `Reranker` trait with a mock; Voyage endpoint tested through wiremock. |
| V. Deterministic over probabilistic | Pass | Selection is a deterministic top-K over scores, ties in fetched order. |
| VI. Off by default | Pass | Gated on `VOYAGE_API_KEY`, which already gates memory. |
| VII. Simplicity | Pass | No new crate; pure top-K in a small module. |

## Project Structure

```text
specs/061-research-rerank/
├── spec.md, plan.md, research.md, data-model.md, quickstart.md
├── contracts/reranker.md
└── checklists/requirements.md

src/
├── traits/rerank.rs        # NEW: Reranker seam + RerankResult
├── client/voyage.rs        # + rerank(): POST /v1/rerank, model rerank-3
├── research/rerank.rs      # NEW: pure keep_top(scores, k) with stable ties
├── research/mod.rs         # Tier gains keep_sources (5/15/35)
├── research/pipeline.rs    # rerank step between contents and extraction
├── research/contract.rs    # Stats.sources_kept, Stats.rerank_skipped,
│                           # SourceRef.relevance
├── server.rs               # wire the Voyage client as the reranker
├── telemetry.rs            # price row: rerank-3; record pages_fetched/kept
├── storage/sqlite.rs       # two nullable record columns
└── observability.rs        # span attributes for the page counts
examples/acceptance_research.rs  # backend selection (claude_cli) for the live run
specs/004-research-layer/contracts/research.tool.json   # output fields
```
