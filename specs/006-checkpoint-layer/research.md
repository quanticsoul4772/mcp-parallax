# Phase 0 Research: Checkpoint Layer

Decisions resolving every unknown in the plan's Technical Context. The
protocol/harness facts below were web-verified 2026-06-12 (two research
passes: MCP spec/SDK capabilities; Claude Code hook contracts and ecosystem
supervision patterns) and are recorded in the corpus amendment
(`docs/design/WATCHDOG_LAYER.md`, 2026-06-12). Sources cited there.

## D1 — Sensor plane: Claude Code hooks, `mcp_tool` handlers preferred

- **Decision**: ship hook configuration in `integrations/claude-code/` wiring
  three events to the three checkpoint tools, preferring the `mcp_tool`
  handler type (the harness calls the connected Parallax server's tool
  directly with the event payload — no shell shim, server env/DB available).
- **Rationale**: MCP offers no server-side alternative — sampling was never
  implemented by Claude Code and is deprecated in the 2026-07-28 spec RC;
  server-initiated requests outside an in-flight client request are removed;
  `includeContext` is deprecated; notifications are not surfaced to the model.
  Hooks are the documented, supported event stream with exactly the control
  surface the corpus's intervention model needs.
- **Alternatives considered**: Claude Code channels (`claude/channel`) — the
  true push channel, but research-preview + Anthropic allowlist; recorded as
  the upgrade path, not the foundation. MCP proxy/gateway — sees only other
  MCP servers' traffic, misses built-in tools (most of what the harness
  does); solves a different problem. CLAUDE.md cadence rituals — documented
  unreliable; reinstates the self-diagnosis dependency.
- **Named fallback**: if S1 shows `mcp_tool` handlers cannot express hook
  control output, use `command` handlers invoking a one-shot CLI mode of the
  `mcp-parallax` binary (stdin: hook JSON → stdout: hook JSON; shared SQLite
  via `DATABASE_PATH`). S1 then also decides how credentials reach the hook
  environment (hook-level `env`, or degraded no-recall gate).

## D2 — Boundary → hook event mapping

- **Decision**:
  - `checkpoint_action` ← `PreToolUse` (hook matcher narrowed to `Bash`,
    `Write`, `Edit` tool families; finer risk patterns applied server-side
    per FR-013). Hold = `permissionDecision: "ask"` + assembled reason.
  - `checkpoint_batch` ← `PostToolBatch` (fires once per parallel batch,
    before the next inference step — exactly the clarified cadence). Flag =
    `decision: "block"` + assembled message (model-visible feedback).
  - `checkpoint_turn` ← `Stop` (payload carries `last_assistant_message` +
    `transcript_path`). Flag = `decision: "block"` → forced continuation
    (FR-014); the once-per-turn guard uses the Stop payload's
    continuation indicator (`stop_hook_active`-style field — S1 confirms the
    exact name) plus the FR-010 cooldown.
- **Rationale**: one-to-one with the spec's three boundaries; the matcher
  narrowing means non-risky tools never even fire the gate hook (FR-013's
  zero-added-latency pass is free).
- **Alternatives**: `PostToolUse` per call (5–10× volume for no v1 detection
  gain — rejected by clarification); `UserPromptSubmit` for sycophancy
  pushback detection (deferred with the sycophancy signal).

## D3 — Trajectory access: `TrajectoryReader` seam over the transcript file

- **Decision**: hook payloads carry `transcript_path` (the session's JSONL on
  disk). A new seventh seam, `TrajectoryReader`, returns a **bounded recent
  window** of normalized entries (tool invocations with normalized inputs +
  outcomes, assistant messages); `FsTrajectoryReader` implements it with
  strict validation: canonicalized path, `.jsonl` extension, file exists,
  session id in the file matches the payload's `session_id`, read capped
  (default: last 200 entries / 2 MB, constants in `mod.rs`). Detectors never
  see the raw file.
- **Rationale**: the transcript is the only complete trajectory feed
  (extended thinking is redacted everywhere — observability floor, recorded
  in the corpus amendment); a seam keeps every detector testable without
  disk (Principle IV). The strict validation bounds the new read capability
  (Principle VI): the tools are always in the catalog, so a malicious/confused
  caller must not be able to use them to read arbitrary files.
- **Alternatives**: OTLP feed (`OTEL_LOG_RAW_API_BODIES`) — heavyweight,
  opt-in env outside our control, 1–5 s latency; rejected for v1. Passing the
  full trajectory in tool params — payload bloat on every checkpoint and the
  harness doesn't do it for us; rejected.

## D4 — Gate relevance: semantic recall under a hard budget (S2 measures)

- **Decision**: the gate's memory-contradiction signal is **deterministic**:
  risk-matched action text → query embed (`Embedder`) → existing pure cosine
  ranking over stored memories → hold iff the top hit is a constraint-kind
  memory with score ≥ τ (constant, tuned in acceptance). No model hop. The
  hold reason quotes the memory; the **user** judges actual contradiction
  (FR-011 escalate-only makes this honest). Hard budget 500 ms; on timeout or
  any error: silence + a `fail_open` record (FR-008/FR-009).
- **Rationale**: FR-003 forbids model passes at this boundary; relevance
  thresholding over verified constraint memories is checkable, fast, and its
  false-hold risk is bounded by FR-013's narrow risk matching + τ.
- **Spike S2** (gates SC-003): measure Voyage query-embed p95 from the dev
  machine. If > ~100 ms (leaving no margin in a 150 ms p95), the recorded
  options are: (a) local lexical matching (token overlap) over the small set
  of constraint memories in the gate, semantic recall reserved for
  end-of-turn; (b) amend SC-003 from measurement. Decided at spike time.
