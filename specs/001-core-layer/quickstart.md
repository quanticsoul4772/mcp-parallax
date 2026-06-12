# Quickstart: Core Layer (Verify)

How to build, configure, run, and exercise the server once this feature is
implemented. Doubles as the manual acceptance path for the spec's success
criteria.

## Build & test

```bash
cargo build
cargo test                 # all tests run without network or disk state
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test   # full gate
```

## Configure

| Variable | Required | Default |
|---|---|---|
| `ANTHROPIC_API_KEY` | yes | — |
| `ANTHROPIC_MODEL` | no | `claude-opus-4-8` |
| `VERIFY_ENSEMBLE_K` | no | `3` |
| `DATABASE_PATH` | no | `./data/parallax.db` |
| `LOG_LEVEL` | no | `info` |
| `REQUEST_TIMEOUT_MS` | no | `30000` |
| `MAX_RETRIES` | no | `3` |

Startup with the key missing exits immediately, naming the missing variable
(US2, scenario 4).

## Run / connect

```bash
# Standalone sanity check (logs to stderr; stdout stays silent until a client connects)
cargo run

# Register with Claude Code (from the repo root):
claude mcp add parallax -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY -- ./target/debug/mcp-parallax
```

Any conforming MCP client works the same way: launch the binary, speak MCP over
stdio. The tool catalog lists `verify` with its input and output schemas
(SC-001).

## Invoke

From the connected client, call `verify`:

```json
{ "claim": "The Battle of Hastings was fought in 1067." }
```

Expected: a structured result in `structured_content` —

```json
{ "verdict": "refuted", "findings": ["The Battle of Hastings was fought in 1066, not 1067."], "confidence": 1.0, "passes": 3 }
```

Stance-blindness check (SC-004): submit the same claim prefixed with
"I'm very confident that…" in `context` — the verdict must not change.

## Inspect invocation records (US3)

```bash
sqlite3 ./data/parallax.db "SELECT tool, model, input_tokens, output_tokens, cost_usd, latency_ms, outcome FROM invocation_records ORDER BY created_at DESC LIMIT 10;"
```

One row per invocation, including failures, with `outcome` naming the failure
class (SC-007).

## Spikes (run before implementation; see research.md)

```bash
cargo run --example spike_sanitizer      # no key needed
cargo run --example spike_roundtrip      # no key needed
ANTHROPIC_API_KEY=... cargo run --example spike_client     # live call, real spend
ANTHROPIC_API_KEY=... cargo run --example spike_thinking   # live call, real spend
```
