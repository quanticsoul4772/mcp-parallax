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

Status: experimental; see [`CHANGELOG.md`](CHANGELOG.md) for the current release. Network egress and code execution are gated and off by default; with only `ANTHROPIC_API_KEY` set, the always-on correctives are available and the only outbound traffic is to the Anthropic API. Built from source; not published to a registry.

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
# => mcp-parallax <version>   (matches Cargo.toml)
```

## Tools

Transport is **stdio**. The catalog is gated by configuration: the six always-on correctives are present whenever the server runs; `grounded_verify`, memory, research, and the checkpoint sensor plane appear only when their root/key/integration is configured (see [Configuration](#configuration)).

| Tool | Purpose | Availability |
|---|---|---|
| `verify` | Verify a claim across parallel passes, each applying a distinct critical lens; returns supported/refuted, findings, and a confidence derived from how much the passes agree. | always |
| `unstick` | Returns exactly one concrete next step with a rationale — not a menu or a plan. | always |
| `diverge` | Returns alternative framings of a problem: parallel passes each apply a distinct angle (invert the goal, change the actor, shift the horizon, deny a load-bearing assumption, reframe the problem class); the server deduplicates and labels each framing with its angle. | always |
| `decide` | Selects among two to ten supplied options: a single pass applies a methodology (weigh / causal / probabilistic) and scores each option; the server picks the highest score and reports the runner-up, the deciding factors, the methodology, and a confidence derived from the score margin. | always |
| `elicit` | Surfaces the objective a request implies and the preferences that should govern it: returns the assumed objective, the governing preferences (each traced to its signal; revealed and stored preferences outrank stated ones), and the divergence points worth resolving first. Reports when signal is low rather than inventing preferences. With a Voyage key set it also consults stored verified preferences. Surfaces only — it does not block or modify (that is the checkpoint layer). | always |
| `check` | Settle a checkable claim by execution: the model translates to a small formal target (arithmetic or an SMT/constraint system), a deterministic engine decides, and the executed form + raw result are returned for audit. A claim bundling several statements is checked part by part, each part's verdict returned in `parts`. Unformalizable claims return `not_checkable`. | always |
| `save` | Store a skill, lesson, or fact for future sessions with provenance; external memories are untrusted unless verification is requested. | `VOYAGE_API_KEY` |
| `recall` | Retrieve memories relevant to the current work, ranked by semantic relevance and labeled with trust standing. | `VOYAGE_API_KEY` |
| `forget` | Permanently delete a memory by id. Irreversible. | `VOYAGE_API_KEY` |
| `surface` | Prompt-time memory push: ranks stored memories against the turn's opening prompt and surfaces the few relevant trusted ones as advisory context — verbatim content, id, and trust standing. Pure ranking, no model pass, hard 500 ms budget, fail-open. Each memory surfaces at most once per session. | `VOYAGE_API_KEY` |
| `research` | Run a web query and return a short, cited, per-claim-verified answer: scoped parallel Exa searches, source text from Exa, refute-biased per-claim verification, and a grounding gate that drops unsupported citations. With `VOYAGE_API_KEY` also set, the fetched pages are reranked against the question and only the top 5 / 15 / 35 (quick / standard / deep) are read; a failed rerank reads them all and says why in `stats.rerank_skipped`. | `EXA_API_KEY` |
| `grounded_verify` | Verify a claim against verbatim source the caller names (file paths, line ranges, or glob patterns within a configured root): the server reads the exact text. Files whose names mark them as secrets (`.env`, keys, credential files) are refused: an exact path fails the call, a glob leaves them out and lists them under `refused`. Returns supported/refuted; for a computable property of a single source (a line/byte/match count vs a threshold) the server counts over the read bytes and the deterministic engine settles it, returning the executed form (e.g. `1224 > 1000`); anything broader or compound returns `inconclusive`. Also returns findings, an audit manifest of what was read, and a completeness signal naming omitted evidence. | `GROUNDED_VERIFY_ROOT` |
| `checkpoint_action` | Pre-action gate: evaluate one risk-matched pending action against verified stored constraints; returns `hold` (quoting the conflicting memory) or silence. Fails open; does not modify the action. | hooks (off by default) |
| `checkpoint_batch` | Post-batch screen: detect loops and repeated failures in the recent trajectory; flags the repeated action and count, or silence. Local, no model call. | hooks (off by default) |
| `checkpoint_turn` | End-of-turn review: check the turn for contradictions against earlier committed statements and verified decisions; a confirmed contradiction is delivered as forced continuation. One blind review pass, server-assembled verdict. | hooks (off by default) |

The `checkpoint_*` tools are designed to be invoked by the harness's hooks, not by the model itself; calling them directly behaves identically. Install the sensor plane from [`integrations/claude-code/`](integrations/claude-code/README.md) to enable them. Every verdict fails open and never rewrites the model's work.

## Configuration

All configuration is environment variables, read once at startup by `Config::from_env`. A present-but-unparseable value is an error, never a silent fallback to the default.

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `ANTHROPIC_API_KEY` | when `PARALLAX_BACKEND=anthropic` (the default) | — | Anthropic API key (empty or unset fails startup when the anthropic backend is selected) |
| `ANTHROPIC_MODEL` | no | `claude-opus-4-8` | Default model id for the anthropic backend |
| `ANTHROPIC_API_BASE` | no | `https://api.anthropic.com` | API endpoint. Exists so the whole client pool — including the per-effort variants 028 builds — can be pointed at a test double or a proxy; before it, a call carrying an effort bypassed the injected client and reached the live endpoint |
| `PARALLAX_BACKEND` | no | `anthropic` | Model backend for every call site: `anthropic` (native Messages), `openai_compat` (Chat Completions — OpenAI, Azure's compat surface, Ollama, vLLM, LM Studio), or `claude_cli` (your own signed-in Claude Code; no API key — see below) |
| `OPENAI_API_KEY` | when `PARALLAX_BACKEND=openai_compat` and the endpoint is the default OpenAI one | — | OpenAI-compatible API key. A custom endpoint may run keyless — Ollama, LM Studio, vLLM leave it unset, and the `Authorization` header is then omitted entirely |
| `OPENAI_MODEL` | when `PARALLAX_BACKEND=openai_compat` | — | Model id on the openai_compat endpoint — model names are provider-specific, so there is no default and startup fails without one |
| `OPENAI_API_BASE` | no | `https://api.openai.com/v1` | OpenAI-compatible API endpoint, `/v1`-suffixed by convention |
| `OPENAI_STRUCTURED_OUTPUT` | no | `auto` | Structured-output ladder for `openai_compat`: `auto` walks `json_schema` → `json_object` → `tool_shim` → `prompt_only`, degrading one rung per capability rejection; a rung name pins exactly that rung |
| `CLAUDE_CLI_PATH` | no | `claude` | The Claude Code CLI the `claude_cli` backend runs: a command on the path, or a full path |
| `CLAUDE_CLI_MODEL` | no | `claude-sonnet-5-5` | Default model on the `claude_cli` backend |
| `CLAUDE_CLI_CONCURRENCY` | no | `4` | Live Claude Code processes allowed at once, server-wide, on the `claude_cli` backend (1–32) |
| `VERIFY_ENSEMBLE_K` | no | `3` | Parallel passes per `verify` (≥ 1) |
| `INPUT_MAX_CHARS` | no | `50000` | Max input length; `VERIFY_MAX_CLAIM_CHARS` honored as a fallback alias |
| `VOYAGE_API_KEY` | no | unset | Presence enables the memory tools and research's page reranking (`rerank-3`); absent, the memory tools are not in the catalog and research reads every fetched page |
| `VOYAGE_MODEL` | no | `voyage-4` | Embedding model (stay within one family — vectors share a space) |
| `MEMORY_RECALL_LIMIT` | no | `5` | Default recall top-k (1–20) |
| `BRAVE_API_KEY` | no | unset | No longer used: research runs on Exa. Set while `EXA_API_KEY` is unset, it stops startup with an error naming both; with `EXA_API_KEY` set it is ignored, so other tools can share it |
| `EXA_API_KEY` | no | unset | Presence enables `research` (Exa search and page text); absent, it is not in the catalog |
| `TYPESAFE_API_KEY` | no | unset | Presence enables nothing by itself. A blank value counts as unset. Never logged. Removed from the claude_cli child environment. |
| `PARALLAX_JEV_MODE_TURN_REVIEW` | no | unset | `off` or `shadow`; unset is `off`. `shadow`: Jev answers beside every end-of-turn review, recorded only. Sends a redacted turn summary, transcript-only statement pairs and the final assistant text. Requires the key. |
| `PARALLAX_JEV_MODE_RESEARCH_CLAIMS` | no | unset | `off` or `shadow`; unset is `off`. `shadow`: Jev answers beside every verified research claim, recorded only. Sends the redacted claim and its source excerpts. Requires the key. |
| `PARALLAX_JEV_MODE_RESEARCH_INJECTION` | no | unset | `off`, `shadow` or `on`; unset is `off`. Jev screens each kept research page for text addressed to an AI. `shadow`: recorded only. `on`: pages at or above the pinned threshold (0.35 on `jev-1.14.0`) carry an advisory warning; no warning does not mean clean. Sends the redacted page URL and capped text. Requires the key; `on` needs a pinned threshold for `PARALLAX_JEV_MODEL`. |
| `PARALLAX_JEV_MODE_TOOL_RISK` | no | unset | `off` or `shadow`; unset is `off`. `shadow`: each pending tool call gets a risk class, recorded only; the gate's verdict never changes. Sends the redacted Bash command (heredoc bodies dropped), Write and Edit file paths, the `file_path`, `path` and `pattern` values of Read, Grep and Glob, or other tools' names and argument names, to Jev and to the configured model for a label. Requires the key. |
| `PARALLAX_JEV_MODEL` | no | unset | Pinned model; `jev-1.14.0` unless set. A `JEV_MODEL` variable is never read. |
| `EXA_MAX_AGE_HOURS` | no | `168` | Freshness limit for research page text, in hours: `0` always fetches live, `-1` uses Exa's cache only (-1 to 720) |
| `FETCH_TIMEOUT_MS` | no | unset | No longer used: Parallax does not fetch pages. Set, it stops startup with an error naming it |
| `RESEARCH_CONCURRENCY` | no | `8` | Concurrent extract/verify cap (1–32) |
| `FETCH_ALLOW_PRIVATE` | no | unset | No longer used: Parallax does not fetch pages. Set, it stops startup with an error naming it |
| `CHECKPOINT_GATE_PATTERNS` | no | empty | Comma-separated substrings extending the pre-action gate's built-in risk patterns; an empty entry (`a,,b`) is an error |
| `CHECKPOINT_TURN_SIGNALS` | no | `off` | `off` or `on`. With `on`, each turn end runs the pushback-reversal and unverified-fact detectors in code and writes one `turn_signal_records` row; nothing is sent. Any other value is an error |
| `GROUNDED_VERIFY_ROOT` | no | unset | Presence enables `grounded_verify`; the single root that locators resolve within (canonicalized at startup; reads are confined to it). Absent, the tool is not in the catalog. If the root is inside a folder whose files are refused as secrets (such as `.ssh`), or is a drive root or the home folder, `grounded_verify` is left out of the catalog with the reason logged; every other tool still starts. Set it to `.` to give each session its own folder: each Parallax uses the folder its session was started in. |
| `GROUNDED_VERIFY_MAX_BYTES` | no | `262144` | Total assembled-evidence byte ceiling per `grounded_verify` call |
| `GROUNDED_VERIFY_MAX_LOCATORS` | no | `64` | Maximum locators accepted per `grounded_verify` call |
| `DATABASE_PATH` | no | `./data/parallax.db` | SQLite path (sessions, memories, invocation + checkpoint records) |
| `LOG_LEVEL` | no | `info` | `error\|warn\|info\|debug\|trace` |
| `REQUEST_TIMEOUT_MS` | no | `120000` | Per-request timeout (ms). Raised from 30 s by 018 D7 in step with the output budget — a model that reasons before answering can exceed 30 s on a large ceiling, turning a truncation into a timeout rather than fixing it |
| `MAX_RETRIES` | no | `3` | Maximum API retry attempts |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | no | unset | Presence enables OTLP telemetry export (traces + metrics, GenAI semantic conventions); the standard `OTEL_*` family is honored. Schemeless endpoints default to `https` — use an explicit `http://localhost:4318` for local collectors. Exported data is record metadata only (tool, model, tokens, cost, latency, outcome) — never input text, memory/transcript content, or credentials |
| `OTEL_SDK_DISABLED` | no | unset | `true` (case-insensitive) force-disables telemetry regardless of endpoint |

### Model backend (BYOM)

`PARALLAX_BACKEND` selects the wire format for **every** model call; the tool
catalog, schemas, verdict semantics and per-call-site routing are identical on
both backends. Switching providers is a config change — no call-site edits.

- **Structured output** (`openai_compat`): the sanitized mode schema is the
  single source of truth on all four ladder rungs — `json_schema` mode,
  `json_object` mode with the schema in the prompt, a forced single-function
  tool call, and prompt-only. `auto` degrades one rung per capability
  rejection; every schema parse still fails loudly (`ValidationFailure`),
  never silently accepted.
- **Outcome taxonomy**: identical across backends — truncation bills as
  `Truncation` (`stop_reason: max_tokens` / `finish_reason: length`), refusals
  as `Refusal`, and any unexplained signal or empty body as an out-of-contract
  `Client` error. A wrong answer is never mistaken for a truncated one.
- **Token accounting**: `usage` maps onto every call's metered record; a
  provider that omits usage records zeros **and** logs a warning per call, so a
  mispriced record is never silent. Pricing for non-Anthropic model ids is
  unknown — cost figures stay flagged as estimated.
- **Effort**: `PARALLAX_EFFORT_*` and the per-call `effort` argument are a
  documented no-op on `openai_compat` — the level is dropped at the wire with
  a logged notice, never silently mistranslated.
- **Credentials** stay in environment variables only, never in tool arguments.
  `OPENAI_API_KEY` is required only for the default OpenAI endpoint; a custom
  endpoint may run keyless, and the `Authorization` header is then omitted.

### Running on your Claude subscription (`claude_cli`)

`PARALLAX_BACKEND=claude_cli` sends every model call through the Claude Code
CLI already installed and signed in on your machine, so Parallax runs on your
Claude subscription instead of API credit. No `ANTHROPIC_API_KEY` is needed.

- **Your login, your use.** Parallax runs the operator's own `claude` and never
  reads, stores or passes credentials; signing in stays in Claude Code. The
  backend is off by default.
- **Isolated calls.** Each call runs `claude -p` with no MCP servers (so it
  cannot call Parallax back), no tools, `--safe-mode` (no CLAUDE.md, hooks,
  skills or plugins), memory files disabled, a fresh private working folder,
  and API-billing variables removed from its environment.
- **Settings.** `CLAUDE_CLI_PATH` (default `claude`; the native executable, not
  a `.cmd` launcher), `CLAUDE_CLI_MODEL` (default `claude-sonnet-5-5`; research
  extraction defaults to `claude-haiku-5-5`), `CLAUDE_CLI_CONCURRENCY` (default
  4 processes at once). `PARALLAX_MODEL_*` and `PARALLAX_EFFORT_*` still apply.
- **Limits and cost.** Calls count against your subscription's usage limits. A
  reached limit is reported as `usage_limit` and not retried. Records carry the
  CLI's API-price estimate with `cost_basis = cli_estimate`, never a charge.

### Per-call-site routing: `PARALLAX_MODEL_*` and `PARALLAX_EFFORT_*`

The server asks a model for a schema-constrained answer in twelve named places. Each is independently routable to a model (018) and to a provider reasoning-effort level (022), over two parallel namespaces. Both are **off by default**: with neither set, every site uses the selected backend's default model — `ANTHROPIC_MODEL` on `anthropic`, `OPENAI_MODEL` on `openai_compat`, as the startup table's `source=` field reports — and the request carries no `effort` field at all, byte-identical to a server without the feature.

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

**Effort support varies by model family, and the pairing is not checked at startup.** `claude-haiku-4-5` rejects the parameter, and it is also the natural choice for `PARALLAX_MODEL_BULK` — so `PARALLAX_MODEL_BULK=claude-haiku-4-5` with `PARALLAX_EFFORT_BULK` set will fail every extraction call in a research run. The server does not carry a capability table (it would go stale in the direction that refuses a configuration which would have worked); instead the provider's rejection is surfaced with the model, the level, and both remedies named. The startup routing table prints the resolved model, effort, and the endpoint the active backend will call for all twelve sites — `source=` names the setting that supplied the model, `endpoint=` names where traffic goes, so a keyless local run (Ollama behind `OPENAI_MODEL`) is self-describing rather than vendor-suggestive.

### Caller-facing arguments: what the model can set per call

The environment namespaces above set the **defaults**. A calling model can override the per-task ones on a single invocation, with no file edited and no restart:

| Argument | Tools | Rule |
|---|---|---|
| `effort` | `verify` `unstick` `diverge` `decide` `elicit` `grounded_verify` `check` | Any level, case-insensitive (`"Medium"` works; the schema shows the lowercase canonical spellings); unknown values are rejected by name |
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

Every invocation is recorded in SQLite (tool, model, tokens, cost, latency, outcome). The console line carries the same facts plus `cost.estimated`: `false` means the figure is priced from the catalog, `true` that it is the conservative Opus-tier fallback for an unknown model id (local models such as Ollama's hit this fallback — their real spend is $0.00, the figure is an over-estimate, and tokens are the resource actually consumed). When an OTLP endpoint is set, spans and metrics are derived from the same records (the flag exports as `parallax.cost_estimated`), so the surfaces cannot disagree; telemetry failures never affect the server. Research cost note: records carry summed LLM tokens, but Brave bills per request, so its fee is not in `cost_usd` — a named inexactness.

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
cargo test --all-features --tests                        # all tests
cargo fmt --all -- --check                               # formatting
cargo clippy --all-features --all-targets -- -D warnings # lint (gating)
cargo cov --summary-only                                 # coverage gate (90% line floor)
```

CI runs format, clippy, the test suite, an MSRV (1.94) build, the coverage gate, and a weekly `cargo audit`. The conventions are compiler-enforced: `#![forbid(unsafe_code)]`, no `unwrap`/`expect` in production paths (denied via `clippy::unwrap_used`/`expect_used`), and structured `tracing` to **stderr only** — stdout is the MCP JSON-RPC channel, so a stray `println!` corrupts the protocol (also denied).

## Security

Parallax handles credentials and, when enabled, reaches the network — treat it accordingly:

- **No code execution.** The deterministic layer evaluates formal targets with an exact in-process arithmetic evaluator (parsed by `evalexpr`) and an in-process Z3; sandboxed code execution is deliberately deferred and off.
- **Network egress is gated and off by default.** Research egress happens only with `EXA_API_KEY` set, and goes only to Exa: Parallax requests no source page itself. Telemetry egress happens only when an `OTEL_*` endpoint is set.
- **The checkpoint layer reads transcript files** (bounded tail reads) and is off until you install the hooks. It is fail-open and never blocks an action on error or timeout.
- **Secrets** are supplied via environment only and are never written to records or exported telemetry.
- Report security issues via a private advisory on the [repository](https://github.com/quanticsoul4772/mcp-parallax).

## License

[MIT](LICENSE)