- **Alternatives**: LLM contradiction judgment in the gate — violates FR-003
  and the latency budget; rejected. Skipping memory in the gate — guts US2;
  rejected.

## D5 — Screening detectors and thresholds (constants, not config)

- **Decision**: two pure detectors over the trajectory window:
  - **Repetition**: ≥ `REPEAT_THRESHOLD` (4) invocations of the same
    normalized action (tool name + normalized input: whitespace-collapsed,
    volatile fields dropped) within the last `WINDOW_BATCHES` (10) batches.
  - **Repeated failure**: ≥ `FAILURE_THRESHOLD` (3) consecutive failing
    outcomes of the same normalized action.
  Thresholds are constants in `mod.rs` per the spec's "tunable defaults fixed
  during planning"; they move only with acceptance evidence.
- **Rationale**: matches US1's acceptance scenarios exactly (4× repetition,
  3× consecutive failure); normalization is the precision lever — exact-match
  after normalization, no similarity scoring, no false-positive surface from
  fuzzy matching.
- **Alternatives**: embedding-similarity repetition detection — probabilistic
  where deterministic suffices (Principle V); rejected for v1.

## D6 — End-of-turn review: deterministic candidate mining gates one blind hop

- **Decision**: candidates come from two pure sources: (a) memory recall —
  final message embedded, top hits of decision/constraint kind above a
  relevance floor; (b) transcript mining — earlier assistant messages in the
  window, sentence pairs with high lexical overlap plus opposing polarity
  cues. If zero candidates: silence, no model call (US3 scenario 2). Else at
  most one `checkpoint_review` hop: flat+closed schema
  `{contradicts: boolean, statement_a: string, statement_b: string, basis:
  string}`, candidates presented **stripped of surrounding self-justification**
  (FR-012, blind judging), decline-biased prompt (contradiction must be
  explicit, not a tone shift). Verdict mapping and flag wording are pure
  functions of the hop's output (FR-005).
- **Rationale**: the 005 pattern applied to the noisiest boundary — the model
  classifies, the server decides and phrases; screening keeps the hop rare
  (cost + alarm fatigue).
- **Alternatives**: NLI model locally — no clean Rust NLI path (already
  recorded in `SDK_LANDSCAPE.md`); rejected. Always-run review hop — cost on
  every turn and a standing false-positive surface; rejected.

## D7 — Cooldown and state: stateless logic over `checkpoint_records`

- **Decision**: no in-memory session state. Cooldown (FR-010): before
  delivering a flag, query `checkpoint_records` for the same `signal_key`
  (detector + normalized evidence hash) in this session within
  `COOLDOWN_WINDOW` (default 30 min, `TimeProvider`-driven); suppressed →
  verdict downgraded to silence with `suppressed: true` recorded. Forced
  continuation once-per-turn (FR-014) is enforced by the Stop payload's
  continuation indicator + cooldown.
- **Rationale**: the server may be restarted mid-session; SQLite is the only
  durable truth; stateless logic is trivially testable through `Storage` +
  `TimeProvider` mocks.

## D8 — Records: `checkpoint_records` table + standard invocation record

- **Decision**: every evaluation writes one row to a new `checkpoint_records`
  table (boundary, session id, signals evaluated, screening/review outcomes,
  signal_key(s), verdict, suppressed, fail_open, latency_ms, cost_usd) and
  one standard invocation record via `run_recorded` (existing pattern, one
  per tool call). Flag rate / hold rate / catch rate are SQL over
  `checkpoint_records` (SC-005: computable from records alone).
- **Rationale**: FR-006 demands more dimensions than `invocation_records`
  carries; extending that table would pollute every other tool's rows.

## D9 — Packaging and inertness

- **Decision**: `integrations/claude-code/` ships `hooks.json` + README with
  explicit install/uninstall steps (copy/merge into the user's hooks config;
  removal restores prior state — FR-007). No installer code in v1. The three
  tools are always in the catalog (harness-independent, spec edge case
  "direct invocation"), but with the integration absent nothing ever calls
  them: SC-006 inertness is config-presence, not server state.
- **Rationale**: minimal first non-binary deliverable; a Claude Code plugin
  manifest is a follow-up once S1 confirms the format end-to-end (named
  deferral — packaging convenience, not capability).

## Spikes (run before dependent implementation)

- **S1 — hook→tool plumbing** (gates D1/D2, the sensor plane): live Claude
  Code session against a scratch hooks config and the dev server. Verifies:
  event payload shape as delivered to an `mcp_tool` handler; mapping of tool
  results onto hook control fields; self-trigger exemption (a hook-invoked
  Parallax call must not re-fire hooks — spec edge case 1); `PostToolBatch`
  availability; `Stop.last_assistant_message` + continuation-indicator field
  names. Protocol + findings recorded in `examples/spike_hooks.md`. Fallback
  on failure: D1's named CLI one-shot mode.
- **S2 — Voyage query-embed latency** (gates D4/SC-003): 50 sequential query
  embeds from the dev machine, report p50/p95. Decision recorded in this
  file's D4.

## Resolved Technical Context items

| Unknown | Resolution |
|---|---|
| Sensor mechanism | D1 (hooks, `mcp_tool` preferred, CLI fallback) |
| Boundary→event mapping | D2 |
| Trajectory access + new capability bounds | D3 |
| Gate within latency budget | D4 + S2 |
| Detection thresholds | D5 |
| Review-hop schema + blinding | D6 |
| Cooldown/state model | D7 |
| Records schema | D8 |
| Packaging/inertness | D9 |
