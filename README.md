# Parallax

Parallax is an MCP server that gives a language model tools to check its own work: verify a claim, run a deterministic check, store and recall memory, research a question, and review its trajectory.

[![CI](https://github.com/quanticsoul4772/mcp-parallax/actions/workflows/ci.yml/badge.svg)](https://github.com/quanticsoul4772/mcp-parallax/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)

The server exposes fifteen tools in four groups:

- **Cognitive correctives**, called by the model: `verify` judges whether a claim holds, `unstick` returns one next step when looping, `diverge` returns alternative framings of a problem, `decide` selects among supplied options and reports the scoring, `elicit` surfaces the objective and governing preferences a request implies before the model commits.
- **Deterministic checks**: `check` settles arithmetic, logic, and constraint claims by executing a formal translation rather than judging it; `grounded_verify` checks a claim against verbatim files the caller names, settling computable properties (e.g. a line count) on the engine and abstaining otherwise.
- **Memory**: `save`, `recall`, and `forget` — cross-session storage, verified before it is trusted; `surface` pushes the relevant trusted ones into a turn without a model pass.
- **Research**: `research` runs a web query on a separate budget and returns a cited, per-claim-verified answer.
- **Trajectory checkpoints**: `checkpoint_action`, `checkpoint_batch`, and `checkpoint_turn`, called by the harness's hooks for failures the model does not self-diagnose.

See [Tools](#tools) for per-tool detail.

Status: experimental, v0.3.0. Network egress and code execution are gated and off by default; with only `ANTHROPIC_API_KEY` set, the always-on correctives are available and the only outbound traffic is to the Anthropic API. Built from source; not published to a registry.

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

Restart the client and the catalog appears. Ask the model to settle a checkable claim and the deterministic engine answers:

```text
> use check: "256 = 2 * 128"
{ "verdict": "supported", "engine": "arithmetic",
  "formal_form": "256 == 2 * 128", "engine_result": "true" }
```

Verify the binary independently of any client:

```bash
./target/release/mcp-parallax --version
# => mcp-parallax 0.3.0
```

## Tools

Transport is **stdio**. The catalog is gated by configuration: the six always-on correctives are present whenever the server runs; `grounded_verify`, memory, research, and the checkpoint sensor plane appear only when their root/key/integration is configured (see [Configuration](#configuration)).

| Tool | Purpose | Availability |
|---|---|---|
| `verify` | Verify a claim across parallel passes, each applying a distinct critical lens; returns supported/refuted, findings, and a confidence derived from how much the passes agree. | always |
| `unstick` | Returns exactly one concrete next step with a rationale — not a menu or a plan. | always |
| `diverge` | Returns alternative framings of a problem: parallel passes each apply a distinct angle (invert the goal, change the actor, shift the horizon, deny a load-bearing assumption, reframe the problem class); the server deduplicates and labels each framing with its angle. | always |
| `decide` | Selects among two or more supplied options: a single pass applies a methodology (weigh / causal / probabilistic) and scores each option; the server picks the highest score and reports the runner-up, the deciding factors, the methodology, and a confidence derived from the score margin. | always |
| `elicit` | Surfaces the objective a request implies and the preferences that should govern it: returns the assumed objective, the governing preferences (each traced to its signal; revealed and stored preferences outrank stated ones), and the divergence points worth resolving first. Reports when signal is low rather than inventing preferences. With a Voyage key set it also consults stored verified preferences. Surfaces only — it does not block or modify (that is the checkpoint layer). | always |
| `check` | Settle a checkable claim by execution: the model translates to a small formal target (arithmetic or an SMT/constraint system), a deterministic engine decides, and the executed form + raw result are returned for audit. Unformalizable claims return `not_checkable`. | always |
| `save` | Store a skill, lesson, or fact for future sessions with provenance; external memories are untrusted unless verification is requested. | `VOYAGE_API_KEY` |
| `recall` | Retrieve memories relevant to the current work, ranked by semantic relevance and labeled with trust standing. | `VOYAGE_API_KEY` |
| `forget` | Permanently delete a memory by id. Irreversible. | `VOYAGE_API_KEY` |
| `surface` | Prompt-time memory push: ranks stored memories against the turn's opening prompt and surfaces the few relevant trusted ones as advisory context — verbatim content, id, and trust standing. Pure ranking, no model pass, hard 500 ms budget, fail-open. Each memory surfaces at most once per session. | `VOYAGE_API_KEY` |
| `research` | Run a web query and return a short, cited, per-claim-verified answer: scoped parallel searches, hygiene-enforced fetching, refute-biased per-claim verification, and a grounding gate that drops unsupported citations. | `BRAVE_API_KEY` |
| `grounded_verify` | Verify a claim against verbatim source the caller names (file paths, line ranges, or glob patterns within a configured root): the server reads the exact text. Returns supported/refuted; for a computable property of a single source (a line/byte/match count vs a threshold) the server counts over the read bytes and the deterministic engine settles it, returning the executed form (e.g. `1224 > 1000`); anything broader or compound returns `inconclusive`. Also returns findings, an audit manifest of what was read, and a completeness signal naming omitted evidence. | `GROUNDED_VERIFY_ROOT` |
| `checkpoint_action` | Pre-action gate: evaluate one risk-matched pending action against verified stored constraints; returns `hold` (quoting the conflicting memory) or silence. Fails open; does not modify the action. | hooks (off by default) |
| `checkpoint_batch` | Post-batch screen: detect loops and repeated failures in the recent trajectory; flags the repeated action and count, or silence. Local, no model call. | hooks (off by default) |
| `checkpoint_turn` | End-of-turn review: check the turn for contradictions against earlier committed statements and verified decisions; a confirmed contradiction is delivered as forced continuation. One blind review pass, server-assembled verdict. | hooks (off by default) |

The `checkpoint_*` tools are designed to be invoked by the harness's hooks, not by the model itself; calling them directly behaves identically. Install the sensor plane from [`integrations/claude-code/`](integrations/claude-code/README.md) to enable them. Every verdict fails open and never rewrites the model's work.

## Configuration

All configuration is environment variables, read once at startup by `Config::from_env`. A present-but-unparseable value is an error, never a silent fallback to the default.

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `ANTHROPIC_API_KEY` | yes | — | Anthropic API key (empty or unset fails startup) |
| `ANTHROPIC_MODEL` | no | `claude-opus-4-8` | Model for the verification/judgment passes |
| `ANTHROPIC_API_BASE` | no | `https://api.anthropic.com` | API endpoint. Exists so the whole client pool — including the per-effort variants 028 builds — can be pointed at a test double or a proxy; before it, a call carrying an effort bypassed the injected client and reached the live endpoint |
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
| `REQUEST_TIMEOUT_MS` | no | `120000` | Per-request timeout (ms). Raised from 30 s by 018 D7 in step with the output budget — a model that reasons before answering can exceed 30 s on a large ceiling, turning a truncation into a timeout rather than fixing it |
| `MAX_RETRIES` | no | `3` | Maximum API retry attempts |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | no | unset | Presence enables OTLP telemetry export (traces + metrics, GenAI semantic conventions); the standard `OTEL_*` family is honored. Schemeless endpoints default to `https` — use an explicit `http://localhost:4318` for local collectors. Exported data is record metadata only (tool, model, tokens, cost, latency, outcome) — never input text, memory/transcript content, or credentials |
| `OTEL_SDK_DISABLED` | no | unset | `true` (case-insensitive) force-disables telemetry regardless of endpoint |

### Per-call-site routing: `PARALLAX_MODEL_*` and `PARALLAX_EFFORT_*`

The server asks a model for a schema-constrained answer in twelve named places. Each is independently routable to a model (018) and to a provider reasoning-effort level (022), over two parallel namespaces. Both are **off by default**: with neither set, every site uses `ANTHROPIC_MODEL` and the request carries no `effort` field at all — byte-identical to a server without the feature.

Resolution is **most-specific-first and independent per namespace**: the call site's own variable, else its tier's, else the default. A site can take its model from a tier and its effort from its own variable.

| Suffix | Call site | Tier |
|---|---|---|
| `VERIFY` `UNSTICK` `DIVERGE` `DECIDE` `ELICIT` `GROUNDED_VERIFY` | the correctives | `JUDGMENT` |
| `CHECK_TRANSLATE` | claim → formal target | `JUDGMENT` |
| `RESEARCH_SCOPE` `RESEARCH_VERIFY` `RESEARCH_SYNTHESIZE` | research phases 1, 4, 5 | `JUDGMENT` |
| `RESEARCH_EXTRACT` | research phase 3 — the one site whose volume scales with sources fetched | `BULK` |
| `CHECKPOINT_REVIEW` | end-of-turn review hop | `JUDGMENT` |

```sh
PARALLAX_MODEL_BULK=claude-haiku-4-5      # every bulk site
PARALLAX_MODEL_VERIFY=claude-opus-4-8     # one site, overriding its tier
PARALLAX_EFFORT_JUDGMENT=high             # every judgment site
PARALLAX_EFFORT_RESEARCH_SCOPE=low        # one site, overriding its tier
```

Effort levels are `low`, `medium`, `high`, `max`, `xhigh`. An unrecognised suffix or an unparseable level is a **startup error naming the variable** — a setting that silently does nothing would leave you believing a call site changed when it did not.

**Effort support varies by model family, and the pairing is not checked at startup.** `claude-haiku-4-5` rejects the parameter, and it is also the natural choice for `PARALLAX_MODEL_BULK` — so `PARALLAX_MODEL_BULK=claude-haiku-4-5` with `PARALLAX_EFFORT_BULK` set will fail every extraction call in a research run. The server does not carry a capability table (it would go stale in the direction that refuses a configuration which would have worked); instead the provider's rejection is surfaced with the model, the level, and both remedies named. The startup routing table prints the resolved model and effort for all twelve sites.

### Caller-facing arguments: what the model can set per call

The environment namespaces above set the **defaults**. A calling model can override the per-task ones on a single invocation, with no file edited and no restart:

| Argument | Tools | Rule |
|---|---|---|
| `effort` | `verify` `unstick` `diverge` `decide` `elicit` `grounded_verify` `check` | Any level; unknown values are rejected |
| `passes` | `verify` `diverge` `grounded_verify` | **Lowering only** — above the configured count is an error naming the ceiling |
| `constraints.concurrency` | `research` | **Lowering only** — above the ceiling is reduced to it, and the run proceeds |
| `limit` | `recall` | Any value in range (predates this) |
| `depth`, `constraints.*` | `research` | Any value in range (predates this) |

Omit them all and outbound requests are byte-identical to a server without the feature.

**Why these and not others.** A setting is the operator's when it sets the terms the account is billed on or governs resources beyond the call — which model runs a call site, egress rate, politeness owed to fetched hosts. It is the caller's when it is a judgment about *this task*: how much reasoning a question warrants, how many passes a claim deserves. Anything that multiplies the number of model calls is lowering-only, because a raise buys work you did not authorise.

**What you give up.** Spend is no longer fully predictable from configuration — a caller can raise effort on one call. `invocation_records.effort` records the override (NULL means none was supplied), so an unexpected cost stays attributable:

```sql
SELECT tool, model, effort, cost_usd FROM invocation_records ORDER BY cost_usd DESC LIMIT 20;
```

Every invocation is recorded in SQLite (tool, model, tokens, cost, latency, outcome). When an OTLP endpoint is set, spans and metrics are derived from the same records, so the two surfaces cannot disagree; telemetry failures never affect the server. Research cost note: records carry summed LLM tokens, but Brave bills per request, so its fee is not in `cost_usd` — a named inexactness.

## Architecture

Four layers, split by whether the model can ask for the help:

1. **Cognitive correctives** — the *what*; invoked when the model can self-diagnose (`verify`, `unstick`, `diverge`, `decide`, `elicit`).
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
