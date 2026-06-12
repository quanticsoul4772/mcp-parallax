# Feature Specification: Checkpoint Layer — Harness-Triggered Correctives

**Feature Branch**: `006-checkpoint-layer`

**Created**: 2026-06-12

**Status**: Draft

**Input**: User description: "Checkpoint layer (the watchdog re-grounded for MCP, per the 2026-06-12 amendment in WATCHDOG_LAYER.md): harness-triggered checkpoints that fire correctives the model can't self-diagnose to call. Claude Code hooks are the sensor/actuator plane (shipped as an off-by-default plugin: PreToolUse gate, PostToolUse/Stop feedback, transcript-fed); Parallax serves the brain — cheap deterministic heuristics (loop/repeat detection, contradiction-vs-memory via recall) gating one constrained model hop, server-assembled verdicts, one invocation record per checkpoint so catch-rate vs noise is measurable. Never rewrites; flags or gates only. Precision (alarm fatigue) is the make-or-break acceptance criterion."

## The problem this solves

Every existing Parallax corrective assumes the calling model recognizes it needs
help and calls the tool. The worst failures — looping on a broken approach,
contradicting an earlier decision, drifting from stated constraints — are
invisible from inside the model's own context: the model that is failing is the
one that would have to notice. This layer removes that self-diagnosis
dependency. The user's coding harness triggers a checkpoint at natural
boundaries (before a risky action, after tool activity, at end of turn); an
independent reviewer examines the bare trajectory against what is known to be
true and — only when a real signal fires — hands the model a specific,
actionable flag, or holds a pending action for the user's confirmation. The
model fixes its own work; the checkpoint never rewrites anything.

Silence is the default and the discipline: a checkpoint layer that cries wolf
is worse than none, so precision is the make-or-break acceptance criterion.

## Clarifications

### Session 2026-06-12

- Q: How often does the post-activity checkpoint run? → A: Once per completed
  tool batch, before the model's next inference step — loop-visible
  granularity at volume proportional to inference steps, screening pure-local.
- Q: Which pending actions does the pre-action gate evaluate? → A: Only
  actions matching a configurable risk-pattern set (default: consequential
  shell commands and writes — deploys, pushes, deletes, config changes);
  everything else passes with zero added latency.
- Q: How is a real end-of-turn flag delivered? → A: Forced continuation — the
  turn does not end until the model addresses the flag (at most one forced
  continuation per turn; cooldown prevents loops), so the correction lands
  before the user relies on the answer.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Catch the loop the model can't see (Priority: P1)

A developer's coding agent has tried the same failing fix four times in a row,
re-reading the same file and re-running the same failing command with trivial
variations. The agent cannot see its own loop. At the next checkpoint after
tool activity, the layer detects the repetition pattern in the recent
trajectory and feeds back one specific flag: what repeated, how many times, and
that a different approach (or the `unstick` corrective) is warranted. The agent
breaks the loop on its own.

**Why this priority**: Looping and repeated failure are the most common,
cheapest-to-detect, and least self-diagnosable trajectory failures — detection
needs no judgment call and no model pass, so this story proves the whole
sensor→brain→feedback loop with purely deterministic logic. It is the MVP.

**Independent Test**: Feed the checkpoint a recorded trajectory containing a
known loop (same tool + similar input N times, or N consecutive failures of the
same command) and a set of benign trajectories. The loop is flagged with a
specific description; the benign trajectories produce silence.

**Acceptance Scenarios**:

1. **Given** a trajectory where the same tool was invoked with
   near-identical input 4 times in the recent window, **When** the
   post-batch checkpoint runs, **Then** the verdict is a flag naming the
   repeated action and count, and the feedback is delivered to the model.
2. **Given** a trajectory where the same command failed 3 consecutive times,
   **When** the checkpoint runs, **Then** the verdict is a flag naming the
   failing command and suggesting a change of approach.
