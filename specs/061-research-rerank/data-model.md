# Data Model: Rerank fetched research pages

## Tier (changed)

| Depth | max_sources (fetched) | keep_sources (new) |
|---|---|---|
| quick | 8 | 5 |
| standard | 25 | 15 |
| deep | 60 | 35 |

## RerankResult (new, in memory)

| Field | Type | Notes |
|---|---|---|
| scores | `Vec<f32>` | one per input document, in input order |
| total_tokens | `u64` | the provider's `usage.total_tokens` |

## Research result (changed, caller-facing)

| Field | Type | Notes |
|---|---|---|
| `stats.sources_kept` | integer | pages that reached extraction |
| `stats.rerank_skipped` | string, optional | why a configured rerank did not run: `provider_error: …`, `timeout`, `malformed_reply: …`; absent when it ran, when there was nothing to cut, or with no reranker configured |
| `sources[].relevance` | number, optional | the page's rerank score; absent when no rerank ran |

## Usage and price (changed)

`usage_by_model` gains a `rerank-3` entry (input tokens only) on runs that
reranked; price row `rerank-3`: $0.05 input, $0 output per million tokens.
