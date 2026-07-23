# Feature Specification: Memory Consolidation and Auto-Capture

**Feature Branch**: `017-memory-consolidation`

**Created**: 2026-07-23

**Status**: Draft

**Input**: User description: "Memory consolidation plus auto-capture — the write-path half of the memory layer, completing the loop 016 deliberately deferred (its clarify record coupled capture to the consolidation levers). Today the store only grows, and only by manual save: no merge of near-duplicates, no decay, no supersession handling, no eviction, and no capture of session outcomes. `MEMORY_LAYER.md` names the write path capture → reflect → consolidate with four levers — importance, merge, decay, eviction — and two traps: summarization drift and memory blindness. Constraints: retrieval precision is the product; the poisoning surface must not widen (candidates never auto-trusted); consolidation must not silently rewrite what the user stored; deterministic-over-probabilistic wherever checkable."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Stale information stops competing (Priority: P1)

A user stored a fact months ago ("the deploy pipeline runs on the old CI
provider"). The world changed, and a newer memory records the update ("the
deploy pipeline moved to the new CI provider"). Today both coexist and
compete: recall and push rank them independently, and the stale one can win.
With supersession, when a newly admitted memory *updates* an existing one,
the older memory is marked superseded — excluded from recall and push results
— while remaining stored, inspectable, and attributed. A statement that is
merely *context-specific* ("I'm working from the Berlin office this week")
never supersedes a standing fact ("the team is based in Lisbon"): an update
replaces, a context coexists, and when the difference is uncertain, both are
kept.

**Why this priority**: Stale memories are the write-path failure that
directly damages the just-shipped read path — a superseded fact surfacing
through push is worse than no memory at all, because it arrives with a trust
label. This is the sharpest live pain and the MVP.

**Independent Test**: Store a fact, store its update, and confirm recall and
push return only the update; confirm the superseded original remains
inspectable with the supersession attributed; confirm a context-specific
statement leaves the standing fact active.

**Acceptance Scenarios**:

1. **Given** a stored fact and a newly admitted memory that updates it,
   **When** consolidation evaluates the pair, **Then** the older memory is
   marked superseded, excluded from future recall and push results, and the
   newer memory carries the supersession attribution (which memory it
   replaced and on what basis).
2. **Given** a stored standing fact and a newly admitted context-specific
   statement about the same subject, **When** consolidation evaluates the
   pair, **Then** both remain active — no supersession.
3. **Given** an uncertain case — the system cannot tell update from context,
   **When** consolidation evaluates the pair, **Then** both are kept active
   (decline bias: a wrong supersession destroys good knowledge; a missed one
   costs a duplicate).
4. **Given** a superseded memory, **When** the user inspects the store,
   **Then** the superseded memory is still present and viewable with its
   status, and deleting either memory remains the user's call via the
   existing deletion path.

---

### User Story 2 - The store stays canonical (Priority: P2)

Over months of use the same knowledge gets saved more than once in slightly
different words. Near-duplicates crowd the ranked results: they occupy
recall slots and push's small cap with redundant content. With merge, when a
newly admitted memory asserts the same thing as an existing one, the two
unify into one canonical record — the surviving content stays **verbatim**
(no generated paraphrase ever replaces stored words — the summarization-drift
trap), the merged record's identity and provenance are preserved in the
audit trail, and future retrieval sees one memory, not three.

**Why this priority**: Redundancy directly wastes the retrieval surface —
with a push cap of a few items, two copies of one fact can crowd out a
second, different fact. It is P2 because duplication degrades quality while
staleness (US1) delivers wrong answers.

**Independent Test**: Save the same fact twice in different words; confirm
retrieval afterward returns one canonical record; confirm the merge is
audited with both identities; confirm dissimilar memories never merge.

**Acceptance Scenarios**:

1. **Given** an existing memory and a newly admitted near-duplicate,
   **When** consolidation evaluates the pair, **Then** one canonical record
   remains active, its content byte-identical to one of the originals —
   never a generated summary — and the merge is recorded with both
   identities.
2. **Given** two memories about the same topic that assert different things,
   **When** consolidation evaluates them, **Then** no merge occurs.
3. **Given** an uncertain case, **When** consolidation evaluates the pair,
   **Then** no merge occurs (decline bias — a wrong merge loses a
   distinction; a missed merge costs a duplicate).

---

### User Story 3 - The store grows from ordinary use (Priority: P3)

Today the store grows only when someone remembers to call `save` — the same
manual-dependence failure the corpus documented for recall. With
auto-capture, when a session produces a capture-worthy outcome — an approach
that demonstrably worked (candidate skill) or a failure with a diagnosed
cause (candidate lesson) — a **candidate memory** is proposed automatically.
Candidates pass an importance gate (most outcomes are not capture-worthy;
silence is the default), and every candidate enters **quarantined — never
trusted**: it cannot surface through push, and it becomes trusted only
through the existing verification pass or explicit user confirmation. The
poisoning surface does not widen: automatic writes can propose, never
promote.

**Why this priority**: This is the growth half — valuable, but only safe
*because* US1/US2 exist to keep the growing store precise, which is exactly
why 016 deferred it. It must never outrank the levers that keep the store
clean.

**Independent Test**: Drive a session to a clear success and a clear
failure; confirm at most a bounded number of candidates appear, all
quarantined; confirm an ordinary uneventful session produces zero
candidates; confirm a quarantined candidate never surfaces through push and
appears in recall only marked untrusted.

**Acceptance Scenarios**:

1. **Given** a session whose outcome demonstrates a reusable approach or a
   diagnosed failure, **When** the capture evaluation runs, **Then** at most
   a small bounded number of candidate memories are proposed, each labeled
   as a candidate with its origin, and each stored **untrusted**.
2. **Given** an ordinary session with no capture-worthy outcome, **When**
   the capture evaluation runs, **Then** nothing is captured — no
   placeholder, no low-value memory.
3. **Given** a quarantined candidate, **When** push evaluates any turn,
   **Then** the candidate is never surfaced; **When** the user or the
   verification pass confirms it, **Then** it gains trusted standing through
   the existing admission paths only.
4. **Given** capture fails or exceeds its budget, **When** the session
   continues, **Then** nothing is captured and nothing blocks or delays the
   session (fail-open).

---

### User Story 4 - Nothing is ever silently lost or rewritten (Priority: P4)

An operator reviewing months of consolidation wants certainty that the
levers never destroyed knowledge: every supersession, merge, and capture
decision left exactly one audit record (what acted on what, and the basis);
superseded and merged records remain present and inspectable, excluded from
retrieval but never erased; stored content was never edited, compressed, or
paraphrased in place; and deletion remained exclusively the user's explicit
action. Decay — the aging lever — only ever down-ranks: an old memory
becomes less prominent, never unreachable (the memory-blindness trap), and
its decayed standing is visible when inspected.

**Why this priority**: This story is the license for the other three. A
consolidation layer that cannot prove it lost nothing would be worse than
the append-only store it replaces. It observes and constrains rather than
delivering new capability.

**Independent Test**: Run supersession, merge, capture, and decay scenarios;
confirm one audit record each; confirm every original remains retrievable by
inspection; confirm no stored content changed; confirm deletion only ever
happened via the explicit deletion path.

**Acceptance Scenarios**:

1. **Given** any consolidation action, **When** the operator inspects the
   audit trail, **Then** exactly one record exists for it, naming the
   memories involved and the basis.
2. **Given** any superseded or merged memory, **When** the store is
   inspected, **Then** the record is present with its status and original
   content byte-identical to what was stored.
3. **Given** an aged memory subject to decay, **When** retrieval ranks it,
   **Then** it ranks lower than an otherwise-equal fresh memory but remains
   retrievable and inspectable — decay never removes.

---

### Edge Cases

- **Update ping-pong.** Two memories that keep superseding each other as the
  user's situation alternates. Supersession is directional and dated; the
  newest admitted update wins, and the audit trail shows the sequence. No
  oscillation logic — each admission is judged once.
- **Chains.** A merged-away record's identity later referenced (e.g. by an
  old push audit row): identities are permanent; audit rows referencing a
  merged or superseded id resolve to the record and its status.
