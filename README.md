# Parallax

An LLM-augmentation MCP server: a **catalog of correctives for the calling
model's predictable failure modes** — *metacognition the model can't run on
itself.*

When Claude calls a reasoning tool, Claude is calling Claude — so the value is
not reasoning *harder*. The value is an external, **independent** pass that
catches the ways the model reliably goes wrong and cannot see from inside its own
context (anchoring, sycophancy, drift, overconfident wrong answers). The name is
the thesis: a second vantage point reveals what one frame can't.

> **Status: core layer + memory layer.** The server speaks MCP over stdio and
> serves **`verify`** (k parallel stance-blind verification passes, default 3,
> aggregated by majority with agreement-derived confidence), **`unstick`** (one
> committed next step for a stuck caller, single pass), and — when
> `VOYAGE_API_KEY` is set — **`save`/`recall`/`forget`** (durable cross-session
> memory with verified-before-stored trust: external content is quarantined as
> untrusted unless an independent verification pass admits it, and refuted
> content is rejected with findings). Every invocation is recorded (tool,
> model, tokens, cost, latency, outcome) in SQLite. The remaining layers follow
> the design north star in
> [`docs/design/NEW_SERVER_DESIGN.md`](docs/design/NEW_SERVER_DESIGN.md).

## The architecture (four layers)

1. **Cognitive correctives** — the *what*; invoked when the model can
   self-diagnose (Verify, Diverge, Decide, …).
2. **Watchdog** — the *when*; fires correctives the model can't self-diagnose to
   call, running beside generation on the activity stream.
3. **Memory / experience** — verified-before-stored skills, lessons, world-state;
   the literature says this can outweigh the model itself.
4. **Deterministic / symbolic** — anything checkable is settled by a solver, not
   a probabilistic judge.

See the [master design](docs/design/NEW_SERVER_DESIGN.md) and the deep-dives it
indexes.

## Build & test

```bash
cargo build
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

## Environment

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `ANTHROPIC_API_KEY` | yes | — | Anthropic API key |
| `ANTHROPIC_MODEL` | no | `claude-opus-4-8` | Model for verification passes |
| `VERIFY_ENSEMBLE_K` | no | `3` | Parallel passes per verify (≥ 1) |
| `INPUT_MAX_CHARS` | no | `50000` | Max input length (`VERIFY_MAX_CLAIM_CHARS` honored as alias) |
| `VOYAGE_API_KEY` | no | unset | Presence enables the memory tools (`save`/`recall`/`forget`); absent, they are not in the catalog |
| `VOYAGE_MODEL` | no | `voyage-4` | Embedding model (stay within one family — vectors share a space) |
| `MEMORY_RECALL_LIMIT` | no | `5` | Default recall top-k (1–20) |
| `DATABASE_PATH` | no | `./data/parallax.db` | SQLite path |
| `LOG_LEVEL` | no | `info` | `error\|warn\|info\|debug\|trace` |
| `REQUEST_TIMEOUT_MS` | no | `30000` | Per-request timeout (ms) |
| `MAX_RETRIES` | no | `3` | Maximum API retry attempts |

## Conventions (carried over from `mcp-reasoning`, enforced)

- `#![forbid(unsafe_code)]`; no `unwrap`/`expect` in production paths
  (compiler-denied).
- Structured `tracing` to **stderr only** — stdout is the MCP JSON-RPC channel.
- Trait-mockable boundaries (`TimeProvider`, `ModelClient`, `Storage`,
  `Embedder`) so the whole server tests without network or disk.
- Composition over trait inheritance.

## License

MIT
