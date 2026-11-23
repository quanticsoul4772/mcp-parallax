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

`PARALLAX_BACKEND=claude_cli cargo run --example acceptance_research` (on
the operator's subscription; no API credit) on the same questions with and
without `VOYAGE_API_KEY`: compare extraction calls per run at standard depth
(SC-001, ≥ 30% fewer) and confirm every answer cites only kept sources and
passes the grounding gate (SC-002). Results are recorded below after the run.

### Results (2026-11-23)

Both runs: `PARALLAX_BACKEND=claude_cli`, the example's seven questions
(3 quick, 2 standard, 1 deep, 1 false premise), default model
`claude-sonnet-5-5` at every call site.

| Question | Depth | Pages read, no rerank | Pages read, rerank | Time, no rerank | Time, rerank |
|---|---|---|---|---|---|
| GitHub branch protection | quick | 8 | 5 | 130 s | 96 s |
| Voyage model families | quick | 8 | 5 | 169 s | 112 s |
| Brave free-tier limits | quick | 8 | 5 | 142 s | 127 s |
| SQLite WAL vs rollback | standard | 24 | 15 | 290 s | 302 s |
| MCP transports | standard | 25 | 15 | 288 s | 286 s |
| Tokio vs async-std | deep | 60 | 35 | 541 s | 541 s |
| Rust 1.0 false premise | quick | 8 | 5 | 148 s | 106 s |

- **SC-001 pass:** 141 → 85 pages read, 40% fewer; standard depth alone
  49 → 30, 39% fewer.
- **SC-002 pass:** 7/7 answers grounded in both runs.
- Every reranked run recorded `rerank_skipped None`. The baseline was run with
  an invalid Voyage key (HTTP 401), so it also showed the US2 fallback live:
  every run completed, each with `provider_error: …` recorded.
- False premise not confirmed and the tiny budget stopped cleanly in both
  runs.
- **Not caused by 061:** the example's standard-depth time limit (240 s)
  fails in both runs (290 s / 302 s), and 4 of 7 runs stopped at their
  deadline in each. This is research speed on the claude_cli backend,
  to be taken up separately.
