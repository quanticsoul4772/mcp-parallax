# Tasks: Rerank fetched research pages

**Input**: Design documents from `specs/061-research-rerank/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/reranker.md, quickstart.md

**Tests**: required. Every behavior change gets a test written to fail first
(project rule). No test calls Voyage (FR-008).

**Organization**: by user story. US1 and US2 are P1, US3 is P2. New modules
are public from the start, so no step leaves unused code behind (060 lesson).

## Phase 1: Setup

None: no new dependency.

## Phase 2: Foundational (blocks every story)

- [X] T001 Write failing tests in `src/traits/rerank.rs` for the `Reranker` seam through its mock: `rerank(query, documents)` returns a `RerankResult` (a score per document in input order, total tokens) or an error
- [X] T002 Implement `Reranker` and `RerankResult` in `src/traits/rerank.rs`; export from `src/traits/mod.rs`
- [ ] T003 Write failing tests in a new public `src/research/rerank.rs` for `keep_top(scores, k)`: the indices of the top `k` scores in fetched order; ties keep fetched order; `k` at or above the count keeps every index; no scores keeps none
- [ ] T004 Implement `keep_top` in `src/research/rerank.rs` (pure)
- [ ] T005 Add `keep_sources` to the research `Tier` in `src/research/mod.rs` (quick 5, standard 15, deep 35), tests first: each below its tier's `max_sources`

**Checkpoint**: the seam, the selection and the keep counts exist; nothing calls them yet.

## Phase 3: User Story 1 — Research reads only the most relevant pages (P1)

**Goal**: with a reranker, only the top K pages reach extraction.

**Independent test**: with a reranker double and more pages than K, extraction runs on exactly the K highest-scoring pages.

- [ ] T006 [US1] Write failing wiremock tests for `VoyageClient` as a `Reranker` in `src/client/voyage.rs`: `POST /v1/rerank` with the bearer key and exactly `{query, documents, model: "rerank-3", truncation: true}`; reply indices mapped back to input order; `usage.total_tokens` returned; a missing, duplicate or out-of-range index, or a non-numeric score, is an error
- [ ] T007 [US1] Implement the rerank call and `impl Reranker for VoyageClient`, reusing the client's retry and backoff
- [ ] T008 [US1] Write failing pipeline tests in `src/research/pipeline_tests.rs`: with a mock reranker and more scorable pages than K, the extract client is called for exactly the K top pages, kept sources carry `relevance`, and `stats.sources_kept` is K; with K or fewer scorable pages the reranker is not called and every page is kept; with no reranker the result is unchanged (no `relevance`, `sources_kept` = pages extracted); empty pages are not sent and stay `Empty`
- [ ] T009 [US1] Implement the rerank step between contents and extraction in `src/research/pipeline.rs` (`ResearchDeps.reranker: Option<Arc<dyn Reranker>>`); add `Stats.sources_kept`, `Stats.rerank_skipped` and `SourceRef.relevance` in `src/research/contract.rs`; update `specs/004-research-layer/contracts/research.tool.json` so its binding tests pass
- [ ] T010 [US1] Wire the Voyage client as the reranker in `src/server.rs` when `VOYAGE_API_KEY` is set; an integration test in `tests/integration.rs` (Exa and Voyage doubles) shows `sources_kept` below `sources_found`

**Checkpoint**: US1 complete against doubles.

## Phase 4: User Story 2 — A failed rerank never fails the run (P1)

**Goal**: an error, a timeout past the run's deadline or a malformed reply leaves today's behavior and records why.

**Independent test**: with a failing reranker double, research completes with every page extracted and `stats.rerank_skipped` set.

- [ ] T011 [US2] Write failing pipeline tests: a reranker error, a reranker still running at the run's deadline, and a malformed reply each give a successful run with every scorable page extracted and `stats.rerank_skipped` naming the reason
- [ ] T012 [US2] Implement the fallback and the deadline bound on the rerank call in `src/research/pipeline.rs`

**Checkpoint**: US2 complete.

## Phase 5: User Story 3 — The cut is visible (P2)

**Goal**: rerank tokens reach the record's usage and cost.

**Independent test**: a reranked run's record lists `rerank-3` in `usage_by_model` with its tokens, priced at $0.05 per million.

- [ ] T013 [US3] Write failing tests: a reranked run adds the reply's tokens to the run's usage under `rerank-3` (input only); `cost_usd` includes them at the `rerank-3` row; a skipped rerank adds none; a research record carries `pages_fetched` and `pages_kept` (nullable, research only; analyze A2), round-trips through SQLite and is exported as span attributes
- [ ] T014 [US3] Implement the accounting in `src/research/pipeline.rs`, the `rerank-3` price row and the `pages_fetched`/`pages_kept` record fields in `src/telemetry.rs`, their nullable columns in `src/storage/sqlite.rs`, span attributes in `src/observability.rs`, and the record and telemetry contracts

**Checkpoint**: all stories pass against doubles.

## Phase 6: Polish & Cross-Cutting

- [ ] T015 README research section, `CHANGELOG.md` `[Unreleased]` entry, and a dated `docs/design/SDK_LANDSCAPE.md` amendment naming `rerank-3` (exact text of each approved first)
- [ ] T016 Independent review: a fresh subagent given only the diff, told to trace every caller (brief approved first); findings asked one by one
- [ ] T017 Live acceptance (quickstart.md): `examples/acceptance_research.rs` gains backend selection (`PARALLAX_BACKEND=claude_cli` runs its model calls through the operator's Claude Code, so no API credit is needed; analyze A1); then run it at standard depth on the same questions with and without `VOYAGE_API_KEY`, compare pages extracted (SC-001, ≥ 30% fewer) and confirm grounded answers citing only kept sources (SC-002); record the results
- [ ] T018 Update the CLAUDE.md active-feature block for 061 (exact text approved first)

## Dependencies & Execution Order

- Foundational (T001–T005) before every story; T001 → T002, T003 → T004.
- US1 (T006–T010) before US2 and US3; T006 → T007, T008 → T009 → T010.
- US2 (T011–T012) and US3 (T013–T014) are independent after US1.
- Polish after all stories; T016 before T017; T018 last.

## Parallel Opportunities

- T001–T002, T003–T004 and T005 touch different files.
- After US1, US2 and US3 can proceed in parallel.

## Implementation Strategy

MVP is US1: research extracts from fewer, more relevant pages. US2 keeps the
rerank from ever costing an answer; US3 makes its tokens visible. Each story
is committed with the gate green before the next starts.
