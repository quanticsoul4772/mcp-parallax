# Feature Specification: Claude Code CLI backend

**Feature Branch**: `060-claude-cli-backend`

**Created**: 2026-11-22

**Status**: Draft

**Input**: User description: "Run every Parallax model call on the operator's own Claude Code login, so the server works without Anthropic API credit. A new backend, chosen explicitly and off by default, sends each model call through the locally installed, already signed-in Claude Code CLI in print mode with the call's output schema. Every call is isolated: no MCP servers (so Parallax cannot call itself), no tools, no user or project settings or hooks, an empty working folder, and Parallax's own system prompt. The Anthropic API key is always removed from the CLI's environment so a call can never fall back to API billing. Parallax never reads, stores or passes credentials. Cheaper default models on this backend (Sonnet for judgment, Haiku for bulk), still overridable by the existing routing settings; fewer concurrent calls by default; token usage recorded with any dollar figure marked as an estimate; failures (CLI missing, not signed in, usage limit reached, schema not met after the one retry) named with their remedy. Out of scope: the Agent SDK, OAuth token handling, changes to the other backends. A spike on 2026-11-22 passed: three decide calls returned schema-valid output in 5-10 s on the subscription with the API key removed."

## Clarifications

### Session 2026-11-22

- Q: What should an invocation record hold for cost on this backend? → A:
  The CLI's API-price estimate, marked as an estimate, not a charge; it
  shows how heavy each call is against the subscription.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Parallax works without an API key (Priority: P1)

An operator who pays for a Claude subscription but has no Anthropic API credit
selects the Claude Code backend and starts Parallax with no Anthropic API key
set. The calling model uses `decide`, `verify`, `unstick`, `diverge`,
`elicit`, `check` and `grounded_verify` and gets the same kinds of results as
on the API backend; every model call ran through the operator's own signed-in
Claude Code.

**Why this priority**: It is the feature. Without it the server is unusable
for an operator without API credit.

**Independent Test**: With the backend selected, no Anthropic API key in the
environment, and the CLI replaced by a test double, run each listed tool and
confirm a valid result and that every call reached the double with the
isolation settings and without the API key in its environment.

**Acceptance Scenarios**:

1. **Given** the Claude Code backend is selected and no Anthropic API key is
   set, **When** the server starts, **Then** it starts and lists the same
   always-on tools as the API backend.
2. **Given** the API key IS set in the operator's environment, **When** a
   model call runs, **Then** the CLI's environment does not contain it.
3. **Given** a model call, **When** the CLI runs, **Then** it runs with no MCP
   servers, no tools, no user or project settings, an empty working folder,
   and Parallax's system prompt for that call.
4. **Given** the CLI returns output that fails the call's schema, **When** the
   existing single violation-fed retry also fails, **Then** the call fails as
   a validation failure, as on the other backends.

---

### User Story 2 - The heavy tools stay usable (Priority: P1)

`research` (gated on its search key, unchanged) and the `checkpoint_turn`
review run on the same backend. Fewer model calls run at once by default on
this backend, so one research run does not start dozens of CLI processes at
the same time.

**Why this priority**: `research` is the tool that spent the API credit; it
must work on the subscription for the backend to replace the API.

**Independent Test**: With a CLI double and a search double, run `research`
and confirm it completes and that the number of simultaneous CLI calls never
exceeds the backend's default limit.

**Acceptance Scenarios**:

1. **Given** the Claude Code backend and a configured search key, **When**
   `research` runs, **Then** it completes with every model call made through
   the CLI.
2. **Given** no concurrency setting, **When** `research` runs on this
   backend, **Then** simultaneous CLI calls stay at or below this backend's
   default limit, which is lower than the API backend's.

---

### User Story 3 - Cheaper defaults and visible usage (Priority: P2)

On this backend, judgment calls default to a Sonnet model and bulk research
extraction to a Haiku model, unless the operator's existing per-site or
per-tier model settings say otherwise. Each invocation record keeps the
token usage the CLI reports.

**Why this priority**: Subscription usage limits replace dollars; defaulting
to the most expensive model would exhaust a usage window quickly.

**Independent Test**: Start with no model settings and confirm the model each
call site sends; set a per-site model and confirm it wins.

**Acceptance Scenarios**:

1. **Given** no model settings, **When** a judgment call and a research
   extraction call run, **Then** they request the Sonnet and Haiku defaults.
2. **Given** a per-site model setting, **When** that site runs, **Then** the
   setting's model is requested.
3. **Given** a completed call, **When** its record is written, **Then** it
   holds the reported input and output tokens and the CLI's API-price
   estimate, marked as an estimate rather than a charge.

---

### User Story 4 - Failures name their cause and fix (Priority: P2)

