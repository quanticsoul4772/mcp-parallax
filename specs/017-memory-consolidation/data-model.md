# Data Model: Memory Consolidation and Auto-Capture

## 1. `memories` — three new columns (first ALTER TABLE migration, research D2)

| Column | Type | Default / backfill | Meaning |
|---|---|---|---|
| `status` | TEXT NOT NULL | `'active'` | active \| superseded \| merged |
| `replaced_by` | TEXT NULL | NULL | id of the superseding / canonical record |
| `last_reinforced_at` | TEXT NOT NULL | backfilled = `created_at` | decay clock (research D5) |

Existing columns are never written after admission (FR-010). Migration:
pragma-guarded `ALTER TABLE ... ADD COLUMN`, loud on failure, tested
against a pre-017 fixture database.

### Status lifecycle

```text
                    admission judgment
 active ──updates──────────────► superseded   (replaced_by = newer id)
 active ──same_assertion───────► merged       (replaced_by = canonical id)
 active ──context/distinct/uncertain── stays active (decline bias)
 any    ──user forget──────────► deleted      (the only removal; any status)
```

No transitions out of superseded/merged (ping-pong is directional: each
*admission* is judged once, and the newest admission wins). Retrieval
filter: `status == active` everywhere memories feed reasoning (recall
ranking, push selection, gate constraints, review/elicit recall);
inspection surfaces list all statuses.

## 2. Admission pipeline (research D3/D4)

```text
save / candidate admission
  └─ embed (existing) ─ store record (active)
     └─ screen: cosine vs ACTIVE memories of same kind
          best pair < SUPERSEDE_SCREEN_TAU (0.75)  → done (no judgment)
          best pair ≥ 0.75                          → ONE judgment pass
               judgment: same_assertion | updates | context_specific | distinct + basis
                 same_assertion ∧ cosine ≥ MERGE_SCREEN_TAU (0.90) ∧ trust-guard
                     → older := merged, replaced_by = new id
                 updates → older := superseded, replaced_by = new id
                 else / failure / budget → keep both (decline bias)
     └─ one consolidation_records row per applied action
```

Trust guard (D4): an untrusted admission never merges away a trusted
record. Judgment mode: registered, flat+closed
(`contracts/consolidation.hop.json`), `CONSOLIDATION_BUDGET_MS = 5000`.

## 3. Turn-hop capture extension (research D6)

`ReviewOut` (checkpoint review) gains, keeping flat+closed:

| Field | Type | Meaning |
|---|---|---|
| `capture_worthy` | bool | decline-biased; most turns ⇒ false |
| `capture_kind` | `"skill" \| "lesson" \| "none"` | reusable success vs diagnosed failure |
| `capture_content` | String | the candidate memory, self-contained ("" when none) |
| `capture_basis` | String | one sentence of grounds ("" when none) |

`run_turn` on `capture_worthy` (memory configured, cap not reached):
stores a new memory — kind from judgment, `origin` = `auto-capture:
session (id), end-of-turn`, `external = true` ⇒ trust `Untrusted`
(quarantine falls out of existing derivation; push never surfaces
untrusted, recall labels it) — plus a `capture_proposed` audit row;
cap-exceeded proposals get `capture_dropped` rows and store nothing.
Failures are fail-open; the turn's checkpoint verdict is never affected.

## 4. `consolidation_records` (new additive table, research D8)

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | UUID v4 |
| `session_id` | TEXT NULL | capture rows carry it; admission rows carry the invoking session when known |
| `action` | TEXT | supersede \| merge \| capture_proposed \| capture_dropped |
| `source_id` | TEXT | the acted-on / proposed memory |
| `target_id` | TEXT NULL | superseding / canonical id |
| `basis` | TEXT | the judgment's one-sentence grounds |
| `created_at` | TEXT | RFC 3339 |

Storage seam: `record_consolidation`, `captures_in_session(session_id) ->
u32`; inherent `list_consolidations` for tests/operators. OTLP mirror
`parallax.consolidation` span + counter by action — ids and counts only,
never content.

## 5. Reinforcement (research D5)

Being returned by `recall` or surfaced by `push` updates
`last_reinforced_at` (fire-and-forget after response assembly; failures
warn-logged). Ranking's recency term reads `last_reinforced_at`; weights
unchanged (0.02 / 30-day half-life); raw-cosine floors untouched — decay
reorders, never gates.

## 6. Constants (research D9)

| Const | Value | Provenance |
|---|---|---|
| `SUPERSEDE_SCREEN_TAU` | 0.75 | conservative; far above the T015 related-content datum (0.406) |
| `MERGE_SCREEN_TAU` | 0.90 | near-duplicate territory; byte-different phrasing of one assertion |
| `CAPTURE_SESSION_CAP` | 2 | candidate-flood bound; silence remains the default |
| `CONSOLIDATION_BUDGET_MS` | 5000 | save path already tolerates verify-at-save latency |
