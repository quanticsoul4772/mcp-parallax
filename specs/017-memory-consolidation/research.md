# Phase 0 Research: Memory Consolidation and Auto-Capture

All unknowns resolved against the shipped code and the three clarify
decisions (capture channel, trigger timing, decay endpoint — each decided
via the `decide` protocol; see spec Clarifications).

## D1 — Status as a dimension, not a deletion

**Decision**: `Memory` gains `status: active | superseded | merged` plus
`replaced_by: Option<id>` and `last_reinforced_at`. Every retrieval path
(recall ranking, push selection, checkpoint gate constraints, review/elicit
recall) filters to `active`; inspection (`list`-side surfaces, recall's raw
store view) sees everything with status. Content columns are never written
after admission (FR-010).

**Rationale**: One dimension gives supersession and merge the same
lifecycle mechanics and makes FR-011's consistency a single filter
predicate. Excluding non-active memories from the checkpoint gate matters:
a superseded constraint must stop producing holds, exactly as it stops
being enforced at turn end (the 015 interplay named in the spec).

**Alternatives considered**: separate tombstone tables — rejected: two
sources of truth for "is this active"; deletion — forbidden by FR-010.

## D2 — First column migration: pragma-guarded ALTER TABLE

**Decision**: `connect` runs the existing `CREATE TABLE IF NOT EXISTS`
block, then a column-migration step: inspect `PRAGMA table_info(memories)`
and `ALTER TABLE memories ADD COLUMN ...` for each missing 017 column, with
defaults (`status='active'`, `replaced_by=NULL`,
`last_reinforced_at=created_at` backfilled in one UPDATE). Loud on any
failure. A dedicated test opens a pre-017 database fixture and asserts the
migrated shape and untouched contents.

**Rationale**: The migration convention to date was purely additive tables;
017 is the first schema change to an existing table, so the pattern gets
established here deliberately, tested, and documented — not improvised.

## D3 — Admission-time evaluation: deterministic screen gates one judgment

**Decision**: On every admission (manual `save`, admitted candidate), after
embedding: compare the new memory against **active** memories of the same
kind by raw cosine. Screen bands (consts, D9 discipline):
`MERGE_SCREEN_TAU = 0.90` and `SUPERSEDE_SCREEN_TAU = 0.75`. Pairs at or
above the supersede screen go to **one** budgeted model judgment (new
registered mode, flat+closed): it classifies the pair as
`same_assertion` / `updates` / `context_specific` / `distinct` with a
one-sentence basis. Apply rules are pure: `same_assertion` (and screen ≥
merge tau) ⇒ merge, older record becomes `merged`, survivor is the
**newer** admission verbatim; `updates` ⇒ older becomes `superseded` with
`replaced_by`; anything else or any judgment failure ⇒ both stay active
(decline bias, FR-002). At most the single best-scoring pair is judged per
admission (bounded cost; remaining duplicates converge over subsequent
admissions).

**Rationale**: The checkpoint layer proved screen-gates-judge; cosine bands
are deterministic and cheap; update-vs-context is not mechanically
checkable (Constitution V names the judgment). Judging only the top pair
bounds admission latency and still converges because admissions recur.
Thresholds start conservative and move only with audit-row measurement —
the 016 T015 datum (related content at 0.406) says these bands are far
above noise.

**Alternatives considered**: judging all screened pairs — unbounded save
latency; pure-cosine merge without judgment — collides with the
Berlin/Lisbon requirement (high similarity ≠ same assertion).

## D4 — Merge survivor: the newer admission, byte-identical

**Decision**: On merge the newly admitted record survives as canonical
(its content verbatim); the older becomes `merged` with `replaced_by`
pointing at the survivor. Trust of the survivor is its own derived trust,
with one guard: an untrusted admission never merges away a trusted record
(the pair falls through to keep-both).

**Rationale**: Newer wording tends to reflect current phrasing/context, and
survivor-is-the-admission makes the apply rule uniform with supersession
(newest wins). The trust guard keeps quarantine meaningful and enables
promotion-by-re-admission (D7) without letting the reverse path launder
untrusted content into a canonical slot.

