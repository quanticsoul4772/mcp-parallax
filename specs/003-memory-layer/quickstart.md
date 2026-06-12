# Quickstart: Memory Layer

## Enable

Memory is off by default. Set the credential (and optionally the model):

| Variable | Default | Notes |
|---|---|---|
| `VOYAGE_API_KEY` | unset | presence enables save/recall/forget |
| `VOYAGE_MODEL` | `voyage-4` | stay within the voyage-4 family (shared space) |
| `MEMORY_RECALL_LIMIT` | `5` | default top-k (max 20) |
| `INPUT_MAX_CHARS` | `50000` | generic input bound (`VERIFY_MAX_CLAIM_CHARS` honored as alias) |

Without the key the server is byte-for-byte the 002 server: two tools, no
Voyage connection.

## Use

```json
// save (first-hand)
{ "content": "For CI-only test failures, export the CI env vars locally first - it isolates environmental diffs in one run.", "kind": "skill", "origin": "debugged in session 2026-06-11", "external": false }

// save (external, with verification)
{ "content": "sqlx's after_connect hook runs for every pooled connection.", "kind": "fact", "origin": "sqlx docs", "external": true, "verify": true }

// recall
{ "query": "how do I debug tests that only fail on CI", "kind": "skill" }

// forget
{ "id": "<memory id>" }
```

## Spike (no key needed)

```bash
cargo run --example spike_bruteforce   # blob round-trip + scoring timing at 5k x 1024
```

## Acceptance (live; needs VOYAGE_API_KEY + ANTHROPIC_API_KEY)

```bash
cargo run --example acceptance_memory
```

12 saves + 10 paraphrased recall queries (SC-001 precision), trust scenarios
(SC-003), latency (SC-004). Results recorded below when run.

### Results (2026-06-11, voyage-4 + claude-opus-4-8, release build)

| Criterion | Target | Result |
|---|---|---|
| SC-001 top-3 | ≥ 9/10 | **10/10** |
| SC-001 top-1 | ≥ 7/10 | **10/10** |
| SC-002 structure | 100% | 100% (typed structs end to end) |
| SC-003 unverified external labeled untrusted | yes | yes |
| SC-003 refuted external save rejected with findings | yes | yes (Apollo 11 1972 claim refuted, 3 findings) |
| SC-004 max recall latency | < 5000 ms | **237 ms** |
| SC-004 max save latency (no verification) | < 10000 ms | **430 ms** |

Verdict: **PASS** — every paraphrased query ranked its intended memory first.

## Inspect

```bash
sqlite3 ./data/parallax.db "SELECT kind, trust, substr(content,1,60) FROM memories ORDER BY created_at DESC LIMIT 10;"
sqlite3 ./data/parallax.db "SELECT tool, outcome, latency_ms FROM invocation_records WHERE tool IN ('save','recall','forget') ORDER BY created_at DESC LIMIT 10;"
```
