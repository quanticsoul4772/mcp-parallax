# Data Model: Push Memory

One additive table; no changes to existing tables or types. All selection
inputs are the existing `Memory` type; no new stored kinds or trust states
(spec Assumptions).

## 1. Selection pipeline (pure, `src/memory/push.rs`)

```text
prompt --(truncate PUSH_PROMPT_CHARS=2000)--> excerpt
excerpt --Embedder::embed_query--> query vector          (the one network call)
Storage::load_memories --ranking::rank(query, now)--> ranked (cosine+recency+trust)
ranked --filter trust.is_trusted()--> trusted            (FR-004)
trusted --filter raw cosine >= PUSH_RELEVANCE_TAU=0.55--> relevant  (FR-003)
relevant --subtract Storage::pushed_memory_ids(session)--> fresh    (FR-005)
fresh --truncate PUSH_CAP=3--> surfaced                  (FR-003)
surfaced --fixed template--> additionalContext block     (FR-002, research D7)
whole pipeline under tokio timeout PUSH_BUDGET_MS=500    (FR-007)
```

Empty at any stage ⇒ silence: no `hookSpecificOutput` at all, audit row with
empty `surfaced_ids`.

## 2. `SurfaceParams` (tool input)

| Field | Type | Notes |
|---|---|---|
| `session_id` | `String` | harness session; suppression scope |
| `prompt` | `String` | the turn-starting user prompt (excerpted server-side) |

## 3. `SurfaceResult` (tool result — typed `Json<T>`)

| Field | Type | Notes |
|---|---|---|
| `surfaced` | `Vec<SurfacedMemory>` | empty ⇒ silence |
| `fail_open` | `bool` | degraded evaluation (timeout/backend failure) |
| `latency_ms` | `u64` | evaluation wall-clock |
| `hookSpecificOutput` | object \| absent | `{hookEventName: "UserPromptSubmit", additionalContext}` — present only when `surfaced` is non-empty; exact field shape confirmed by the S2 spike |

`SurfacedMemory`: `{ id, kind, trust, score, content }` — content verbatim,
score is the raw cosine (auditability of the floor).

## 4. `push_records` (new table; FR-008, SC-005)

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | UUID v4 |
| `session_id` | TEXT | suppression + aggregation key |
| `surfaced_ids` | TEXT | JSON array of memory ids (empty array = silence) |
| `latency_ms` | INTEGER | |
| `fail_open` | INTEGER | 0/1 |
| `input_tokens` | INTEGER | embed usage (cost attribution) |
| `created_at` | TEXT | RFC 3339 via `TimeProvider` |

Additive `CREATE TABLE IF NOT EXISTS`; no migration. **Load-bearing audit**:
`pushed_memory_ids(session_id)` = union of `surfaced_ids` for the session —
suppression reads the audit trail, so FR-005 and FR-008 are the same data
(research D4).

`Storage` seam additions: `record_push(&PushRecord)`,
`pushed_memory_ids(session_id) -> Vec<String>`.

## 5. Observability (007 pattern)

`observability::emit_push(&PushRecord)` at the same exit point as the store
write — span `parallax.push` with session id, surfaced count, latency,
fail_open, and embed tokens; no memory *content* in attributes (the
checkpoint records' no-evidence-in-attributes rule applies).

## 6. Constants (research D9)

| Const | Value | Provenance |
|---|---|---|
| `PUSH_RELEVANCE_TAU` | 0.55 | `GATE_RELEVANCE_TAU`'s measured 0-false-positive record |
| `PUSH_CAP` | 3 | below pull default 5; unrequested context competes for attention |
| `PUSH_BUDGET_MS` | 500 | clarify Q2 (decide, margin 30); gate-symmetric |
| `PUSH_PROMPT_CHARS` | 2000 | bounded embed input (review-excerpt pattern) |