- **Deleting a canonical.** The user deletes a memory that others were
  merged into: deletion removes that record (the user's explicit right); the
  audit trail still records the historic merge; merged-away originals remain
  in their stored state.
- **Candidate flood.** A long, eventful session could propose many
  candidates: the per-session bound caps proposals, most-important first;
  the audit row records what was considered and dropped.
- **Sensitive content in candidates.** Auto-capture may propose something
  the user would not have chosen to store. Quarantine is the mitigation:
  candidates never surface through push, are visibly marked in recall, and
  are deletable like any memory; nothing becomes durable-trusted without an
  explicit admission step.
- **Preference memories and 015 enforcement.** The end-of-turn checkpoint
  enforces trusted stored preferences; supersession changes what is
  enforceable (a superseded preference stops being enforced — correct, it
  was updated) and decay does not (down-ranking must not silently weaken
  enforcement of a still-active preference). The interplay is named here so
  test design covers it.
- **Cross-kind conflicts.** A lesson and a fact about the same subject never
  supersede each other across kinds; supersession operates within a kind.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: When a newly admitted memory **updates** an existing active
  memory of the same kind, the system MUST mark the older memory superseded:
  excluded from all future recall and push results, retained in the store
  with its status and original content unchanged.
- **FR-002**: A **context-specific** statement MUST NOT supersede a standing
  memory; when the update-vs-context distinction is uncertain, the system
  MUST keep both active (decline bias).
- **FR-003**: Every supersession MUST be attributed: which memory replaced
  which, when, and on what basis — inspectable alongside the memories.
- **FR-004**: When a newly admitted memory asserts the same thing as an
  existing active memory of the same kind, the system MUST merge them into
  one canonical active record whose content is byte-identical to one of the
  originals; generated paraphrases or summaries MUST NOT replace stored
  content. Uncertain pairs MUST NOT merge.
- **FR-005**: Ranking MUST down-weight memories that age without
  reinforcement (retrieval or re-admission refreshes standing); decay MUST
  only affect ranking prominence — it MUST NOT remove, hide from inspection,
  or auto-evict a memory.
- **FR-006**: The system MAY propose candidate memories from session
  outcomes (reusable success → candidate skill; diagnosed failure →
  candidate lesson), bounded per session, through an importance gate whose
  default is silence.
- **FR-007**: Every automatically captured candidate MUST enter untrusted
  (quarantined): never surfaced by push, visibly marked at recall, and
  promotable to trusted standing ONLY through the existing verification pass
  or an explicit user admission — never automatically.
- **FR-008**: Capture MUST fail open within a hard time budget: a capture
  failure or overrun changes nothing about the session and stores nothing.
- **FR-009**: Every consolidation action (supersession, merge) and every
  capture evaluation MUST write exactly one audit record from which action
  rates, per-memory histories, and drop counts are computable.
- **FR-010**: Stored memory content MUST never be modified in place by any
  lever; the only content-removing operation remains the user's explicit
  deletion, which continues to work on every record regardless of status.
- **FR-011**: Recall and push MUST honor consolidation consistently: both
  exclude superseded and merged-away records and rank canonical records with
  decay applied; inspection surfaces MUST still list every record with its
  status.
- **FR-012**: With the memory capability unconfigured, all behavior MUST be
  unchanged; consolidation and capture add no behavior to sessions without
  memory.

### Key Entities

- **Memory status**: active | superseded | merged-away — a new dimension on
  stored memories; only active records participate in retrieval; all
  records remain inspectable.
- **Supersession link**: replaced-by relationship with date and basis.
- **Merge link**: canonical-record relationship preserving the merged-away
  identity and content.
- **Candidate memory**: an automatically proposed, quarantined (untrusted)
  memory with its origin (which session, which outcome) attached.
- **Consolidation audit record**: one per action/evaluation — actor lever,
  memories involved, basis, outcome.
- **Reinforcement**: a retrieval or re-admission event that refreshes a
  memory's decay standing.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: After storing a fact and its update, 100% of subsequent recall
  and push evaluations return only the update; the superseded original is
  inspectable with its attribution — demonstrable live in a single session.
- **SC-002**: The Berlin/Lisbon scenario holds: a context-specific statement
  leaves the standing fact active in 100% of the feature's
  update-vs-context test scenarios; uncertain pairs never supersede.
- **SC-003**: After a duplicate save, retrieval returns one canonical record
  whose content is byte-identical to an original; dissimilar pairs never
  merge across the feature's test scenarios.
- **SC-004**: Zero content mutations and zero non-user deletions across all
  consolidation scenarios: every original remains byte-identical and
  inspectable; 100% of actions have exactly one audit record.
- **SC-005**: Auto-capture proposes at most the per-session bound, all
  candidates untrusted; zero candidates ever surface through push while
  quarantined; an uneventful session captures nothing.
- **SC-006**: Consolidation and capture add no perceptible delay to the
  operations they ride on, and a capture failure never disturbs a session
  (fail-open, within budget).

## Assumptions

Defaults chosen for planning; the three flagged questions below are
deliberately left for the clarify phase, per the feature request.

- **Capture channel (to be confirmed in clarify)**: default is
  harness-triggered at end of turn via the existing installed sensor plane —
  the same opt-in integration the checkpoint and push layers use; no new
  consent surface.
- **Consolidation trigger timing (to be confirmed in clarify)**: default is
  on-admission — supersession and merge are evaluated when a new memory
  enters the store (manual save or admitted candidate); no background or
  periodic sweeps.
- **Decay endpoint (to be confirmed in clarify)**: default is ranking-only —
  decay never leads to automatic eviction; explicit user deletion remains
  the only removal.
- **Judgment placement**: update-vs-context and same-assertion decisions are
  semantic judgments; where they cannot be settled mechanically, any model
  involvement is a named, budgeted, decline-biased pass — never a silent
  rewrite. Which parts are decidable deterministically is a plan-phase
  question.
- **Trust model unchanged**: no new trust states; "candidate" is a status +
  the existing untrusted standing, and promotion uses the existing
  verification/admission paths.
- **Scope exclusions**: no automatic eviction, no compression or
  summarization of stored content, no shared/multi-user store semantics, no
  retroactive re-consolidation sweep of the existing store at upgrade time
  (existing memories consolidate as they next participate in an admission).
