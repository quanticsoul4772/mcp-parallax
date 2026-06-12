# Data Model: Checkpoint Layer

## 1. Core enums and types (`src/checkpoint/mod.rs`)

```text
Boundary      = Action | Batch | Turn          (snake_case on the wire)
Verdict       = Silence | Flag | Hold          (snake_case on the wire)
SignalKind    = Repetition | RepeatedFailure | MemoryConflict | SelfContradiction
Signal        = { kind: SignalKind, evidence: String, signal_key: String }
                 // signal_key = kind + stable hash of normalized evidence (cooldown identity)
```

Constants (tunable defaults fixed at planning; move only with acceptance
evidence):

| Constant | Default | Bound/Use |
|---|---|---|
| `WINDOW_ENTRIES` | 200 | max transcript entries read per evaluation |
| `WINDOW_BYTES` | 2 MB | max transcript bytes read |
| `WINDOW_BATCHES` | 10 | repetition lookback |
| `REPEAT_THRESHOLD` | 4 | identical normalized actions → flag (US1-AS1) |
| `FAILURE_THRESHOLD` | 3 | consecutive failures of same action → flag (US1-AS2) |
| `GATE_BUDGET_MS` | 500 | hard pre-action budget; timeout → fail-open (FR-009) |
| `GATE_RELEVANCE_TAU` | τ (set by S2/acceptance) | min cosine for a hold (D4) |
| `REVIEW_CANDIDATES_MAX` | 4 | cap on candidate pairs sent to the review hop |
| `COOLDOWN_WINDOW_MS` | 1_800_000 | flag suppression window (FR-010) |
| `GATE_RISK_PATTERNS` | built-in set | FR-013 default: consequential shell commands (deploy/push/delete/migrate/...) and config-file writes; overridable via `CHECKPOINT_GATE_PATTERNS` env (comma-separated substrings; present-but-unparseable = error, per config convention) |

## 2. Trajectory window (`src/checkpoint/trajectory.rs`)

What `TrajectoryReader` yields — detectors never see raw JSONL:

```text
TrajectoryWindow {
  session_id: String,
  entries: Vec<TrajectoryEntry>,        // oldest → newest, bounded
}
TrajectoryEntry =
  | ToolCall   { batch_index: u32, tool_name: String, normalized_input: String, failed: bool }
  | Assistant  { text: String }          // assistant message content (final + earlier)
```

Normalization (the precision lever, D5): whitespace-collapsed, volatile
fields (ids, timestamps, absolute temp paths) dropped, then exact-match
comparison. No fuzzy similarity anywhere in screening.

**`TrajectoryReader` seam (`src/traits/trajectory.rs`)**:
`read(path, session_id, bounds) -> Result<TrajectoryWindow, AppError>`.
`FsTrajectoryReader` validates before reading (§5); mock implementations feed
ground-truth tables in tests.

## 3. Wire contracts (MCP-side, `src/checkpoint/contract.rs`)

Params (one struct per boundary — see `contracts/*.tool.json`):

```text
CheckpointActionParams { session_id, transcript_path, tool_name, tool_input }
CheckpointBatchParams  { session_id, transcript_path }
CheckpointTurnParams   { session_id, transcript_path, final_message, continuation: bool }
```

Result (shared):

```text
CheckpointResult {
  verdict: "silence" | "flag" | "hold",
  message: string | null,        // assembled, model/user-facing; null iff silence
  signals: [ { kind, evidence } ],   // empty iff silence with nothing fired
  suppressed: bool,              // a flag was due but cooldown-suppressed (FR-010)
  fail_open: bool,               // evaluation degraded (FR-008) — verdict is silence
  latency_ms: integer
}
```

The integration layer (hooks.json) maps `hold` → `permissionDecision:"ask"`,
`flag` → `decision:"block"` + message, `silence` → no-op. The server never
emits hook rewrite fields (FR-002).

## 4. Model-hop schema (flat + closed — Principle II; `src/checkpoint/review.rs`)

`checkpoint_review` (the only model pass in the layer; end-of-turn only):

```text
{ contradicts: boolean,
  statement_a: string,           // verbatim earlier statement / memory text
  statement_b: string,           // verbatim final-message excerpt
  basis: string }                // one sentence: why these conflict (or why not)
additionalProperties: false; all strings; no nesting, no enums-in-options
```

Decline-biased prompt: a contradiction must be explicit and material — tone
shifts, refinements, and additions are NOT contradictions, and a reversal
justified by evidence that appeared between the two statements is NOT a
contradiction (FR-004(d)). Each candidate pair carries a compact summary of
the tool outcomes observed between the statements so the rule is applicable.
Candidates are presented stripped of surrounding self-justification (FR-012).

## 5. New capability bounds: transcript read (Principle VI)

`FsTrajectoryReader` accepts a path only if ALL hold:

- canonicalizes successfully and has the `.jsonl` extension;
- the file exists and is a regular file;
- the JSONL's session id matches the params' `session_id`;
- read is capped at `WINDOW_BYTES`/`WINDOW_ENTRIES` (tail window).

Violations are `ValidationFailure` (recorded), never partial reads. Nothing
from the transcript flows back to the caller except assembled evidence
strings inside `message`/`signals`.

## 6. Storage: `checkpoint_records` (FR-006, SC-005)

```sql
CREATE TABLE checkpoint_records (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  boundary TEXT NOT NULL,              -- action | batch | turn
  signals_evaluated TEXT NOT NULL,     -- JSON array of SignalKind
  signals_fired TEXT NOT NULL,         -- JSON array of {kind, signal_key}
  review_ran INTEGER NOT NULL,         -- 0/1 (turn boundary only)
  verdict TEXT NOT NULL,               -- silence | flag | hold
  suppressed INTEGER NOT NULL,         -- cooldown downgrade happened
  fail_open INTEGER NOT NULL,
  latency_ms INTEGER NOT NULL,
  cost_usd REAL NOT NULL,
  created_at TEXT NOT NULL
);
```

Queries: cooldown lookup (`signal_key` ∈ `signals_fired`, same session,
within window — D7); rate metrics (flag rate, hold rate, suppression rate,
fail-open rate) as plain SQL (SC-005). One row per evaluation, plus the
standard invocation record via `run_recorded` (tool ids
`checkpoint_action`/`checkpoint_batch`/`checkpoint_turn`).

## 7. Verdict assembly (pure functions — FR-005)

```text
gate:   risk-matched ∧ top constraint-memory ≥ τ        → Hold(message quotes memory)
        else                                            → Silence
batch:  any screening signal (post-cooldown)            → Flag(message names action+count)
        else                                            → Silence
turn:   candidates = mine(window, recall)
        candidates = ∅                                  → Silence (no hop)
        hop says contradicts                            → Flag(message cites a & b) [forced continuation]
        hop says no                                     → Silence (record: review cleared)
        continuation == true                            → screening only; flags cooldown-checked (FR-014)
any:    error / timeout anywhere                        → Silence + fail_open (FR-008)
```

Message templates are fixed strings parameterized only by evidence (SC-007:
every flag names its specific evidence; no generic warnings).