3. **Given** a benign trajectory (varied tools, no repetition, progress
   visible), **When** the checkpoint runs, **Then** the verdict is silence —
   nothing is injected into the model's context.
4. **Given** any checkpoint evaluation, **When** it completes, **Then** exactly
   one record exists with the signal(s) evaluated, the verdict, and timing.

---

### User Story 2 - Hold a risky action that contradicts the record (Priority: P2)

The user has a stored, verified decision in Parallax memory: "deployments go
through staging first — never deploy straight to production." Mid-session, the
agent — having lost that constraint to context compaction — is about to run a
direct production deploy. The pre-action checkpoint recalls stored memories
relevant to the pending action, detects the contradiction, and holds the action
for the user's explicit confirmation, citing the stored decision it conflicts
with. The user denies it; the agent course-corrects.

**Why this priority**: This is the gate intervention — the highest-stakes
corrective (irreversible actions) — and the first one that pairs checkpoints
with the memory layer (memory holds what should be true; the checkpoint
enforces it). It depends on the same evaluation pipeline as US1.

**Independent Test**: Store a constraint memory, present a pending action that
contradicts it and one that doesn't; the contradicting action is held with the
memory cited, the benign action passes through with no added friction.

**Acceptance Scenarios**:

1. **Given** a stored constraint memory and a pending action that contradicts
   it, **When** the pre-action checkpoint runs, **Then** the action is held for
   user confirmation with the conflicting memory quoted in the reason.
2. **Given** a pending action with no relevant stored memories, **When** the
   pre-action checkpoint runs, **Then** the action proceeds with no added
   friction and no visible output.
3. **Given** the pre-action checkpoint cannot complete within its time budget
   (or the layer is unreachable), **When** the harness awaits the verdict,
   **Then** the action proceeds as if no checkpoint existed (fail-open) and the
   failure is recorded.
4. **Given** a held action, **When** the user confirms it, **Then** it executes
   unmodified — the layer never alters what the agent proposed.

---

### User Story 3 - End-of-turn review against the trajectory (Priority: P3)

An agent finishes a long turn having quietly contradicted itself: an answer
stated earlier in the session was reversed without any new evidence appearing
in between. At the end-of-turn checkpoint, cheap screening finds candidate
tension between the final message and earlier committed statements or stored
decisions; only then does a single independent, blind review pass examine the
candidates and confirm one real contradiction. The model receives a flag naming
both statements; it reconciles them before the user acts on the wrong one.

**Why this priority**: The end-of-turn review is where deeper signals
(self-contradiction, unjustified reversal) live, and where the one constrained
model pass is permitted. It carries the highest noise risk, so it ships last —
on top of an already-measured precision baseline from US1/US2.

**Independent Test**: Run the end-of-turn checkpoint over recorded sessions
with a seeded contradiction and over benign sessions; the contradiction is
flagged with both statements cited; benign sessions stay silent; no review pass
runs when screening finds no candidates.

**Acceptance Scenarios**:

1. **Given** a session whose final message contradicts an earlier stored
   decision or committed statement, **When** the end-of-turn checkpoint runs,
   **Then** a flag citing the two conflicting statements is delivered by
   forced continuation — the turn does not end until the model addresses it.
2. **Given** a benign session, **When** the end-of-turn checkpoint runs,
   **Then** no review pass is invoked and the verdict is silence.
3. **Given** screening fires but the review pass judges the candidates
   consistent, **When** the checkpoint completes, **Then** the verdict is
   silence and the record shows screening fired with a cleared review.

---

### Edge Cases

- **Self-triggering**: checkpoint evaluations themselves invoke tools; the
  sensor plane MUST exclude this layer's own activity so a checkpoint can
  never trigger a checkpoint (no feedback loop).
- **Trajectory unavailable**: the session transcript is missing, truncated,
  or unreadable → the checkpoint returns silence and records the condition;
  it never blocks the session.
- **Oversized trajectory**: evaluation reads a bounded recent window, never
  the unbounded full history.
