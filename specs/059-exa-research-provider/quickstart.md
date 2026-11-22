# Quickstart: research on Exa

## Configure

Set `EXA_API_KEY` in the server's environment and remove
`FETCH_ALLOW_PRIVATE` and `FETCH_TIMEOUT_MS` if present (each is now a startup
error). `BRAVE_API_KEY` can stay if other tools use it: with `EXA_API_KEY`
set, Parallax ignores it. Optionally set `EXA_MAX_AGE_HOURS` (default 168; 0 forces a live
crawl, -1 uses Exa's cache only).

## Check the startup errors

Start the server with `BRAVE_API_KEY` set and no `EXA_API_KEY`: it exits
with an error naming both. With both set, it starts.

## Live acceptance (before the PR)

```bash
ANTHROPIC_API_KEY=... EXA_API_KEY=... cargo run -j 2 --release --example acceptance_research
```

Run three times. Each run must return at least one supported finding with
sources (SC-001). Record, per run: depth, findings, sources, any truncated
sources, Exa cost, elapsed time. Run once at `deep` to measure `deep-lite`'s
latency and cost against `standard` (research.md D2).

## Check the record

After a run, the latest `research` row in `invocation_records` has a
non-null `search_cost_usd` (when Exa reported costs), and with OTLP export
configured the `parallax.research` span carries the same value as
`parallax.search.cost_usd`.

## Live acceptance results (2026-11-22)

Debug build, `EXA_API_KEY` and `ANTHROPIC_API_KEY`, default model
`claude-opus-4-8`. Two attempts at the planned three runs each stopped
partway when the Anthropic credit balance ran out (HTTP 400 "credit balance
is too low") — an account limit, not a code error. Nine questions passed
across the two attempts, every citation resolving:

| Question | Depth | Time | Sources | Findings | Exa cost |
|---|---|---|---|---|---|
| GitHub default branch protection | quick | 77 s / 72 s | 8 / 8 | 90 / 81 | $0.029 / $0.029 |
| Voyage AI embedding families | quick | 81 s / 76 s | 8 / 8 | 84 / 86 | $0.029 / $0.029 |
| Brave Search free-tier limits | quick | 68 s / 67 s | 7 / 7 | 75 / 75 | $0.029 / $0.029 |
| SQLite WAL vs rollback | standard | 176 s / 184 s | 24 / 24 | 270 / 274 | $0.059 / $0.059 |
| MCP and its transports | standard | 149 s | 25 | 263 | $0.060 |

Quick runs stayed under 150 s and standard under 240 s (SC-003). Exa cost
about $0.03 per quick and $0.06 per standard question; the Anthropic model
calls used about $20 of credit in under ten minutes of runs. Not measured:
the deep question (`deep-lite` latency and cost, research.md D2), the
false-premise check, the tiny-budget check, and truncated-source counts
(the example does not print them).
