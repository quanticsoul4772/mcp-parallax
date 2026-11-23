# Research: Claude Code CLI backend

Sources: the 2026-11-22 spike (`claude -p` with decide's schema, three
calls, all valid); Claude Code docs via Context7 (`code.claude.com`:
headless, cli-reference, env-vars, model-config, agent-sdk ResultMessage,
legal-and-compliance), read 2026-11-22.

## D1 — How a call reaches the CLI

**Decision**: `claude -p --output-format json --json-schema <schema>` with
the prompt written to stdin; the reply is one JSON object on stdout, and only
its `structured_output` field is used as the answer.

**Rationale**: print mode reads stdin (capped at 10 MB, well above
`INPUT_MAX_CHARS`), so the prompt never meets the Windows command-line limit
(32,767 characters). The schema stays an argument: the largest (decide at 10
options) is about 3 KB.

**Alternatives**: the Agent SDK (a Node or Python runtime inside a Rust
server; out of scope by the spec); `--input-format stream-json` (more
protocol for no gain on single-shot calls).

## D2 — Isolation

**Decision**: every call passes `--strict-mcp-config --mcp-config
{"mcpServers":{}}` (no MCP servers, so Parallax cannot reach itself),
`--tools ""` (no tools), `--setting-sources project` with an empty working
folder (no user or project settings, hooks or plugins), `--no-session-persistence`,
`--max-turns 4`, and `--system-prompt <fixed text>`.

**Rationale**: all six were used by the spike except `--no-session-persistence`
and `--max-turns`. Structured output took 2 turns in the spike; 4 leaves room
for the CLI's own structured-output retry without letting a call loop.

**Alternatives**: `--bare` (documented not to read `CLAUDE_CODE_OAUTH_TOKEN`,
and it is aimed at API-key use); `--setting-sources ""` (not documented as
accepted).

**Empty folder**: one folder per server process, created at first use under
the OS temp directory and never written to.

## D3 — Environment

**Decision**: the child inherits the server's environment minus
`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`,
`CLAUDE_CODE_USE_VERTEX`, `CLAUDE_CODE_USE_FOUNDRY` and
`CLAUDE_CODE_EFFORT_LEVEL`; `CLAUDE_CODE_EFFORT_LEVEL` is then set by
Parallax (D6). `CLAUDE_CODE_OAUTH_TOKEN` is kept: it is a subscription
credential the operator may use instead of an interactive login.

**Rationale**: the first five route billing to an API key or a cloud
account; removing them is what makes FR-004 hold whatever the operator's
environment contains. Parallax never reads their values (FR-005): it only
names them to remove.

## D4 — Model defaults

**Decision**: a new `CLAUDE_CLI_MODEL` (default `claude-sonnet-5-5`) is this
backend's default model, as `OPENAI_MODEL` is openai_compat's. The bulk tier
defaults to `claude-haiku-5-5` on this backend. Precedence, most specific
first: `PARALLAX_MODEL_<SITE>`, `PARALLAX_MODEL_<TIER>`, the backend's tier
default, the backend's default model.

**Rationale**: matches FR-008 and the existing resolution order. The tier
default is reported as its own supplier (`claude_cli bulk default`), never as
`PARALLAX_MODEL_BULK`, so `--help` and startup logs do not name a variable
the operator did not set (the 040 lesson on attribution).

**Price row**: `claude-haiku-5-5` needs a row in `telemetry.rs` (the
every-default-has-a-price test, 041). The rate is confirmed from Anthropic's
published pricing at implementation, not guessed. `claude-sonnet-5-5`
already matches the `claude-sonnet-5` row.

## D5 — Concurrency

**Decision**: `CLAUDE_CLI_CONCURRENCY` (default `4`, `1..=32`) caps live CLI
processes across the whole server with one shared semaphore in the client
pool.