- **Empty memory store / memory layer disabled**: memory-paired signals are
  silently inactive; activity signals still work.
- **Direct invocation**: the checkpoint capability called directly (not via
  the harness) behaves identically — it is an ordinary capability with no
  hidden coupling to the sensor plane.
- **Repeated identical flags**: the same unresolved signal at consecutive
  checkpoints must not re-flag every time (escalating noise); a flagged
  signal is suppressed for a cooldown window once delivered.
- **Sessions from other tools**: harnesses other than the supported one never
  see this layer (no sensor plane installed); the server remains fully
  functional without it.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide checkpoint evaluation invocable at three
  harness boundaries: before a pending action (gate-capable), after each
  completed tool batch — once per group of tool calls, before the model's next
  inference step (feedback-capable) — and at end of turn (feedback-capable).
  Each accepts the harness event payload plus access to the session
  trajectory.
- **FR-002**: Checkpoint verdicts MUST be limited to: **silence** (no output
  reaches the model or user), **flag** (a specific, actionable observation
  delivered to the model), or **hold** (a pending action is paused for user
  confirmation with a stated reason). The system MUST NOT modify, rewrite, or
  suppress the model's output, the action's inputs, or any tool's results.
- **FR-003**: Every checkpoint MUST run cheap deterministic screening first;
  the single independent review pass (at most one per checkpoint) MAY run only
  when screening produces candidates, and only at the end-of-turn boundary.
  The pre-action and post-batch boundaries MUST decide deterministically.
- **FR-004**: The v1 signal catalog is: (a) repetition/looping — near-identical
  actions repeated within the recent window; (b) repeated failure — the same
  action failing consecutively; (c) contradiction with stored memory — a
  pending action or committed statement conflicting with a verified stored
  decision/constraint, surfaced via semantic recall; (d) end-of-turn
  self-contradiction — the turn's conclusion conflicting with an earlier
  committed statement without intervening evidence. All other signals from the
  design corpus (sycophantic flip, goal drift, hallucination/grounding,
  injection detection) are named deferrals.
- **FR-005**: Verdict selection and all user/model-facing wording MUST be
  assembled by the system from detected signals; the review pass classifies
  candidates but never decides or phrases the verdict.
- **FR-006**: Every checkpoint evaluation MUST produce exactly one invocation
  record capturing the boundary, signals evaluated, screening and review
  outcomes, verdict, latency, and cost — sufficient to compute flag rate,
  hold rate, and catch rate vs noise over time without any additional
  instrumentation.
- **FR-007**: The sensor plane MUST be packaged as an installable add-on that
  is off by default. With it absent or disabled there MUST be zero behavior
  change anywhere: no new latency, no context injection, no catalog change.
  Installing it MUST NOT require modifying the server; uninstalling MUST
  restore the prior state completely.
- **FR-008**: The layer MUST fail open: any checkpoint error, timeout, or
  unavailability lets the session proceed exactly as if the layer were absent,
  while the failure is recorded. A broken checkpoint layer MUST NOT be able to
  block or degrade the user's session.
- **FR-009**: Pre-action checkpoints sit in the action's critical path and
  MUST decide within a hard time budget (default 500 ms, screening-only);
  post-batch and end-of-turn checkpoints MUST NOT extend the critical path
  beyond their own bounded evaluation.
- **FR-013**: The pre-action gate MUST evaluate only pending actions matching
  a configurable risk-pattern set (default: consequential shell commands and
  writes — deploys, pushes, deletes, configuration changes). Non-matching
  actions MUST pass through with no evaluation and no added latency.
- **FR-010**: A delivered flag MUST NOT be re-delivered for the same unresolved
  signal within its cooldown window; holds are never rate-limited.
- **FR-014**: An end-of-turn flag MUST be delivered by forced continuation:
  the turn does not end until the model has addressed the flag. At most one
  forced continuation MAY occur per turn; a flag arising from the continuation
  itself falls under the FR-010 cooldown (no continuation loops).