## D5 — Decay: reinforcement-refreshed recency, ranking-only

**Decision**: The ranking recency term reads `last_reinforced_at` instead
of `created_at`; being returned by `recall` or surfaced by `push` refreshes
it (fire-and-forget write after the response is assembled). Weight and
half-life keep their current values; the raw-cosine floors are untouched,
so decay reorders within qualifiers and can never hide an above-floor
memory from push, nor remove anything (clarify Q3: ranking-only).

**Rationale**: The existing recency term already has the right shape; the
only missing piece was reinforcement. Keeping the weight at 0.02 preserves
the measured ranking behavior (band-edge tests) and honors
memory-blindness: decay is a tiebreaker, not a gate.

## D6 — Capture: the turn hop's third judgment

**Decision**: `ReviewOut` gains a capture judgment (fields:
`capture_worthy: bool`, `capture_kind: skill|lesson|none`,
`capture_content`, `capture_basis`), decline-biased ("most turns capture
nothing"), evaluated in the same single end-of-turn pass as contradiction
(006) and preference violation (015). `run_turn` stores an accepted
proposal as a new memory: kind from the judgment, `origin` naming the
session and boundary, `external = true` ⇒ **`Untrusted`** (quarantine via
the existing derived-trust rule — no new trust states), embedded via the
existing embedder, capped by `CAPTURE_SESSION_CAP = 2` counted from the
session's consolidation audit rows. Memory off ⇒ the judgment is not
requested (evaluated-kinds conditional, the 016 D7 pattern). Capture
storage failures are fail-open (FR-008) — the turn's verdict is unaffected.

**Rationale**: Clarify Q1 chose the harness channel; the boundary's
one-model-pass convention (held through 006 → 015) makes extending the hop
the only conforming delivery. Marking candidates `external=true` is
honest — model-authored content the user did not write is not first-hand
experience of the *user's* choosing — and it makes quarantine fall out of
the existing trust derivation with zero new mechanics: push already never
surfaces untrusted (016 FR-004), recall already labels trust.

**Alternatives considered**: a first-hand-but-candidate trust state —
rejected: new trust state, new push/recall rules; a separate capture hop —
rejected: second model pass at the boundary.

## D7 — Promotion: re-admission, no new tool

**Decision**: No `promote` surface. A quarantined candidate becomes trusted
knowledge when its content is explicitly re-admitted: the user (or the
model on the user's behalf) calls `save` first-hand — merge unifies the
pair and the trusted admission survives as canonical (D4's guard makes the
direction safe) — or `save` with `verify` for external claims, using the
verification pass that already exists.

**Rationale**: FR-007 requires existing paths only; the levers themselves
implement promotion, keeping the tool surface at zero growth. The `recall`
label ("untrusted") plus the candidate's origin string give the user what
they need to decide.

## D8 — Audit: `consolidation_records`

**Decision**: New additive table: `id, session_id (nullable), action
(supersede | merge | capture_proposed | capture_dropped), source_id,
target_id (nullable), basis, created_at`. One row per applied action, per
capture proposal, and per cap-dropped proposal (the spec's candidate-flood
edge case). `captures_in_session(session_id)` counts proposals for the cap.
Mirrored to OTLP as `parallax.consolidation` spans (007 dual-sink; counts
and ids only — content never enters attributes).

## D9 — Constants, not config

**Decision**: `MERGE_SCREEN_TAU = 0.90`, `SUPERSEDE_SCREEN_TAU = 0.75`,
`CAPTURE_SESSION_CAP = 2`, `CONSOLIDATION_BUDGET_MS = 5_000` (the judgment
rides the save path, which already tolerates verify-at-save latency) — all
`const` with provenance doc comments, movable only with audit-row
measurement (the established D9 discipline from 016, now with the T015
datum as precedent).

## D10 — Corpus amendment in the same change

**Decision**: `MEMORY_LAYER.md`'s write-path section gains the dated
amendment: capture → consolidate shipped (admission-time levers,
ranking-only decay, quarantined harness capture), with the summarization-
drift and memory-blindness traps encoded as hard rules, and the three
clarify decisions recorded. Constitution I.
