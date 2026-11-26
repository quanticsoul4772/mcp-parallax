# Research: Rerank fetched research pages

Sources: Voyage docs via Context7 (`docs.voyageai.com`: reranker API,
pricing, rate limits), read 2026-11-23; `src/research/pipeline.rs`.
Decisions scored with Parallax `decide` (neutral input) and chosen by the
operator.

## D1 — Where the rerank goes

**Decision**: after the contents call returns page text, before per-page
extraction (Parallax 82; title-only before fetching 42, both 66).
**Rationale**: extraction is the per-page model call, so the cut lands where
the usage is; page text is a far stronger signal than a title.

## D2 — Model

**Decision**: `rerank-3` (Parallax 85; `rerank-3-lite` 70; `rerank-2.5` 45).
**Rationale**: the provider's pricing page lists `rerank-3` as current
($0.05 per million tokens, 200 million free per account) and `rerank-2.5`
under older models with no free tokens. The spec was amended to name it.

## D3 — Keep counts

**Decision**: quick 5 of 8, standard 15 of 25, deep 35 of 60 (Parallax 85;
6/17/42 scored 60; 4/10/25 scored 55).
**Rationale**: about 40% fewer extraction calls, a margin over SC-001's 30%,
without thinning sources.

## D4 — The seam and the request

**Decision**: a `Reranker` trait, `rerank(query, documents) -> RerankResult`
(a score per input document, in input order, plus total tokens),
implemented by `VoyageClient` as `POST /v1/rerank` with
`{"query", "documents", "model": "rerank-3", "truncation": true}` and no
`top_k`, so every document comes back scored. The reply's `index` fields
map scores back to input order; a missing or duplicate index is a malformed
reply.
**Limits** (rerank-3 family): up to 1,000 documents; query ≤ 8,000 tokens;
query + any one document ≤ 32,000; total (query × documents + all document
tokens) ≤ 600,000. A deep run of 60 pages at the 16,000-character cap is
about 240,000 tokens. `truncation: true` lets the provider trim an outlier
page instead of failing.

## D5 — Gating

**Decision**: the reranker is present exactly when `VOYAGE_API_KEY` is set,
using the same `VoyageClient` as the memory tools. No new setting.

## D6 — Failure

**Decision**: an error, a timeout past the run's deadline, or a malformed
reply leaves the pages unreranked: every fetched page goes to extraction, as
before this feature, and `stats.rerank_skipped` names the reason. The run
never fails on rerank (FR-005, 2026-11-23).
**Retries**: the client's existing Voyage retry and backoff apply; the call
is additionally bounded by the run's remaining time.

## D7 — Which pages are scored

**Decision**: only pages with non-empty text are sent; pages without text
are already dropped as `Empty` today and stay so. Ties keep fetched order
(stable sort). With no more scorable pages than K, the rerank call is skipped
entirely (nothing to cut) and every page is kept.

## D8 — Accounting and output

**Decision**: rerank tokens are added to the run's usage under model
`rerank-3` (input only), priced by a new `rerank-3` row ($0.05 per million),
so they appear in `usage_by_model` and `cost_usd`, beside the CLI figure on
`claude_cli` (060 review finding 5). The result's `stats` gain
`sources_kept` and `rerank_skipped`; each source gains `relevance` (the
score, absent when no rerank ran). The invocation record also carries
`pages_fetched` and `pages_kept` (nullable, research only) so the cut is
queryable after the result is gone (FR-006; analyze A2).
