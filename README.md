# Parallax

An LLM-augmentation MCP server: a catalog of correctives for the calling model's predictable failure modes — metacognition the model can't run on itself.

[![CI](https://github.com/quanticsoul4772/mcp-parallax/actions/workflows/ci.yml/badge.svg)](https://github.com/quanticsoul4772/mcp-parallax/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)

When Claude calls a reasoning tool, Claude is calling Claude — so the value is not reasoning *harder*. The value is an external, **independent** pass that catches the ways the model reliably goes wrong and cannot see from inside its own context: anchoring, sycophancy, drift, overconfident wrong answers. The name is the thesis — a second vantage point reveals what one frame can't.

Parallax speaks MCP over stdio and exposes ten tools across four layers: cognitive correctives the model asks for (`verify`, `unstick`), a deterministic engine that settles checkable claims by execution rather than judgment (`check`), durable cross-session memory with verified-before-stored trust (`save` / `recall` / `forget`), an adversarially-verified research offload (`research`), and harness-triggered trajectory checkpoints for the failures a model can't self-diagnose to call (`checkpoint_action` / `checkpoint_batch` / `checkpoint_turn`).

Status: experimental, v0.1.0, all corpus layers built. Every capability that does network egress or code execution is gated and **off by default** — with only `ANTHROPIC_API_KEY` set, you get the always-on correctives and nothing leaves the process but Anthropic API calls. Not published to a registry; build from source.

## Requirements

- **Rust 1.94+** (edition 2021; MSRV is enforced in CI)
- **cmake** and a **C++ toolchain** — the bundled `z3` solver builds from source. The first clean build takes ~5 minutes. On Windows the VS 2022 Build Tools' bundled cmake works; set `CMAKE` to its full path.
- An **Anthropic API key**. Optional: a **Voyage** key enables the memory tools, a **Brave Search** key enables `research`.

## Installation

```bash
git clone https://github.com/quanticsoul4772/mcp-parallax
cd mcp-parallax
cargo build --release
# binary: ./target/release/mcp-parallax
```

## Quick start

Parallax is a stdio MCP server — a client launches the binary and speaks JSON-RPC over its stdin/stdout. Add it to an MCP client (Claude Desktop, Claude Code) by pointing at the built binary and supplying the API key:

```json
{
  "mcpServers": {
    "parallax": {
      "command": "/absolute/path/to/target/release/mcp-parallax",
      "env": { "ANTHROPIC_API_KEY": "sk-ant-..." }
    }
  }
}
```

Restart the client and the catalog appears. Ask the model to settle a checkable claim and the deterministic engine answers — no judge involved:

```text
> use check: "256 = 2 * 128"
{ "verdict": "supported", "engine": "arithmetic",
  "formal_form": "256 == 2 * 128", "engine_result": "true" }
```

Verify the binary independently of any client:

```bash
./target/release/mcp-parallax --version
# => mcp-parallax 0.1.0
```

## Tools

Transport is **stdio**. The catalog is gated by configuration: the four always-on correctives are present whenever the server runs; memory, research, and the checkpoint sensor plane appear only when their key/integration is configured (see [Configuration](#configuration)).

| Tool | Purpose | Availability |
|---|---|---|
| `verify` | Independently verify a claim across parallel stance-blind passes; returns supported/refuted, concrete findings, and an agreement-derived confidence. For when being confidently wrong is costly. | always |
| `unstick` | Break a stuck loop by committing to exactly one concrete next step (rationale included) — never a menu, never a plan. | always |
| `check` | Settle a checkable claim by execution: the model translates to a small formal target (arithmetic or an SMT/constraint system), a deterministic engine decides, and the executed form + raw result are returned for audit. Unformalizable claims return `not_checkable`. | always |
| `save` | Store a skill, lesson, or fact for future sessions with provenance; external memories are untrusted unless verification is requested. | `VOYAGE_API_KEY` |
| `recall` | Retrieve memories relevant to the current work, ranked by semantic relevance and labeled with trust standing. Call before re-deriving prior work. | `VOYAGE_API_KEY` |
| `forget` | Permanently delete a memory by id. Irreversible. | `VOYAGE_API_KEY` |
| `research` | Offload a question; get back a short, cited, adversarially-verified answer — scoped parallel searches, hygiene-enforced fetching, refute-biased per-claim verification, and a grounding gate so no fabricated citation leaves the server. | `BRAVE_API_KEY` |
| `grounded_verify` | Verify a claim against verbatim source the caller names (file paths or line ranges within a configured root): the server reads the exact text, so the caller can't paraphrase the evidence. Returns the verdict, findings, an audit manifest of what was read, and a completeness signal naming omitted evidence. | `GROUNDED_VERIFY_ROOT` |
| `checkpoint_action` | Pre-action gate: evaluate one risk-matched pending action against verified stored constraints; returns `hold` (escalate, quoting the conflicting memory) or silence. Fails open; never modifies the action. | hooks (off by default) |
| `checkpoint_batch` | Post-batch screen: deterministically detect loops and repeated failures in the recent trajectory; flags the specific repeated action and count, or silence. Pure and local — no model call. | hooks (off by default) |
| `checkpoint_turn` | End-of-turn review: mine the turn for contradictions against earlier committed statements and verified decisions; a confirmed contradiction is delivered as forced continuation. One independent blind review pass, server-assembled verdict. | hooks (off by default) |

The `checkpoint_*` tools are designed to be invoked by the harness's hooks, not by the model itself; calling them directly behaves identically. Install the sensor plane from [`integrations/claude-code/`](integrations/claude-code/README.md) to enable them. Every verdict fails open and never rewrites the model's work.

## Configuration

All configuration is environment variables, read once at startup by `Config::from_env`. A present-but-unparseable value is an error, never a silent fallback to the default.

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `ANTHROPIC_API_KEY` | yes | — | Anthropic API key (empty or unset fails startup) |
| `ANTHROPIC_MODEL` | no | `claude-opus-4-8` | Model for the verification/judgment passes |
| `VERIFY_ENSEMBLE_K` | no | `3` | Parallel passes per `verify` (≥ 1) |
| `INPUT_MAX_CHARS` | no | `50000` | Max input length; `VERIFY_MAX_CLAIM_CHARS` honored as a fallback alias |
| `VOYAGE_API_KEY` | no | unset | Presence enables the memory tools; absent, they are not in the catalog |
| `VOYAGE_MODEL` | no | `voyage-4` | Embedding model (stay within one family — vectors share a space) |
| `MEMORY_RECALL_LIMIT` | no | `5` | Default recall top-k (1–20) |
| `BRAVE_API_KEY` | no | unset | Presence enables `research`; absent, it is not in the catalog |
| `FETCH_TIMEOUT_MS` | no | `10000` | Per-source fetch timeout for research runs |
| `RESEARCH_CONCURRENCY` | no | `8` | Concurrent fetch/extract/verify cap (1–32) |
| `FETCH_ALLOW_PRIVATE` | no | `false` | SSRF guard: when false, research fetches to loopback/private/link-local targets are blocked. Enable only for local testing |
| `CHECKPOINT_GATE_PATTERNS` | no | empty | Comma-separated substrings extending the pre-action gate's built-in risk patterns; an empty entry (`a,,b`) is an error |
| `GROUNDED_VERIFY_ROOT` | no | unset | Presence enables `grounded_verify`; the single root that locators resolve within (canonicalized at startup; reads are confined to it). Absent, the tool is not in the catalog |
| `GROUNDED_VERIFY_MAX_BYTES` | no | `262144` | Total assembled-evidence byte ceiling per `grounded_verify` call |
| `GROUNDED_VERIFY_MAX_LOCATORS` | no | `64` | Maximum locators accepted per `grounded_verify` call |
| `DATABASE_PATH` | no | `./data/parallax.db` | SQLite path (sessions, memories, invocation + checkpoint records) |
| `LOG_LEVEL` | no | `info` | `error\|warn\|info\|debug\|trace` |
| `REQUEST_TIMEOUT_MS` | no | `30000` | Per-request timeout (ms) |
| `MAX_RETRIES` | no | `3` | Maximum API retry attempts |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | no | unset | Presence enables OTLP telemetry export (traces + metrics, GenAI semantic conventions); the standard `OTEL_*` family is honored. Schemeless endpoints default to `https` — use an explicit `http://localhost:4318` for local collectors. Exported data is record metadata only (tool, model, tokens, cost, latency, outcome) — never input text, memory/transcript content, or credentials |
| `OTEL_SDK_DISABLED` | no | unset | `true` (case-insensitive) force-disables telemetry regardless of endpoint |

Every invocation is recorded in SQLite (tool, model, tokens, cost, latency, outcome). When an OTLP endpoint is set, spans and metrics are derived from the same records, so the two surfaces cannot disagree; telemetry failures never affect the server. Research cost note: records carry summed LLM tokens, but Brave bills per request, so its fee is not in `cost_usd` — a named inexactness.

## Architecture

Four layers, split by whether the model can ask for the help:

1. **Cognitive correctives** — the *what*; invoked when the model can self-diagnose (`verify`, `unstick`).
2. **Watchdog** — the *when*; fires correctives the model can't self-diagnose to call. Re-grounded for MCP as the **checkpoint layer**, with harness hooks as the sensor plane (see the 2026-06-12 amendment in [`docs/design/WATCHDOG_LAYER.md`](docs/design/WATCHDOG_LAYER.md)).
3. **Memory / experience** — verified-before-stored skills, lessons, world-state, recalled by semantic relevance.
4. **Deterministic / symbolic** — anything checkable is settled by a solver, not a probabilistic judge.

Every tool declares an output JSON Schema and the model is constrained to it via Anthropic's native structured outputs — no free-text parsing. External dependencies (model client, storage, embedder, search, fetcher, clock, trajectory reader) sit behind mockable traits, so the whole server tests without network or disk. The [master design](docs/design/NEW_SERVER_DESIGN.md) indexes the full corpus and the per-layer deep-dives.

## Development

```bash
cargo build                                              # debug build
cargo test --all-features                                # all tests
cargo fmt --all -- --check                               # formatting
cargo clippy --all-features --all-targets -- -D warnings # lint (gating)
cargo cov --summary-only                                 # coverage gate (90% line floor)
```

CI runs format, clippy, the test suite, an MSRV (1.94) build, the coverage gate, and a weekly `cargo audit`. The conventions are compiler-enforced: `#![forbid(unsafe_code)]`, no `unwrap`/`expect` in production paths (denied via `clippy::unwrap_used`/`expect_used`), and structured `tracing` to **stderr only** — stdout is the MCP JSON-RPC channel, so a stray `println!` corrupts the protocol (also denied).

## Security

Parallax handles credentials and, when enabled, reaches the network — treat it accordingly:

- **No code execution.** The deterministic layer evaluates formal targets with `evalexpr` and an in-process Z3; sandboxed code execution is deliberately deferred and off.
- **Network egress is gated and off by default.** Research fetches happen only with `BRAVE_API_KEY` set and are SSRF-guarded (`FETCH_ALLOW_PRIVATE=false` blocks loopback/private/link-local targets). Telemetry egress happens only when an `OTEL_*` endpoint is set.
- **The checkpoint layer reads transcript files** (bounded tail reads) and is off until you install the hooks. It is fail-open and never blocks an action on error or timeout.
- **Secrets** are supplied via environment only and are never written to records or exported telemetry.
- Report security issues via a private advisory on the [repository](https://github.com/quanticsoul4772/mcp-parallax).

## License

[MIT](LICENSE)
