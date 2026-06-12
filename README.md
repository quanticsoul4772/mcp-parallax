# Parallax

An LLM-augmentation MCP server: a **catalog of correctives for the calling
model's predictable failure modes** — *metacognition the model can't run on
itself.*

When Claude calls a reasoning tool, Claude is calling Claude — so the value is
not reasoning *harder*. The value is an external, **independent** pass that
catches the ways the model reliably goes wrong and cannot see from inside its own
context (anchoring, sycophancy, drift, overconfident wrong answers). The name is
the thesis: a second vantage point reveals what one frame can't.

> **Status: core + memory + research + deterministic + checkpoint layers.** The server
> speaks MCP over stdio and serves **`verify`** (k parallel stance-blind
> verification passes, default 3, aggregated by majority with
> agreement-derived confidence), **`unstick`** (one committed next step for a
> stuck caller, single pass), **`check`** (checkable claims settled by
> execution, not judgment: the model only classifies and translates into a
> small formal target — an arithmetic expression or an SMT-LIB 2 constraint
> system — and a deterministic engine decides, with the executed form and raw
> result returned for audit; always on, no extra credential) — plus, when
> `VOYAGE_API_KEY` is set, **`save`/`recall`/`forget`** (durable
> cross-session memory with verified-before-stored trust), and, when
> `BRAVE_API_KEY` is set, **`research`** (offload a question; get back a
> short, cited, adversarially-verified answer — scoped parallel searches,
> hygiene-enforced fetching, refute-biased per-claim verification, and a
> deterministic grounding gate so no fabricated citation ever leaves the
> server) — plus the **checkpoint layer** (`checkpoint_action` /
> `checkpoint_batch` / `checkpoint_turn`): the watchdog re-grounded for MCP —
> harness hooks trigger trajectory checkpoints the model can't self-diagnose
> to call (loop/repeated-failure flags, constraint-conflict holds quoting the
> stored memory, end-of-turn contradiction review). **Off by default**:
> install the hooks in
> [`integrations/claude-code/`](integrations/claude-code/README.md) to enable
> the sensor plane (live-verified — `examples/spike_hooks.md`); everything
> fails open and never rewrites the model's work. Every invocation is recorded (tool, model, tokens, cost, latency,
> outcome) in SQLite.
>
> Research cost note: records carry summed LLM tokens; Brave bills
> per-request, so its fee is not in `cost_usd` (a named inexactness).

## The architecture (four layers)

1. **Cognitive correctives** — the *what*; invoked when the model can
   self-diagnose (Verify, Diverge, Decide, …).
2. **Watchdog** — the *when*; fires correctives the model can't self-diagnose to
   call. Re-grounded for MCP as the **checkpoint layer** (harness hooks as the
   sensor plane — see the 2026-06-12 amendment in
   [`docs/design/WATCHDOG_LAYER.md`](docs/design/WATCHDOG_LAYER.md)).
3. **Memory / experience** — verified-before-stored skills, lessons, world-state;
   the literature says this can outweigh the model itself.
4. **Deterministic / symbolic** — anything checkable is settled by a solver, not
   a probabilistic judge.

See the [master design](docs/design/NEW_SERVER_DESIGN.md) and the deep-dives it
indexes.

## Build & test

> Contributor note: the `z3` dependency builds Z3 from source (`bundled`) —
> the first clean build takes ~5 minutes and requires **cmake** (on Windows,
> the VS 2022 Build Tools' bundled cmake works: set the `CMAKE` env var to
> its full path). CI runners ship cmake; rust-cache amortizes the build.

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
| `BRAVE_API_KEY` | no | unset | Presence enables the `research` tool; absent, it is not in the catalog |
| `FETCH_TIMEOUT_MS` | no | `10000` | Per-source fetch timeout for research runs |
| `RESEARCH_CONCURRENCY` | no | `8` | Concurrent fetch/extract/verify cap (1–32) |
| `DATABASE_PATH` | no | `./data/parallax.db` | SQLite path |
| `LOG_LEVEL` | no | `info` | `error\|warn\|info\|debug\|trace` |
| `REQUEST_TIMEOUT_MS` | no | `30000` | Per-request timeout (ms) |
| `MAX_RETRIES` | no | `3` | Maximum API retry attempts |

## Conventions (carried over from `mcp-reasoning`, enforced)

- `#![forbid(unsafe_code)]`; no `unwrap`/`expect` in production paths
  (compiler-denied).
- Structured `tracing` to **stderr only** — stdout is the MCP JSON-RPC channel.
- Trait-mockable boundaries (`TimeProvider`, `ModelClient`, `Storage`,
  `Embedder`, `SearchProvider`, `Fetcher`) so the whole server tests without
  network or disk.
- Composition over trait inheritance.

## License

MIT
