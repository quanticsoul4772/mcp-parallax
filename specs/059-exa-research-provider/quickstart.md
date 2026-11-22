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
