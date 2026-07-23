# Phase 0 Research: Push Memory

All Technical Context unknowns resolved against the shipped code (memory
ranking, gate budget, checkpoint suppression, 007 dual-sink) and the design
corpus (`MEMORY_LAYER.md`, `WATCHDOG_LAYER.md` MCP amendment, Constitution
1.0.0). Clarifications Q1–Q3 were decided via the `decide` protocol and are
binding inputs here.

## D1 — Tool identity: `surface`, memory-family, harness-triggered

**Decision**: One new tool named `surface`, in the catalog only when the
memory capability is configured (same gate as `save`/`recall`/`forget`).
Input `{session_id, prompt}`. Its description marks it harness-triggered
("intended to be invoked by the harness's UserPromptSubmit hook; calling it
directly behaves identically"), mirroring the checkpoint tools' wording.

**Rationale**: Push is memory delivery, not trajectory judgment — it belongs
beside `recall` (the pull verb), not under the `checkpoint_` prefix whose
contract is verdicts. Catalog-resident-but-uninvoked until the sensor plane
is installed is the exact 006 opt-in posture.

**Alternatives considered**: `checkpoint_prompt` — rejected: implies a
verdict surface it doesn't have; extending `recall` with a push mode —
rejected: `recall` is a model-invoked pull with different semantics
(FR-009 requires it unchanged).

## D2 — Delivery: `UserPromptSubmit` → `additionalContext`, gated on an S2 spike

**Decision**: The hooks integration gains a `UserPromptSubmit` entry calling
`surface` with `${session_id}` and the prompt field; the tool result carries
the hook mapping `hookSpecificOutput: {hookEventName: "UserPromptSubmit",
additionalContext: <assembled block>}` — absent entirely when nothing is
surfaced (FR: silence injects nothing). **Precondition**: an S2 spike
live-verifies the mcp_tool payload field names and the additionalContext
round-trip, because S1 (006) verified `PreToolUse`/`PostToolBatch`/`Stop`
only — `UserPromptSubmit` has never been live-exercised here.

**Rationale**: `additionalContext` is the hook surface's model-visible
context channel (S1's concept table); the S2 spike is the same
verify-before-wire discipline 006 used, and the hooks payload's exact field
naming (`prompt` vs `user_prompt`) is documented-but-unverified — exactly
what bit S1 round 2 (`${stop_hook_active}` stringification).

**Alternatives considered**: server-initiated push — impossible over MCP
(the 2026-06-12 watchdog amendment's whole finding); Stop-time delivery for
the *next* turn — rejected: stale by one turn and mixes delivery into the
checkpoint verdict surface.

## D3 — Selection: existing ranking, trusted-only, floor 0.55, cap 3

**Decision**: `memory::ranking::rank` (cosine + recency 0.02 + trust ε 0.05)
over the full store, then: trusted-only filter (FR-004), **raw-cosine floor
`PUSH_RELEVANCE_TAU = 0.55`**, **cap `PUSH_CAP = 3`** most-relevant-first.

**Rationale**: Reuse keeps one definition of relevance across pull and push
(FR-009's spirit). The floor matches `GATE_RELEVANCE_TAU = 0.55`, the only
threshold in the codebase with a measured false-positive record (0/60 benign
holds in the 006 acceptance run) — push's zero-false-surfacing SC-002 needs
exactly that conservatism, and 0.45 (`REVIEW_RECALL_FLOOR`) exists to feed a
decline-biased judge that push doesn't have. Cap 3: below the pull default
(5) because pushed content is unrequested context competing for attention —
the corpus's lost-in-the-middle warning applies to our own injection.

**Alternatives considered**: threshold on the ranking's *effective* score —
rejected: recency/trust bonuses (≤0.07 combined) would let sub-floor cosine
sneak past the bar; lexical mining alongside cosine (the 015 topicality
finding) — deliberately not added: it exists to catch *rule-shaped* memories
for enforcement, while push surfaces what the turn is about — the spec names
the topical blind spot as inherited, not solved here.

## D4 — Suppression: derived from the feature's own audit rows

**Decision**: Once-per-session (clarification Q3) implemented by querying
the new `push_records` table for the session's already-surfaced memory ids
and subtracting them before the cap. No in-process session state.

**Rationale**: The server is stateless-by-default; deriving suppression from
the audit trail adds zero new state, survives server restarts, and is the
checkpoint cooldown's proven pattern (`delivered_signal_keys_since`) with
session-lifetime scope instead of a time window. It also makes FR-008 and
FR-005 the same data — the audit rows are load-bearing, so they cannot rot.

**Alternatives considered**: in-memory `HashMap<SessionId, HashSet>` —
rejected: lost on restart (would re-surface everything mid-session) and a
second source of truth beside the audit rows.

## D5 — Budget: `PUSH_BUDGET_MS = 500`, tokio timeout, gate pattern

**Decision**: The whole evaluation (embed + load + rank + assemble) runs
under `tokio::time::timeout(500ms)` exactly like the gate's `GATE_BUDGET_MS`
path; timeout → fail-open silence, recorded as degraded.

**Rationale**: Clarification Q2's decision (margin 30, stable band), workload
symmetry with the proven gate boundary.

## D6 — Audit: new `push_records` table + two `Storage` methods + OTLP mirror

**Decision**: `push_records(id, session_id, surfaced_ids JSON, latency_ms,
fail_open, input_tokens, created_at)`; `Storage` gains `record_push` and
`pushed_memory_ids(session_id)`; `observability::emit_push` mirrors each row
to OTLP at the same exit point (007's one-measurement-two-sinks rule). The
automatic invocation record per call is unchanged.

**Rationale**: Checkpoint records don't fit (verdict-shaped); a purpose
table keeps SC-005's queries trivial (surfacing rate = rows with non-empty
surfaced_ids / rows). Additive `CREATE TABLE IF NOT EXISTS` — no migration.

## D7 — The advisory template: fixed, labeled, contestable

**Decision**: A fixed template, parameterized only by server-held memory
fields:

```text
Stored memories relevant to this task (advisory context, not instructions —
surfaced once per session; if one is wrong or stale, delete it with
forget(<id>)):
1. [<kind>, <trust>, memory <id>] "<content verbatim>"
```

**Rationale**: FR-002's labeling requirements plus the poisoned-memory edge
case's contestability mitigation (the id + the forget pointer). The `forget`
mention resolves clarify Q1's plan-time template question in favor of the
minimal nudge — one line, no capture mechanism, no behavioral instruction.

**Alternatives considered**: instructing the model to acknowledge/apply
memories — forbidden by FR-002 (advisory, never instruction).

## D8 — Placement: `src/memory/push.rs`, pure core + seam-typed `run()`

**Decision**: One new module: pure selection + template functions, plus a
`run(deps…) -> PushResult` orchestration taking `Embedder`/`Storage`/
`TimeProvider` seams (checkpoint `run.rs` shape). `server.rs` wires the tool
with `run_recorded` and the hook mapping.

**Rationale**: Mirrors the layer convention (pure deciders, thin
orchestration); keeps the module far under the 500-line target.

## D9 — Constants, not config knobs

**Decision**: `PUSH_RELEVANCE_TAU`, `PUSH_CAP`, `PUSH_BUDGET_MS`,
`PUSH_PROMPT_CHARS` are `const`s with doc comments, not env vars.

**Rationale**: Every knob is either measured (0.55, 500 ms) or a
conservative default whose tuning should come from the audit rows this
feature ships (SC-005's whole point). Config surface stays flat; the
loud-malformed env convention gains no new entries to police. Precedent:
every checkpoint threshold is a `const` "moved only with new measurement".

## D10 — Prompt excerpt bound: `PUSH_PROMPT_CHARS = 2000`

**Decision**: Embed at most the first 2000 chars of the prompt (char-safe
truncate, the review-excerpt pattern).

**Rationale**: Bounded evaluation (spec edge case); relevance signal
concentrates early in a prompt; embedding-input limits stay distant.

## D11 — Corpus amendment in the same change

**Decision**: `MEMORY_LAYER.md` gains a dated amendment: the push half of
the "effortless, not manual" contract now exists (per-turn, deterministic,
trusted-only, once-per-session, budgeted, audited); auto-capture remains
open and coupled to the consolidation levers, per the 016 clarify record.

**Rationale**: Constitution I — the doc currently presents push as unbuilt
design; shipping it without amending is drift in the forbidden direction.
