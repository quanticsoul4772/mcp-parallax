# Quickstart: Rerank fetched research pages

## Turn it on

Set `VOYAGE_API_KEY` (it also enables the memory tools). Nothing else.

## Check it works

1. Run `research` at standard depth.
2. In the result, `stats.sources_kept` is at most 15 while
   `stats.sources_found` may be higher, each source carries `relevance`, and
   `stats.rerank_skipped` is absent.
3. The invocation record's `usage_by_model` includes `rerank-3`.

## Live acceptance

`cargo run --example acceptance_research` on the same questions with and
without `VOYAGE_API_KEY`: compare extraction calls per run at standard depth
(SC-001, ≥ 30% fewer) and confirm every answer cites only kept sources and
passes the grounding gate (SC-002). Results are recorded below after the run.
