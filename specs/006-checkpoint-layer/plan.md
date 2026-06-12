# Implementation Plan: Checkpoint Layer — Harness-Triggered Correctives

**Branch**: `006-checkpoint-layer` | **Date**: 2026-06-12 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/006-checkpoint-layer/spec.md`

## Summary

The watchdog re-grounded for MCP (per the 2026-06-12 amendment in
`docs/design/WATCHDOG_LAYER.md`): Claude Code hooks are the sensor/actuator
plane — shipped as an **off-by-default integration** in this repo — and
Parallax serves the brain. Three new MCP tools, one per harness boundary:
**`checkpoint_action`** (pre-action gate, risk-matched, deterministic,
hold-or-silence within a hard 500 ms budget), **`checkpoint_batch`**
(post-tool-batch feedback: loop/repeated-failure screening, pure and local),
**`checkpoint_turn`** (end-of-turn review: deterministic candidate mining
gating at most one blind, flat-schema model hop; forced-continuation flag).
Verdicts (`silence`/`flag`/`hold`) and all wording are server-assembled; the
layer never rewrites anything and fails open. Every evaluation writes one
checkpoint record (plus the standard invocation record) so flag rate, hold
rate, and catch rate vs noise are computable from storage alone — precision
(SC-001) is the make-or-break acceptance criterion.

## Technical Context

**Language/Version**: Rust (pinned stable via `rust-toolchain.toml`, MSRV 1.94)

**Primary Dependencies**: existing stack only — no new crates planned
(screening is string/JSON processing; the review hop uses `ModelClient`; gate
relevance uses `Embedder` + the existing pure cosine ranking). The sensor
plane is JSON configuration + docs, not code.

**Storage**: existing SQLite via `Storage` — new `checkpoint_records` table
(FR-006) + one invocation record per call (existing pattern). Cooldown
(FR-010/FR-014) is computed from `checkpoint_records`, not in-memory state.

**Testing**: cargo test — screening detectors against ground-truth trajectory
tables (deterministic, never mocked); gate via mocked `Embedder`/`Storage`;
review via `MockModelClient`; trajectory reading behind a new mockable seam
with tempfile fixtures; in-process rmcp integration tests; live acceptance
example replaying recorded benign + seeded-failure trajectories (SC-001/002).

**Target Platform**: cross-platform stdio binary; the sensor plane is Claude
Code (and Agent SDK) only — a named, corpus-amended limitation.

**Performance Goals**: screening pure-local (target p95 < 10 ms);
`checkpoint_action` decides within a hard 500 ms budget including the
embedding lookup, fail-open on timeout (SC-003's p95 < 150 ms is gated on
spike S2 — see research.md D4); `checkpoint_turn` bounded by the existing
`REQUEST_TIMEOUT_MS`.

**Constraints**: never rewrites (FR-002 — hook rewrite fields are never
emitted); fail-open everywhere (FR-008); silence is the default and the
cooldown suppresses repeat flags (FR-010); holds escalate to the user, never
autonomously deny (FR-011); blind judging — the review hop receives candidate
statements stripped of self-justification (FR-012); risk-matched gating only
(FR-013); forced continuation at most once per turn (FR-014).

**Scale/Scope**: four v1 signals (repetition, repeated failure,
memory-contradiction hold, end-of-turn self-contradiction); three boundaries;
everything else named-deferred (FR-004/FR-011, spec Assumptions).

## Constitution Check

*GATE: evaluated against constitution v1.0.0 before Phase 0; re-checked after
Phase 1 design.*

| Principle | Status | Notes |
|---|---|---|
| I. Design-corpus fidelity | PASS | Builds the watchdog layer exactly as re-grounded by the 2026-06-12 MCP-reality amendment to `WATCHDOG_LAYER.md` (harness hooks as sensor/actuator, server as brain, checkpoint granularity, feedback/gate only, alarm fatigue as make-or-break). Named deviations: sensor plane is client-specific config shipped outside the binary (amended in corpus); v1 catalog is 4 of the corpus's 7 signals (sycophantic flip, goal drift, hallucination/grounding, injection are named deferrals — spec FR-004); a **seventh seam** (`TrajectoryReader`) is added to keep transcript access mockable (research.md D3). |
| II. Constrained-output contract | PASS | One model hop (`checkpoint_review`) with a flat+closed schema (`{contradicts, statement_a, statement_b, basis}` — boolean/strings, no nesting). Wire results are MCP-side. Screening and the gate make no model calls at all. |
| III. Compiler discipline | PASS | No new unsafe surface; no stdout; the sensor plane is configuration, not code. |
| IV. Seams + tests | PASS | `ModelClient` (review hop), `Embedder` (gate/turn recall), `Storage` (records, memories), `TimeProvider` (cooldown windows), new `TrajectoryReader` (transcript window). Screening detectors are pure functions tested against ground-truth trajectory tables. |
| V. Deterministic over probabilistic | PASS | Pre-action and post-batch boundaries decide 100% deterministically (FR-003). The model hop exists only at end-of-turn, only after deterministic screening produces candidates, and only classifies — verdict mapping and wording are pure functions (FR-005). |
| VI. Capabilities off by default | PASS, one named capability | The sensor plane is an explicit install (off by default, FR-007; uninstall restores prior state). New capability: **read-only access to harness transcript files** — bounded window, strict path validation (canonicalized, `.jsonl`, session-matched), nothing beyond assembled evidence flows back; recorded in research.md D3 and data-model.md §5. The three tools are otherwise pure/in-process + existing-credential calls. |
| VII. Simplicity / ≤500-line modules | PASS | Module split below; v1 signals minimal; thresholds are constants, not a config subsystem. |

**Post-Phase-1 re-check**: PASS — the contracts introduce no new violations.

## Project Structure

### Documentation (this feature)

```text
specs/006-checkpoint-layer/
├── plan.md              # This file
├── research.md          # Phase 0 output (decisions D1–D9, spikes S1–S2)
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   ├── checkpoint_action.tool.json
│   ├── checkpoint_batch.tool.json
│   └── checkpoint_turn.tool.json
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── checkpoint/
│   ├── mod.rs            # constants (windows, thresholds, budgets, cooldown), Verdict/Signal types
│   ├── contract.rs       # wire types: per-boundary params + CheckpointResult (MCP-side)
│   ├── trajectory.rs     # bounded transcript-window model: the entries detectors consume
│   ├── screen.rs         # deterministic detectors: repetition, repeated failure (pure)
│   ├── gate.rs           # risk-pattern match + constraint-memory relevance → hold (deterministic)
│   ├── review.rs         # end-of-turn: candidate mining (pure) + the one flat-schema model hop
│   └── run.rs            # per-boundary orchestration: validate → screen → (review) → assemble → record
├── traits/
│   └── trajectory.rs     # TrajectoryReader seam (+ FsTrajectoryReader: validated, bounded JSONL read)
├── storage/              # + checkpoint_records table + queries (cooldown lookup, rate metrics)
└── server.rs             # + three #[tool]s via run_recorded (always in catalog; harness-independent)

