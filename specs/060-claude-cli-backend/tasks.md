# Tasks: Claude Code CLI backend

**Input**: Design documents from `specs/060-claude-cli-backend/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/claude-cli-call.md, quickstart.md

**Tests**: required. Every behavior change gets a test written to fail first
(project rule). No test starts the real CLI (FR-014); the process runner is
covered by the live acceptance run.

**Organization**: by user story. US1 and US2 are P1; US3 and US4 are P2. US2,
US3 and US4 build on US1's client.

## Phase 1: Setup

- [X] T001 Add tokio's `process` feature in `Cargo.toml` (no new crate)

## Phase 2: Foundational (blocks every story)

- [X] T002 Write failing unit tests in `src/traits/runner.rs` for the `CliRunner` seam using its mock: a `CliRequest` (program, arguments, stdin text, variables to remove, variables to set, working folder, timeout) returns a `CliOutput` (exit code, stdout, stderr) or a timeout error
- [X] T003 Implement `CliRunner`, `CliRequest`, `CliOutput` in `src/traits/runner.rs` and the process-backed `TokioCliRunner` (piped stdin/stdout/stderr, never inherited; `kill_on_drop`; timeout kills the child) in `src/client/claude_cli.rs`; export from `src/traits/mod.rs`. `TokioCliRunner` gets its own tests, seen failing first, against stand-in programs, never `claude` (analyze A1): an environment print (`cmd /C set` on Windows, `env` elsewhere) shows the removed variables absent and the set ones present; `rustc --version` shows stdout captured; a 30 s sleep (`ping -n 30 127.0.0.1` / `sleep 30`) under a short timeout returns the timeout error and leaves no process running
- [X] T004 Add `reported_cost_micros: Option<u64>` to `Completion` in `src/traits/client.rs`, `None` at every existing construction site (both clients, mocks, tests), so the crate builds unchanged
- [X] T005 Add `CLAUDE_CLI_PATH` (default `claude`), `CLAUDE_CLI_MODEL` (default `claude-sonnet-5-5`), `CLAUDE_CLI_CONCURRENCY` (default 4, `1..=32`, out of range or unparseable is a startup error naming it) and `DefaultVar::ClaudeCliModel` to `src/config.rs` and `src/routing.rs`, tests first; update `src/config_facts/`, `--help` in `src/main.rs` and README's configuration rows (exact text approved first) so their binding tests pass. The backend value itself lands with its client in T011, so no match arm is ever a stub

**Checkpoint**: the seam and settings exist; no backend uses them yet.

## Phase 3: User Story 1 — Parallax works without an API key (P1)

**Goal**: every always-on tool returns a valid result through an isolated `claude -p` call.

**Independent test**: with `PARALLAX_BACKEND=claude_cli`, no API key and a runner double, every listed tool returns a valid result and every call reached the double per the contract.

- [X] T006 [US1] Write failing tests in `src/client/claude_cli/invocation.rs`: arguments exactly as `contracts/claude-cli-call.md` §Arguments, in order; the prompt is stdin and in no argument; the six variables are removed; `CLAUDE_CODE_EFFORT_LEVEL` is the routed level or `unset`; the working folder is the process's empty folder; variables are removed by name only, and a call's debug log contains no environment values (FR-005, analyze A5)
- [X] T007 [US1] Implement argument and environment building in `src/client/claude_cli/invocation.rs` (pure)
- [X] T008 [US1] Write failing tests in `src/client/claude_cli/classify.rs` for the non-error rows of the contract table: success with `structured_output` → `Completion` (input tokens = input + cache creation + cache read; cost micros = round(`total_cost_usd` × 10^6)); success without `structured_output`, `error_max_structured_output_retries` and `error_max_turns` → `ValidationFailure`; stdout not one JSON object → `Client` with stderr capped at 2,000 characters
- [X] T009 [US1] Implement those rows in `src/client/claude_cli/classify.rs` (pure)
- [X] T010 [US1] Write failing tests for `ClaudeCliClient` in `src/client/claude_cli.rs` with a runner double: `complete()` sends the invocation and returns the classified result; `build_model_client` in `src/client/mod.rs` returns it for `Backend::ClaudeCli` for every routed `(model, effort)`
- [X] T011 [US1] Add `Backend::ClaudeCli` (`claude_cli`) to `src/config.rs`, tests first: `ANTHROPIC_API_KEY` not required on it, `default_model()` returns `CLAUDE_CLI_MODEL`, routing falls through to `DefaultVar::ClaudeCliModel`; implement `ClaudeCliClient` (`ModelClient`), the per-process empty folder, and the `Backend::ClaudeCli` arm in `src/client/mod.rs`
- [X] T012 [US1] Write failing integration tests in `tests/integration.rs`: a server on `claude_cli` with a runner double and no API key lists the same always-on tools as `anthropic` (spec US1 scenario 1); `decide`, `verify`, `unstick`, `diverge`, `elicit`, `check` and `grounded_verify` each return a valid result; a first reply failing the schema then a valid one completes through the existing single retry (scenario 4); make them pass

**Checkpoint**: US1 complete against doubles.

## Phase 4: User Story 2 — The heavy tools stay usable (P1)

**Goal**: research and the checkpoint review run on the backend under a server-wide process cap.

**Independent test**: with blocking runner doubles, simultaneous CLI calls never exceed `CLAUDE_CLI_CONCURRENCY`, across tools and models.

- [X] T013 [US2] Write failing tests in `src/client/pool.rs`: with a runner double that holds each call, N parallel `complete()` calls across two routed models never exceed `CLAUDE_CLI_CONCURRENCY` live calls, and all N complete
- [X] T014 [US2] Implement one semaphore shared by every `ClaudeCliClient` the pool builds (research.md D5)
- [X] T015 [US2] Write failing integration tests: `research` on `claude_cli` (runner double + Exa double) completes with every model call through the runner; the `checkpoint_turn` review completes; make them pass

**Checkpoint**: US2 complete.

## Phase 5: User Story 3 — Cheaper defaults and visible usage (P2)

**Goal**: Sonnet and Haiku defaults on this backend; each record holds tokens and the CLI's estimate, marked as one.

**Independent test**: with no model settings the judgment and bulk sites request Sonnet and Haiku; a completed call's record has `cost_basis = cli_estimate`.

- [ ] T016 [US3] Write failing tests in `src/routing.rs`: on `claude_cli` with no settings, judgment sites resolve to `CLAUDE_CLI_MODEL` and the bulk site to `claude-haiku-5-5`; `PARALLAX_MODEL_<SITE>` and `PARALLAX_MODEL_BULK` win; the bulk default's supplier is reported as `claude_cli bulk default`, never `PARALLAX_MODEL_BULK`; other backends unchanged
- [ ] T017 [US3] Implement the backend tier default in `src/routing.rs` and its `--help` line in `src/main.rs`
- [ ] T018 [US3] Add the `claude-haiku-5-5` price row in `src/telemetry.rs` at Anthropic's published rate (source cited in the comment); extend the every-default-has-a-price test to the `claude_cli` defaults, seen failing first
- [ ] T019 [US3] Write failing tests: an invocation whose passes all reported a cost records their sum with `cost_basis = cli_estimate`; any pass without one records the price-table figure with `price_table`; `cost_basis` round-trips through `src/storage/sqlite.rs` (nullable column; older rows read as `NULL`); the span carries `parallax.cost_basis`
- [ ] T020 [US3] Implement `cost_basis` in `src/telemetry.rs`, `src/storage/sqlite.rs` and `src/observability.rs`; update `specs/007-observability-layer/contracts/telemetry.md` and the record contract so their binding tests pass

**Checkpoint**: US3 complete.

## Phase 6: User Story 4 — Failures name their cause and fix (P2)

**Goal**: each CLI failure is classified with its cause and remedy.

**Independent test**: runner doubles for each cause produce the contract's error and outcome.

- [ ] T021 [US4] Write failing tests in `src/client/claude_cli/classify.rs` and `src/client/claude_cli.rs` for the error rows: `refusal`; 401/403 → `provider_rejected` with the sign-in remedy; 429 or a usage-limit `api_error` → `usage_limit`, not retried; other 4xx → `provider_rejected`; 5xx retried under `MAX_RETRIES`; a call past `REQUEST_TIMEOUT_MS` → `Timeout` naming the CLI
- [ ] T022 [US4] Add `Outcome::UsageLimit` and `AppError::UsageLimit` in `src/error.rs`, bound by the existing outcome-contract test to the record and telemetry contracts (contract files updated); implement the error rows and the 5xx retry
- [ ] T023 [US4] Write failing tests for the startup check: on `claude_cli`, a `CLAUDE_CLI_PATH` that fails `--version` through the runner is a startup error naming the variable; success logs the version
- [ ] T024 [US4] Implement the startup check (10 s bound) where the server is built in `src/main.rs` / `src/server.rs`

**Checkpoint**: US4 complete; all stories pass against doubles.

## Phase 7: Polish & Cross-Cutting

- [ ] T025 Dated amendments to `docs/design/NEW_SERVER_DESIGN.md` §3.1 and `docs/design/SDK_LANDSCAPE.md` §core; README backend section with the research.md D12 statement; `CHANGELOG.md` `[Unreleased]` (exact text of each approved first)
- [ ] T026 Write `examples/acceptance_claude_cli.rs` (quickstart §Live acceptance): every always-on tool once and one quick `research` question through the server; then 20 `decide`-schema calls made directly through `ClaudeCliClient` with the real runner wrapped by a recording runner, so each call's wall time can be compared with the reply's `duration_api_ms` (SC-003) and its first attempt checked by the local validator (SC-004) (analyze A2); prints time, overhead, first-attempt pass, tokens and estimate per call, and fails if any record shows a recursive Parallax call
- [ ] T027 Independent review: a fresh subagent given only the diff, told to trace every caller (brief approved first); findings asked one by one
- [ ] T028 Live acceptance (quickstart.md) with `ANTHROPIC_API_KEY` unset and Parallax configured as an MCP server in Claude Code; record results in quickstart.md (SC-001 to SC-005)
- [ ] T029 Update the CLAUDE.md active-feature block for 060 (exact text approved first)

## Dependencies & Execution Order

- T001 → T002/T003; T004 and T005 independent of T002/T003.
- Foundational before every story. US1 (T006–T012) before US2–US4.
- US2, US3 and US4 are independent of one another after US1.
- Polish after all stories; T026 before T028; T027 before T028; T029 last.

## Parallel Opportunities

- T004 and T005 alongside T002–T003 (different files).
- After US1: US2, US3, US4 can proceed in parallel (pool, routing/telemetry, error/classify).
- T006 and T008 (separate pure modules).

## Implementation Strategy

MVP is US1: with it alone, the always-on tools work on the subscription.
US2 makes research safe to run; US3 and US4 make usage and failures legible.
Each story is committed with the gate green before the next starts.
