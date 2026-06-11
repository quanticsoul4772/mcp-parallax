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

**Status: scaffold.** Foundation only — configuration, error types, the mockable
trait boundaries (`TimeProvider`, `ModelClient`, `Storage`), and stderr logging. The
transport and tool surface are **not yet wired**. Don't describe the server as
working; it doesn't serve tools yet.

## The design is the source of truth

Read these before proposing architecture. The master doc indexes the rest.

- [`docs/design/NEW_SERVER_DESIGN.md`](docs/design/NEW_SERVER_DESIGN.md) — **start here.**
  Value model, the four-layer architecture, failure-mode catalog, primitives,
  routing, what's validated, what stops existing.
- [`docs/design/SDK_LANDSCAPE.md`](docs/design/SDK_LANDSCAPE.md) — the chosen SDK/crate
  stack per layer (web-grounded, 2026-06), with versions and caveats.
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

# Full gate before every commit (also the /validate command):
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test
```

Toolchain is pinned (`rust-toolchain.toml`: stable + clippy/rustfmt/llvm-tools).
MSRV is 1.94 (CI verifies it). `pre-commit` hooks mirror the gate — `pre-commit install`.

## Conventions (carried over from `mcp-reasoning`, compiler-enforced)

- `#![forbid(unsafe_code)]`. No `unwrap`/`expect` in production paths — denied via
  `clippy::unwrap_used`/`expect_used`. Test modules opt out with a local
  `#[allow(...)]`. On an error: read it, fix the actual broken thing — do **not** add
  fallbacks/try-catch to hide it.
- `clippy::all` + `pedantic` + `nursery`, warnings denied in CI.
- Structured `tracing` to **stderr only** — stdout is the MCP JSON-RPC channel.
- Composition over trait inheritance (the `ModelClient`/`Storage`/`TimeProvider`
  seams). Every external dependency sits behind a mockable trait so the server tests
  without network or disk.
- Target ≤500 lines per `.rs` file for new modules.
- Off by default / gated: every new capability (network egress, code execution) is
  env-gated and off by default.

## Repo layout

```
src/
├── main.rs           # entry point: --version/--help, stderr tracing, config load
├── lib.rs            # crate docs + lint preamble
├── error.rs          # AppError, ConfigError (thiserror)
├── config.rs         # Config::from_env()
└── traits/           # the three mockable seams
    ├── clock.rs      # TimeProvider + SystemClock
    ├── client.rs     # ModelClient — the constrained-output contract (prompt + schema → JSON)
    └── storage.rs    # Storage — session persistence
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
(rmcp + thin Anthropic structured-outputs client behind `ModelClient`, makes
`complete(prompt, schema)` real) → **memory** (Voyage 4 + sqlite-vec; spike the
sqlx-loading caveat) → **research** (Brave provider + local extraction) →
**deterministic** (z3 + validator first; sandboxed code-exec optional, off) →
**observability** (OTLP from the first server commit). This is a recommended order,
not a mandate — confirm priorities before building.

## Working style

Build the full agreed scope; don't silently narrow it or swap the stack. If something
must be cut or deferred, say so and name it. Report outcomes straight — if a check
fails, show the output; don't minimize a real problem. No filler.