integrations/claude-code/ # the sensor plane (off by default — user installs explicitly)
├── hooks.json            # PreToolUse / PostToolBatch / Stop → checkpoint tools (S1 fixes exact shape)
└── README.md             # install/uninstall, what each hook does, fail-open behavior

tests/integration.rs      # + catalog, per-boundary round trips, fail-open parity, record assertions
examples/spike_hooks.md   # S1 protocol + findings (hook→tool plumbing; manual, live Claude Code)
examples/acceptance_checkpoint.rs  # SC-001/002/003/005/007 over recorded + seeded trajectories
```

**Structure Decision**: single crate, new `checkpoint/` module family mirroring
`deterministic/` (contract split, pure detectors, one orchestrator). The
sensor plane lives in `integrations/claude-code/` — the project's first
non-binary deliverable, named in the corpus amendment; it contains no logic
beyond wiring hook events to the three tools.

## Complexity Tracking

No constitution violations to justify. Two named engineering risks, both
spiked before dependent work (research.md):

- **S1 — hook→tool plumbing** (gates the sensor plane): whether the `mcp_tool`
  hook handler delivers the documented event payload as tool input and maps
  tool results onto hook control fields (`permissionDecision`,
  `decision: "block"`); whether a hook-invoked Parallax call is exempt from
  re-triggering hooks (the self-trigger edge case); whether `PostToolBatch`
  and `Stop.last_assistant_message` behave as documented. Named fallback if
  `mcp_tool` cannot express hook outputs: `command` handlers invoking a
  one-shot CLI mode of the binary (shared SQLite; credential availability in
  the hook environment becomes S1's second question).
- **S2 — gate latency** (gates SC-003): p95 of a single Voyage query embed
  from the dev machine. If the embed cannot fit a 150 ms p95, the recorded
  options are (a) local lexical constraint matching in the gate with semantic
  recall reserved for end-of-turn, or (b) amending SC-003 from measurement —
  decided at spike time, not silently.