- **FR-011**: The hold verdict MUST only ever escalate to the user (request
  confirmation); the layer MUST NOT autonomously and silently deny actions in
  v1. Autonomous denial is a named deferral pending measured precision.
- **FR-012**: The system MUST evaluate signals against the bare trajectory
  (what was done and said), never against the model's self-reported reasoning,
  and the review pass MUST receive candidates stripped of the model's
  self-justification (blind judging).

### Key Entities

- **Checkpoint event**: one harness boundary crossing — its kind (pre-action /
  post-batch / end-of-turn), the triggering payload (pending action or
  recent activity or final message), and a reference to the session
  trajectory.
- **Signal**: one named detector from the v1 catalog, with whatever it
  detected (the repeated action, the conflicting memory, the contradicting
  statements) and whether it came from screening or review.
- **Verdict**: the checkpoint's outcome — silence, flag (with assembled
  message), or hold (with assembled reason) — plus which signals produced it.
- **Checkpoint record**: the per-evaluation audit row — boundary, signals,
  screening/review outcomes, verdict, latency, cost.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001** (precision — the make-or-break): across a corpus of at least 20
  recorded benign sessions replayed through all three boundaries, at least 95%
  of checkpoint evaluations return silence, and zero holds fire.
- **SC-002** (catch rate): across at least 12 seeded-failure trajectories
  covering all four v1 signals, at least 80% are flagged or held at the first
  checkpoint where the failure is observable, and 100% of seeded
  memory-contradicting actions are held.
- **SC-003** (latency): pre-action checkpoints decide within their hard time
  budget in 100% of evaluations; the 95th-percentile pre-action decision
  completes in under 150 ms.
- **SC-004** (fail-open): with the layer made unavailable or erroring
  mid-session, 100% of exercised sessions complete with no blocked actions and
  no user-visible degradation.
- **SC-005** (auditability): 100% of checkpoint evaluations produce exactly one
  record, and flag rate / hold rate / catch rate are computable from records
  alone.
- **SC-006** (inertness when off): with the sensor plane uninstalled or
  disabled, harness sessions behave identically to the same server before
  installation — no latency is added, nothing is injected, no checkpoint
  evaluations occur, and zero checkpoint records accrue during a benign
  session. (The checkpoint capabilities remain in the catalog like any other
  tool; installing or uninstalling the sensor plane changes no server
  behavior — FR-007.)
- **SC-007** (actionability): every delivered flag names the specific evidence
  (the repeated action, the conflicting statements, or the contradicted
  memory) — no flag is a generic warning.

## Assumptions

- The supported harness is the one the project's users already use (Claude
  Code and its SDK), whose hook events provide per-boundary payloads and a
  path to the session trajectory. Other harnesses are out of scope; the
  server remains fully usable without the sensor plane.
- The sensor plane ships as configuration + thin glue in this repository
  (the project's first deliverable outside the server binary — a named scope
  extension per the 2026-06-12 amendment to `WATCHDOG_LAYER.md`).
- The memory-contradiction signal requires the memory layer to be enabled
  (its existing credential); without it the signal is silently inactive
  rather than an error.
- The end-of-turn review pass uses the existing model credential already
  required by the server; no new credential or capability is introduced.
- "Recent window" bounds (how much trajectory each signal examines), the
  repetition thresholds, and the cooldown duration are tunable defaults
  fixed during planning; the spec fixes only that they exist and are bounded.
- Mid-generation interruption, push-channel delivery, sycophantic-flip, goal
  drift, hallucination/grounding, and injection signals, and autonomous
  denial of actions are named deferrals — out of v1 scope (per the corpus
  amendment and FR-004/FR-011).
- Benign-session and seeded-failure corpora for SC-001/SC-002 are assembled
  from this project's own recorded development sessions plus synthetic
  trajectories; they become part of the repository's test assets.
