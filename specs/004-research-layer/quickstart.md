# Quickstart: Research Layer

## Enable

Research is off by default. Set the credential (and optionally the knobs):

| Variable | Default | Notes |
|---|---|---|
| `BRAVE_API_KEY` | unset | presence enables the `research` tool |
| `FETCH_TIMEOUT_MS` | `10000` | per-source fetch timeout |
| `RESEARCH_CONCURRENCY` | `8` | concurrent fetch/extract/verify cap (1..=32) |
| `INPUT_MAX_CHARS` | `50000` | bounds the question (existing) |

Without the key the server is byte-for-byte the 003 server: no `research` in
the catalog, zero research egress.

## Use

```json
// quick look
{ "question": "What changed in sqlx 0.9's SQLite driver?", "depth": "quick" }

// standard (default depth)
{ "question": "Is brute-force cosine over a few thousand embeddings competitive with vector indexes?" }

// deep, constrained
{
  "question": "What are the tradeoffs between Brave, Tavily, and Exa as search APIs for agents?",
  "depth": "deep",
  "constraints": { "max_sources": 30, "domains_deny": ["pinterest.com"], "deadline_ms": 300000 }
}
```

The answer cites sources inline as `[s3]`; every id resolves in `sources`.
`stopped_early: true` + `stop_reason` means a ceiling was hit and the answer
covers only what was verified in time — the unfinished parts are in `gaps`.

## Spikes (before the pipeline)

```bash
cargo run --example spike_extract     # S1: rs-trafilatura vs bundled HTML fixtures (offline)
cargo run --example spike_brave      # S2: one live Brave request, asserts response shape
```

## Acceptance (live; needs BRAVE_API_KEY + ANTHROPIC_API_KEY)

```bash
cargo run --release --example acceptance_research
```

At least 6 live questions (SC-001 zero fabricated citations, SC-003 latency),
a tiny-ceiling run (SC-004 stopped-early honesty), and a false-premise
question (SC-007). Results recorded below when run.

### Status (2026-06-12)

**Blocked on credential**: the configured `BRAVE_API_KEY` is rejected by the
Brave Search API (`SUBSCRIPTION_TOKEN_INVALID`) — confirmed against the live
endpoint and via the local brave-search MCP, which fails identically. The
acceptance example and spike S2 are written and compile; issue a fresh key at
the Brave Search API dashboard, then run both and record results here.

## Inspect

```bash
sqlite3 ./data/parallax.db "SELECT tool, outcome, latency_ms, cost_usd FROM invocation_records WHERE tool = 'research' ORDER BY created_at DESC LIMIT 10;"
```