**Rationale**: a per-process cap covers every tool at once (`verify`'s
parallel passes, `research`'s fan-out, a checkpoint review) instead of only
research. `RESEARCH_CONCURRENCY` keeps its meaning and default; on this
backend the lower cap governs. Spec FR-009 is met with a new variable rather
than a backend-dependent default for `RESEARCH_CONCURRENCY`, which the
config-facts scanner could not read.

## D6 — Effort

**Decision**: a routed or per-call effort is passed as
`CLAUDE_CODE_EFFORT_LEVEL=<level>` in the child environment; with no effort
it is set to `unset`, which the docs define as sending no effort level.

**Rationale**: the env variable takes precedence over `--effort` and
settings, so no stray configuration can change it; `unset` reproduces the
API backend's "no effort field" behavior. All five Parallax levels exist on
the Claude 5 models (model-config table). Unsupported pairs are rejected by
the CLI and surface through D8 with the existing 027 remedy text.

## D7 — Timeout and cancellation

**Decision**: each call is bounded by `REQUEST_TIMEOUT_MS`; on expiry, or
when the invocation is cancelled, the process is killed (`kill_on_drop`) and
the call fails as `Timeout { what: "claude CLI", ms }`.

**Rationale**: FR-011; no orphan processes holding subscription usage.

## D8 — Result classification

**Decision**, in order:

1. Spawn fails (not found) → startup check covers it (D9); at call time
   `AppError::Client` naming `CLAUDE_CLI_PATH`.
2. Exit with unparseable stdout → `AppError::Client` with the first 2,000
   characters of stderr.
3. `stop_reason == "refusal"` → `Refusal`.
4. `api_error_status == 401` or 403 → `ProviderRejected` with "sign in to
   Claude Code (`claude` then `/login`)".
5. `api_error_status == 429`, or `terminal_reason == "api_error"` with a
   usage-limit message → `UsageLimit` (new; D10), not retried.
6. Other `api_error_status` 4xx → `ProviderRejected`; 5xx → retried like the
   other backends, under `MAX_RETRIES`.
7. `subtype == "error_max_structured_output_retries"`, or success without
   `structured_output` → `ValidationFailure`, which feeds the existing single
   violation-fed retry.
8. `subtype == "error_max_turns"` → `ValidationFailure` naming the turn cap.
9. Success with `structured_output` → `Completion`.

**Rationale**: every branch reads a documented field (ResultMessage);
message text is consulted only to tell a usage limit from another API error,
and the fixture for that wording is captured in the live run (T-live).

## D9 — Startup check

**Decision**: on `claude_cli`, startup runs `<CLAUDE_CLI_PATH> --version`
with a 10 s bound; failure to run is a startup error naming
`CLAUDE_CLI_PATH`. Sign-in is not checked at startup (it would spend a model
call); a signed-out CLI fails its first call with D8.4.

## D10 — `usage_limit` outcome

**Decision**: `Outcome::UsageLimit` (`"usage_limit"`), bound by the 057 test
to the record and telemetry contracts like every other outcome. Spec FR-010
is amended from "rate-limit outcome" to "usage-limit outcome" (none existed).

## D11 — Cost

**Decision**: `Completion` gains `reported_cost_micros: Option<u64>` (the
CLI's `total_cost_usd` in millionths of a dollar; `None` on the other
backends, keeping `Eq`). When every pass of an invocation reported a cost,
the record's `cost_usd` is their sum and `cost_basis` is `cli_estimate`;
otherwise `cost_usd` is the price-table figure and `cost_basis` is
`price_table`. The span carries `parallax.cost_basis`.

**Rationale**: the clarification chose the CLI's figure, marked as an
estimate. `cost_basis` is that mark, on the record and in telemetry, so the
figure is never read as a charge. Tokens recorded per pass are the reply's
`input_tokens + cache_creation_input_tokens + cache_read_input_tokens` and
`output_tokens`.

## D12 — Terms of use

**Decision**: the README states that this backend runs the operator's own
Claude Code, signed in by the operator, for their own use; that Parallax
never reads, stores or passes credentials; and that it is off by default.

**Rationale**: Claude Code's legal page reserves subscription sign-in for
ordinary use of Claude Code and forbids third-party products from offering
Claude login or handling credentials. This backend does neither: it runs the
operator's installed Claude Code, and sign-in stays in Anthropic's flow.
