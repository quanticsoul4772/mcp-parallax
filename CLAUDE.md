# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**Parallax** — an LLM-augmentation MCP server (Rust, stdio). When Claude calls a
reasoning tool, Claude is calling Claude, so the value is **not** reasoning *harder*.
The value is an external, **independent** pass that catches the ways the model
reliably goes wrong and cannot see from inside its own context (anchoring,
sycophancy, drift, confidently-wrong answers). The name is the thesis: a second
vantage point reveals what one frame can't. It is a **catalog of correctives for the
calling model's predictable failure modes** — metacognition the model can't run on
itself.

**Status: all corpus layers built — core + memory + research + deterministic + checkpoint + observability.** The server
speaks MCP over stdio and serves **`verify`** (k parallel stance-blind
passes, each under a distinct critical lens so disagreement can surface,
agreement-derived confidence — 010), **`unstick`** (one committed next
step, single pass), **`diverge`** (k stance-blind passes under distinct
*generative* lenses — invert/actor/horizon/assumption/class — returning a
deterministically deduplicated set of distinct problem framings; the
divergence counterpart to verify, always on — 012), **`check`** (always on, no gate — pure in-process
engines: the model classifies checkability and translates to a small typed
formal target, evalexpr or Z3 executes, and verdict + explanation are
server-assembled; one violation-fed retry on real engine signals only), the
memory tools **`save`/`recall`/`forget`** (gated on `VOYAGE_API_KEY`;
verified-before-stored trust, brute-force cosine ranking — the named
sqlite-vec deviation, `SDK_LANDSCAPE.md` §memory), and **`research`** (gated
on `BRAVE_API_KEY`; five-phase scope→search→fetch+extract→verify→synthesize
pipeline with refute-biased per-claim verification and a deterministic
grounding gate — the model writes only the answer prose; findings, labels,
confidences, sources, and stats are server-assembled), and the **checkpoint
layer** — the watchdog re-grounded for MCP (`WATCHDOG_LAYER.md` 2026-06-12
amendment): three harness-triggered tools (`checkpoint_action` gate /
`checkpoint_batch` loop screening / `checkpoint_turn` review with the
layer's only model hop), always in the catalog but **off by default** — the
sensor plane is an installable hooks config in `integrations/claude-code/`
(live-verified by the S1 spike, `examples/spike_hooks.md`); verdicts are
silence/flag/hold, server-assembled, fail-open, one `checkpoint_records`
audit row per evaluation. One invocation record per call in SQLite;
**OTLP export** (gated on the standard `OTEL_EXPORTER_OTLP_ENDPOINT` —
unset means no providers, no egress) mirrors every invocation record and
checkpoint record as traces + metrics with GenAI semconv names, derived at
the same exit points so the surfaces cannot disagree
(`specs/007-observability-layer/contracts/telemetry.md` is the exported
surface). Build
note: `z3` (bundled) needs cmake — first clean build ~5 min; on Windows set
`CMAKE` to the VS Build Tools cmake path. Feature artifacts:
`specs/001-core-layer/` through `specs/012-diverge-perspectives/` (core +
memory + research + deterministic + checkpoint + observability, then
grounded-verify 008 / glob-locators 009 / verification-reliability 010 /
grounded-compute-settle 011 / diverge 012).

## The design is the source of truth

Read these before proposing architecture. The master doc indexes the rest.

- [`docs/design/NEW_SERVER_DESIGN.md`](docs/design/NEW_SERVER_DESIGN.md) — **start here.**
  Value model, the four-layer architecture, failure-mode catalog, primitives,
  routing, what's validated, what stops existing.
- [`docs/design/SDK_LANDSCAPE.md`](docs/design/SDK_LANDSCAPE.md) — the chosen SDK/crate
  stack per layer (web-grounded, 2026-06), with versions and caveats.
- [`docs/design/SDK_USAGE_CORE.md`](docs/design/SDK_USAGE_CORE.md) — *how* to wire the
  core SDKs: rmcp tools with `Json<T>` structured output, the thin Anthropic
  structured-outputs client behind `ModelClient`, the schema-sanitizer gotcha, and the
  spikes to run before building core.
- Layer deep-dives: `WATCHDOG_LAYER.md`, `MEMORY_LAYER.md`, `DETERMINISTIC_LAYER.md`,
  `CORRECTIVE_SELECTION.md`, `RESEARCH_PRIMITIVE.md`, `THEORY_OF_MIND.md`,
  `PREFERENCE_ELICITATION.md`, `OFFLOAD_LANDSCAPE.md`, `NEXT_REASONING_SERVER.md`.

### The four layers (split by whether the model can ask for the help)

1. **Cognitive correctives** — the *what*; invoked when the model can self-diagnose
   (Verify, Diverge, Decide, Step, Recall, Search, Research).
2. **Watchdog** — the *when*; fires correctives the model can't self-diagnose to call,
   running beside generation on the activity/event stream.
3. **Memory / experience** — verified-before-stored skills, lessons, world-state.
4. **Deterministic / symbolic** — anything checkable is settled by a solver, not a
   probabilistic judge.

### The core contract: constrained output (now native)

Every mode declares an output JSON Schema and the model is constrained to it via
Anthropic's **native structured outputs** (`output_config.format` / strict tools),
GA and supported on Opus 4.8. No `tool_choice` hack, no free-text parsing, no
`extract_json`. The API grammar drops numeric/length constraints and recursion, so a
**thin schema validator** enforces those, and mode schemas stay **flat + closed**
(`additionalProperties: false`). See `SDK_LANDSCAPE.md` §core.

## Build & test

```bash
cargo build                       # debug build
cargo test                        # all tests
cargo fmt --check                 # formatting
cargo clippy -- -D warnings       # lint (gating)
cargo test <module>               # e.g. cargo test config
cargo test <name> -- --exact      # single test by full path

# Aliases from .cargo/config.toml:
cargo ci                          # check --all-features
cargo lint                        # clippy -- -D warnings
cargo cov                         # llvm-cov coverage report (no gate yet)

# Full gate before every commit (also the /validate command):
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test
```

Toolchain is pinned (`rust-toolchain.toml`: stable + clippy/rustfmt/llvm-tools).
MSRV is 1.94 (CI verifies it with a dedicated job — bump `Cargo.toml` `rust-version`
and the CI job in lockstep). A separate weekly `cargo audit` workflow gates
advisories. `pre-commit` hooks mirror the gate — `pre-commit install`.

## Runtime configuration (`Config::from_env()`)

All config is environment variables: `ANTHROPIC_API_KEY` (required — the binary
errors at startup without it), `ANTHROPIC_MODEL` (default `claude-opus-4-8`),
`VERIFY_ENSEMBLE_K` (default `3`), `INPUT_MAX_CHARS` (default `50000`;
`VERIFY_MAX_CLAIM_CHARS` honored as alias), `VOYAGE_API_KEY` (optional —
presence enables the memory tools; absent, they are not in the catalog),
`VOYAGE_MODEL` (default `voyage-4`), `MEMORY_RECALL_LIMIT` (default `5`,
1..=20), `BRAVE_API_KEY` (optional — presence enables the `research` tool),
`FETCH_TIMEOUT_MS` (default `10000`), `RESEARCH_CONCURRENCY` (default `8`,
1..=32), `FETCH_ALLOW_PRIVATE` (default `false` — SSRF guard for research
fetches), `CHECKPOINT_GATE_PATTERNS` (default empty — comma-separated
substrings extending the gate's built-in risk patterns),
`GROUNDED_VERIFY_ROOT` (optional — presence enables the `grounded_verify`
tool; the single root that locators resolve within, confined at startup;
absent, the tool is not in the catalog), `GROUNDED_VERIFY_MAX_BYTES`
(default `262144`), `GROUNDED_VERIFY_MAX_LOCATORS` (default `64`),
`DATABASE_PATH`
(default `./data/parallax.db`), `LOG_LEVEL` (default `info`),
`REQUEST_TIMEOUT_MS` (default `30000`), `MAX_RETRIES` (default `3`).
A present-but-unparseable value is an error, never a silent fallback to the
default. Telemetry is enabled solely by the standard OTel variables
(`OTEL_EXPORTER_OTLP_ENDPOINT` et al.; `OTEL_SDK_DISABLED=true` honored
app-side, OTel-spec lenient semantics — the one named exception to the
loud-malformed convention).

## Conventions (carried over from `mcp-reasoning`, compiler-enforced)

- `#![forbid(unsafe_code)]`. No `unwrap`/`expect` in production paths — denied via
  `clippy::unwrap_used`/`expect_used`. Test modules opt out with a local
  `#[allow(...)]`. On an error: read it, fix the actual broken thing — do **not** add
  fallbacks/try-catch to hide it.
- `clippy::all` + `pedantic` + `nursery`, warnings denied in CI. The lint policy
  is declared in **both** `Cargo.toml [lints]` and the `lib.rs` preamble — change
  them together.
- Structured `tracing` to **stderr only** — stdout is the MCP JSON-RPC channel.
- Composition over trait inheritance (the `ModelClient`/`Storage`/
  `TimeProvider`/`Embedder`/`SearchProvider`/`Fetcher` seams). Every external
  dependency sits behind a mockable trait so the server tests without network
  or disk.
- Target ≤500 lines per `.rs` file for new modules.
- Off by default / gated: every new capability (network egress, code execution) is
  env-gated and off by default.

## Repo layout

```
src/
├── main.rs           # entry point: --version/--help, stderr tracing, config load
├── lib.rs            # crate docs + lint preamble
├── error.rs          # AppError, ConfigError, the outcome taxonomy (thiserror)
├── config.rs         # Config::from_env()
├── server.rs         # rmcp handler: tools, catalog gating, run_recorded (one record per call)
├── client/           # AnthropicClient, VoyageClient (embeddings), BraveClient (search)
├── modes/            # mode registry + verify (per-pass lenses, 010) / unstick / diverge (generative lenses + deterministic dedup, 012) / grounded_verify (010 abstain → 011 compute-settle: count line/byte/match over read bytes, arithmetic engine decides, executed form)
├── deterministic/    # check: translate -> execute (evalexpr/Z3) -> assembled verdict
├── memory/           # Memory/Kind/Trust, pure ranking, save/recall/forget logic
├── research/         # five-phase pipeline, hygiene fetcher, pure verdict/grounding
├── grounded/         # grounded_verify: root-confined reader + all-or-nothing assembly (008); glob/ = extended-glob engine (009)
├── schema/           # sanitizer (grammar subset) + local validator
├── storage/          # SqliteStorage (sessions, memories, invocation records)
├── telemetry.rs      # InvocationRecord + per-model pricing
└── traits/           # the mockable seams (clock/client/embedder/search/fetcher/source/storage/trajectory)
    ├── clock.rs      # TimeProvider + SystemClock
    ├── client.rs     # ModelClient — the constrained-output contract (prompt + schema → JSON)
    ├── embedder.rs   # Embedder — asymmetric document/query embeddings
    ├── search.rs     # SearchProvider — web search hits
    ├── fetcher.rs    # Fetcher — hygiene-enforced page fetches
    ├── source.rs     # SourceReader — root-confined verbatim source reads (008)
    └── storage.rs    # Storage — sessions, memories, records
docs/design/          # the full design corpus (north star)
```

## Spec-driven workflow (Spec Kit)

The `/speckit-*` skills are installed (`.claude/skills/`, `.specify/`): `constitution`
→ `specify` → `clarify` → `plan` → `tasks` → `analyze` → `implement`. The constitution
(`.specify/memory/constitution.md`) is the template placeholder — run
`/speckit-constitution` to set Parallax's governance before the first feature. Git
hooks (branch/commit) are wired via the speckit-git extension.

## Sequencing (from `SDK_LANDSCAPE.md`)

The scaffold's trait seams are the slots the SDKs fill. Rough order: **core**
(rmcp + thin Anthropic structured-outputs client behind `ModelClient` — done) →
**memory** (Voyage 4 + brute-force cosine over f32 BLOBs, the named sqlite-vec
deviation — done) → **research** (Brave provider + local extraction — done) →
**deterministic** (z3 + evalexpr + the existing validator — done; sandboxed
code-exec stays deferred, off) →
**observability** (OTLP from the first server commit). This is a recommended order,
not a mandate — confirm priorities before building.

## Active feature (Spec Kit)

<!-- SPECKIT START -->
Current feature: `013-decide-methodology` — [spec](specs/013-decide-methodology/spec.md) ·
[plan](specs/013-decide-methodology/plan.md) · [research](specs/013-decide-methodology/research.md) ·
[data model](specs/013-decide-methodology/data-model.md) · [contracts](specs/013-decide-methodology/contracts/)
<!-- SPECKIT END -->

## Working style

Build the full agreed scope; don't silently narrow it or swap the stack. If something
must be cut or deferred, say so and name it. Report outcomes straight — if a check
fails, show the output; don't minimize a real problem. No filler.
