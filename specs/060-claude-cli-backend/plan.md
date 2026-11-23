# Implementation Plan: Claude Code CLI backend

**Branch**: `060-claude-cli-backend` | **Date**: 2026-11-22 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/060-claude-cli-backend/spec.md`

## Summary

A third value of `PARALLAX_BACKEND`, `claude_cli`, fills the existing
`ModelClient` seam with a client that runs each model call as one isolated
`claude -p` process on the operator's own signed-in Claude Code. The prompt
goes in on stdin, the call's sanitized schema goes in `--json-schema`, and
the result is read from the JSON reply's `structured_output`. The process
runs with no MCP servers, no tools, no user or project settings, no session
persistence, an empty working folder and a fixed system prompt, and with
every API-billing credential variable removed from its environment. The
existing validator, single retry, pool, routing and records are reused; the
new parts are the client, its result classification, three settings, the
backend's model defaults, a process cap, a new `usage_limit` outcome, and a
cost basis on the record. A spike (2026-11-22) passed: three `decide` calls,
all schema-valid, 5-10 s each, on the subscription with the API key removed.

## Technical Context

**Language/Version**: Rust (toolchain 1.100.0, MSRV 1.94)

**Primary Dependencies**: tokio (adds the `process` feature), serde_json,
existing crates only; no new crate

**Storage**: SQLite — one nullable column on `invocation_records`
(`cost_basis`)

**Testing**: `cargo test`; the CLI is replaced by a `CliRunner` trait double
(no test starts a process); one ignored-by-default live example

**Target Platform**: Windows, macOS, Linux (wherever Claude Code runs)

**Project Type**: single Rust binary (MCP server over stdio)

**Performance Goals**: CLI overhead ≤ 3 s per call over model time (SC-003;
spike: about 1.5 s)

**Constraints**: stdout stays the MCP channel (the child's stdout is captured,
never inherited); a call past `REQUEST_TIMEOUT_MS` kills its process; the
prompt never travels on the command line (Windows argument limit)

**Scale/Scope**: one new client module (~400 lines) plus config, routing,
telemetry, storage and error edits; 12 call sites unchanged

## Constitution Check

| Principle | Status | Note |
|---|---|---|
| I. Design-corpus fidelity | Amend | `NEW_SERVER_DESIGN.md` §3.1 (BYOM) and `SDK_LANDSCAPE.md` §core gain a dated amendment naming the CLI backend and why (no API credit). |
| II. Constrained output | Deviation, named | The CLI validates `structured_output` after generation rather than constraining decoding (its docs: absent when the run ends without valid output). Parallax's local validator and single retry already cover backends that do not enforce the format (openai_compat JSON-object and prompt-only rungs); this backend joins that class. No free-text parsing: only the `structured_output` field is read. |
| III. Compiler discipline | Pass | No `unwrap`; the child's stdout/stderr are piped, never inherited. |
| IV. Seams and tests | Pass | Process execution sits behind a new `CliRunner` trait; every test uses a double. |
| V. Deterministic over probabilistic | Pass | Result classification is by fields (`subtype`, `stop_reason`, `api_error_status`, `terminal_reason`), not by asking a model. |
| VI. Off by default | Pass | Shell-out happens only when `PARALLAX_BACKEND=claude_cli`; the default stays `anthropic`. |
| VII. Simplicity | Pass | No new crate; the client stays under 500 lines by keeping argument building and result classification in a sibling module. |

## Project Structure

### Documentation (this feature)

```text
specs/060-claude-cli-backend/
├── spec.md
├── plan.md              # this file
├── research.md          # decisions D1–D12
├── data-model.md
├── quickstart.md
├── contracts/
│   └── claude-cli-call.md   # the exact invocation and result classification
└── checklists/requirements.md
```

### Source Code (repository root)

```text
src/
├── client/
│   ├── claude_cli.rs         # NEW: ClaudeCliClient (ModelClient), process cap
│   ├── claude_cli/
│   │   ├── invocation.rs     # NEW: argument + environment building (pure)
│   │   └── classify.rs       # NEW: reply → Completion | AppError (pure)
│   └── mod.rs                # build_model_client: Backend::ClaudeCli arm
├── traits/
│   └── runner.rs             # NEW: CliRunner seam (spawn, stdin, timeout)
├── config.rs                 # Backend::ClaudeCli; CLAUDE_CLI_* settings; key not required
├── routing.rs                # backend tier defaults (bulk → Haiku on claude_cli)
├── error.rs                  # Outcome::UsageLimit, AppError::UsageLimit
├── telemetry.rs              # cost basis; claude-haiku-5-5 price row
├── storage/sqlite.rs         # cost_basis column
└── observability.rs          # parallax.cost_basis attribute
examples/
└── acceptance_claude_cli.rs  # NEW: live run (SC-001..SC-005)
```

**Structure Decision**: the CLI client lives beside the other provider
clients in `src/client/`, built through the single `build_model_client`
switch, so no call site changes.

## Complexity Tracking

| Deviation | Why needed | Simpler alternative rejected because |
|---|---|---|
| Post-generation schema check instead of constrained decoding (Principle II) | The CLI exposes no constrained-decoding control | Staying API-only leaves the operator without a working server (the reason for the feature) |
| New outcome `usage_limit` (spec FR-010 said "rate-limit outcome"; none exists) | A subscription usage limit lasts hours; it is neither a retryable 429 nor a provider rejection | Mapping to `retries_exhausted` would retry pointlessly and misname the cause; `provider_rejected` is defined by 057 as 4xx other than 429 |
