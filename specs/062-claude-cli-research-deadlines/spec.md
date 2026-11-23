# Feature Specification: Research deadlines that fit the claude_cli backend

**Feature Branch**: `062-claude-cli-research-deadlines`

**Created**: 2026-11-23

**Status**: Draft

**Input**: When `PARALLAX_BACKEND=claude_cli`, research's default per-depth
deadlines are quick 120 s (unchanged), standard 360 s (was 240 s) and deep
720 s (was 480 s). A caller's `constraints.deadline_ms` still overrides; other
backends keep today's deadlines.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Standard and deep research finish on the subscription (Priority: P1)

An operator running Parallax on their Claude subscription (`claude_cli`) asks
`research` a question at standard or deep depth. Today the run hits its
deadline and drops claims that were never checked. With this change the run
has enough time to check every claim and settle more of the question.

**Why this priority**: standard is research's default depth, so on
`claude_cli` the default run loses work today. Measured 2026-11-23 at
concurrency 4: standard took 283–302 s against 240 s, deep about 541 s against
480 s, and 26 of 180 claims were dropped in one standard run.

**Independent Test**: with the backend set to `claude_cli` and a clock that
reaches a given elapsed time, a standard run is still working at 300 s and a
deep run at 600 s; with any other backend, the same runs stop at 240 s and
480 s as today.

**Acceptance Scenarios**:

1. **Given** the `claude_cli` backend and no caller deadline, **When** research
   runs at standard depth, **Then** its deadline is 360 s.
2. **Given** the `claude_cli` backend and no caller deadline, **When** research
   runs at deep depth, **Then** its deadline is 720 s.
3. **Given** the `claude_cli` backend, **When** research runs at quick depth,
   **Then** its deadline is 120 s, as today.

---

### User Story 2 - Nothing changes anywhere else (Priority: P1)

An operator on the `anthropic` or `openai_compat` backend sees exactly
today's research behavior, and a caller that passes its own deadline gets
that deadline on every backend.

**Why this priority**: those backends meet today's deadlines; changing them
would change cost and latency for operators who did not ask for it.

**Independent Test**: research on the other backends uses 120 / 240 / 480 s,
and a caller's `deadline_ms` is used as given on all three backends.

**Acceptance Scenarios**:

1. **Given** the `anthropic` or `openai_compat` backend, **When** research runs
   at any depth with no caller deadline, **Then** the deadline is 120 / 240 /
   480 s as today.
2. **Given** any backend and a caller `deadline_ms`, **When** research runs,
   **Then** that deadline is used, unchanged.

### Edge Cases

- A caller deadline below the minimum (5000 ms) is still rejected, on every
  backend.
- A caller deadline longer than the `claude_cli` default is honored as given.
- The per-call request timeout is not changed; a single slow model call still
  fails by that timeout.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: With the `claude_cli` backend, research's default deadline MUST
  be 120 s at quick, 360 s at standard and 720 s at deep depth.
- **FR-002**: With any other backend, research's default deadlines MUST stay
  120 / 240 / 480 s.
- **FR-003**: A caller's `constraints.deadline_ms` MUST override the default on
  every backend, with today's validation.
- **FR-004**: The CHANGELOG MUST record the new `claude_cli` defaults (neither
  the tool description nor the README states the default deadlines today).
- **FR-005**: Budget tokens, concurrency, verification votes and every other
  tier setting MUST be unchanged on every backend.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A live standard-depth run on `claude_cli` at concurrency 4
  completes with `stopped_early` false on the SQLite WAL question from
  2026-11-23 (which stopped at the deadline at 240 s).
- **SC-002**: A live deep-depth run on `claude_cli` completes with
  `stopped_early` false, or, if it still stops, does so after at least 680 s
  (so the 720 s deadline applied).
- **SC-003**: On the other backends, every research tier setting is identical
  to before, shown by tests.

## Assumptions

- The 2026-11-23 timings are representative of `claude_cli` at concurrency 4;
  360 s gives standard about 20% headroom over 302 s, and 720 s gives deep
  about 33% over 541 s.
- Quick depth needs no change: 93 s against 120 s.
- A longer run uses more of the subscription's usage per run only by the work
  the old deadline cut off.
- Out of scope: concurrency defaults, per-call `claude -p` speed, budget tokens.