When the CLI is not installed, not signed in, out of subscription usage, or
times out, the tool call fails with an error that names the cause and what
the operator can do.

**Why this priority**: The CLI adds failure modes the API backend does not
have; an unexplained failure would look like a Parallax bug.

**Independent Test**: Use CLI doubles that exit for each cause and confirm
each error's wording and recorded outcome.

**Acceptance Scenarios**:

1. **Given** the configured CLI path does not exist, **When** the server
   starts, **Then** it fails to start with an error naming the path setting.
2. **Given** the CLI reports it is not signed in, **When** a call runs,
   **Then** the error says to sign in to Claude Code.
3. **Given** the CLI reports a usage limit, **When** a call runs, **Then** the
   error says the subscription usage limit was reached and is recorded as a
   usage-limit outcome, not a validation failure.
4. **Given** the CLI runs past the request timeout, **When** the timeout
   passes, **Then** the process is stopped and the call fails as a timeout.

---

### Edge Cases

- The CLI exits with success but omits the structured output: treated as a
  schema failure and fed to the single retry.
- The operator's own Claude Code configuration has MCP servers, hooks or
  plugins: none of them run for Parallax's calls.
- A call's input exceeds what a command-line argument can hold on Windows:
  the prompt reaches the CLI by a path with no such limit.
- `verify` with several passes starts several CLI processes at once: each
  is isolated and none shares a session.
- Effort settings on this backend: [resolved in the plan, depending on
  whether the CLI accepts an effort setting; if it does not, a configured
  effort is a startup error on this backend, not silently ignored].

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The backend MUST be selected explicitly by the existing backend
  setting; the default backend MUST stay the Anthropic API.
- **FR-002**: With this backend selected, the Anthropic API key MUST NOT be
  required at startup.
- **FR-003**: Every model call MUST run the CLI with no MCP servers, no tools,
  no user or project settings or hooks, an empty working folder, and the
  call's system prompt.
- **FR-004**: The CLI's environment MUST NOT contain the Anthropic API key or
  auth-token variables, whatever the operator's environment holds.
- **FR-005**: Parallax MUST NOT read, store, log or pass any Claude
  credential; signing in is left to the CLI.
- **FR-006**: Each call MUST pass its output schema to the CLI and validate
  the returned output with the existing local validator and single retry.
- **FR-007**: The CLI's location MUST be configurable, with a default of the
  command name on the operator's path; a configured path that does not
  resolve MUST be a startup error.
- **FR-008**: Judgment call sites MUST default to a Sonnet model and the bulk
  site to a Haiku model on this backend; existing per-site and per-tier
  model settings MUST take precedence.
- **FR-009**: The backend MUST cap simultaneous CLI calls at a default lower
  than the API backend's, overridable within the existing ceiling.
- **FR-010**: Not-installed, not-signed-in, usage-limit and timeout failures
  MUST each produce a distinct error naming the cause and remedy; the usage
  limit MUST be recorded as a usage-limit outcome (new; plan D10 — no
  rate-limit outcome existed).
- **FR-011**: A call past the request timeout MUST stop its CLI process.
- **FR-012**: Each invocation record MUST hold the token usage the CLI
  reports and its API-price estimate, marked as an estimate wherever the
  record or its telemetry is read, so it is never mistaken for a charge.
- **FR-013**: The README MUST state that this backend runs on the operator's
  own Claude Code login for their own use, and that Parallax never handles
  credentials.
- **FR-014**: Tests MUST use a CLI double; no test may start the real CLI.

### Key Entities

- **CLI call**: one model call sent through the CLI: model, system prompt,
  user prompt, output schema, isolation settings, environment without API
  credentials, timeout.
- **CLI result**: the structured output (or its absence), reported token
  usage, error flag and error text, duration.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every always-on tool returns a valid result on this backend with
  zero Anthropic API credit and no API-billed request.
- **SC-002**: A quick `research` run completes on this backend in a live run.
- **SC-003**: A single-pass tool call adds no more than 3 seconds over the
  model's own response time (spike: about 1.5 s).
- **SC-004**: Across a live run of at least 20 calls, at least 95% return
  schema-valid output on the first attempt.
- **SC-005**: No Parallax call ever runs the operator's MCP servers, hooks or
  plugins (verified by the double recording the arguments, and by a live run
  with Parallax itself configured as an MCP server).

## Assumptions

- The operator has Claude Code installed and signed in to a subscription
  that allows Claude Code use.
- The CLI's print mode with a JSON schema keeps the behavior the spike
  observed (2026-11-22): structured output in a named field, absent when the
  run ends without valid output.
- Embeddings (memory) and web search (research) keep their own providers and
  keys; only model calls move.
- Subscription usage limits, not dollars, bound this backend; the operator
  manages them.
