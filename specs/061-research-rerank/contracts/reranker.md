# Contract: one rerank call

## Request

`POST {VOYAGE_BASE}/v1/rerank`, header `Authorization: Bearer <VOYAGE_API_KEY>`.

```json
{
  "query": "<the research question>",
  "documents": ["<page 1 text>", "<page 2 text>", "..."],
  "model": "rerank-3",
  "truncation": true
}
```

Only pages with non-empty text, each already trimmed to the per-source cap,
in fetched order. Sent only when there are more such pages than the depth's
keep count.

## Reply

```json
{"data": [{"index": 0, "relevance_score": 0.81}, ...], "usage": {"total_tokens": 12345}}
```

Every input index exactly once → a score per page in input order. Anything
else (missing, duplicate or out-of-range index, non-numeric score) is a
malformed reply.

## Selection

The top `keep_sources` pages by score go to extraction; ties keep fetched
order. The rest are neither extracted nor cited.

## Failure

Error, timeout or malformed reply → no rerank: every page goes to
extraction, and `stats.rerank_skipped` carries the reason. Never a run
failure.
