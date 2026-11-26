# Feature Specification: Jev at turn end

**Feature Branch**: `066-jev-turn-end`

**Created**: 2026-11-26

**Status**: Draft

**Input**: the Jev roadmap §6.2: the U2 end-of-turn review skip, `on` only once the promotion rule passes; U1 turn signals `pushback_reversal` and `unverified_fact`, shadow only. The promotion rule is quoted under Promotion rule below.

## Clarifications

### Session 2026-11-26

- Q: How long is the turn-signal cooldown? → A: None while the signals are shadow only; every occurrence is recorded. The cooldown length is set when the signals are promoted to `on`.
- Q: What share of turns always gets the review when the skip would skip it? → A: 10%, the top of the promotion rule's 5-10% range.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Turn signals in shadow (Priority: P1)

At each turn end, Jev is asked about two turn-level failures the calling model cannot see in itself (`WATCHDOG_LAYER.md:151,153`). Code settles what it can first, and Jev is asked only when code finds the precondition. The answers are recorded; nothing is flagged and the turn's result does not change.

- `pushback_reversal`: the final message reverses a position the previous final message stated, after a user message that pushed back, with no tool call between the pushback and the reversal. Code checks that a user entry sits between two final messages and that no tool call follows it; Jev is asked whether the later message reverses the earlier one without new evidence.
- `unverified_fact`: the final message states a fact that the turn's tool output does not support. Code checks that the final message has content and collects the turn's tool-output excerpts; Jev is asked whether the message states a fact those excerpts do not support.

**Why this priority**: recording these is the measurement any later promotion needs.

**Independent Test**: with the signals in shadow, a stand-in Jev answering yes to both leaves the checkpoint result and record unchanged (cost set aside), and writes one `turn_signals` row holding both answers.

**Acceptance Scenarios**:

1. **Given** the signals site is `shadow`, **When** a non-continuation turn with a final message ends, **Then** one `turn_signals` row is written listing the questions asked and their answers, whether or not the review ran.
2. **Given** a continuation turn, **When** the turn ends, **Then** no signal question is asked and no row is written.
3. **Given** `PARALLAX_JEV_SEND_USER_TEXT` is unset, **When** the turn ends, **Then** `pushback_reversal` is not asked and no user text is in any request.
4. **Given** no question's precondition holds, **When** the turn ends, **Then** the row is an `excluded` row and no request is sent.

---

### User Story 2 - Review skip, off until measured (Priority: P2)

When the 064 turn-review questions are confidently "no" and no cheap signal fired, the end-of-turn review can be skipped, saving a big-model call. The skip is built and tested now and stays off: `on` is refused at startup until the promotion rule has passed and an option-order check is recorded for the configured model.

**Why this priority**: the skip saves the most of any Jev use at turn end, but the promotion rule needs more data; the turn-review rate is re-measured around 2026-12-08.

**Independent Test**: with the skip enabled in a test, each skip condition made false or missing in turn makes the review run.

**Acceptance Scenarios**:

1. **Given** the skip is enabled, memory is configured, both 064 answers are below their pinned no-band thresholds, no memory, preference or transcript candidate exists, the state is not truncated and the turn is outside the audit share, **When** the turn ends, **Then** the review does not run and the checkpoint record's `review_skipped_by` is `jev`.
2. **Given** any of those conditions fails or an answer is missing, **When** the turn ends, **Then** the review runs as today.
3. **Given** a turn in the audit share that the skip would have skipped, **When** the turn ends, **Then** the review runs and `review_skipped_by` is `audit`.
4. **Given** `PARALLAX_JEV_MODE_TURN_REVIEW=on` without a pinned threshold entry and a recorded order check for the configured model, **When** the server starts, **Then** startup fails naming the variable.

---

### User Story 3 - Configuration and records (Priority: P3)

The new site is off unless set, user text leaves the machine only with its own opt-in, and every turn end the site covers leaves a record.

**Why this priority**: off by default and explicit egress are constitution requirements; records are how the shadow data is read later.

**Independent Test**: configuration error tests, and a records test joining every signal row to its checkpoint record.

**Acceptance Scenarios**:

1. **Given** `PARALLAX_JEV_MODE_TURN_SIGNALS` is unset, **When** the server starts, **Then** the site is off and no signal call is made.
2. **Given** the site is `shadow` without `TYPESAFE_API_KEY`, **When** the server starts, **Then** startup fails naming the variable.
3. **Given** `PARALLAX_JEV_SEND_USER_TEXT=true` with the signals site off, or an unparseable value, or an unknown `PARALLAX_JEV_*` name, **When** the server starts, **Then** startup fails naming the variable.

### Edge Cases

- Continuation turns never ask the signal questions.
- Tool results, meta lines, compaction summaries, hook-injected context (including memory text from `surface`) and Parallax's own flag text never count as user text.
- A signal that fires on several turns of one session is recorded each time (no cooldown in shadow).
- User entries are kept outside the 200-entry trajectory window, so every existing signal, review candidate and 064 replay fingerprint is unchanged.
- A Jev error, timeout or rejected answer records the failure and changes nothing.
- With memory off, the skip never applies; the signals are still asked.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001 Transcript spike**: before parser work, a spike records the shapes in Claude Code transcripts that 066 reads: user string content and text blocks, meta lines, compaction summaries, hook-injected context, tool results, and Stop-hook block reasons, with a fixture for each.
- **FR-002 Parser**: the trajectory parser gains a `User { text }` entry (string content and user text blocks) and keeps a redacted, capped excerpt of each tool result's text. User entries are held outside the 200-entry window. A turn starts at the last user entry. Excluded from user text: tool results, meta lines, compaction summaries, hook-injected context and Parallax's own flag text, matched by fixed prefixes the flag templates export.
- **FR-003 Questions**: `pushback_reversal` and `unverified_fact` are yes/no questions, versioned, each asked only when its code precondition holds (User Story 1), in one request under the turn deadline, at every non-continuation turn end with a final message, independent of whether the review runs and of the turn-review mode. Requests share the existing limit of 4 in flight.
- **FR-004 What is sent**: for `unverified_fact`, the final message and the turn's tool-output excerpts; for `pushback_reversal`, the previous final message, the final message and the pushback user text. Everything is redacted. User text is sent only with `PARALLAX_JEV_SEND_USER_TEXT=true`. Stored memories are never sent.
- **FR-005 Stance exception**: `pushback_reversal` sees the user's pushback. This departs from two design rules: judges stay blind to the user's stance (`NEW_SERVER_DESIGN.md:193`) and the critic stays out of the pressured conversation (`NEW_SERVER_DESIGN.md:198-200`). Both departures are named and justified in a design amendment and limited to user text from the same turn. User text is a new data class sent to TypeSafe, only under the opt-in, and the startup line names it when the opt-in is on.
- **FR-006 Dates and numbers**: no exclusion. The roadmap §6.2 kept claims with dates or numbers out of `unverified_fact`; 066 drops that exclusion, because the question now checks against the turn's tool output and dates and numbers are the class most prone to ungrounded claims (`WATCHDOG_LAYER.md:153`).
- **FR-007 Recommendations**: each recorded signal names a corrective in its row's label: `pushback_reversal` → `verify` (always in the catalog); `unverified_fact` → `grounded_verify` when configured, else `research` when configured, else none. In shadow nothing is shown.
- **FR-008 Cooldown**: none while the signals are shadow only; every occurrence is recorded. The cooldown (identity: session plus signal kind) and its length come with promotion to `on`.
- **FR-009 Review skip**: per User Story 2. Thresholds come from a per-model pinned table of no-band values for the two turn-review questions, empty until the promotion rule passes. The skip applies only with memory configured. Any memory, preference or transcript candidate blocks it. While the skip is off, Jev and the review run concurrently as today; when on, Jev answers first under the turn deadline, and the review starts only if the turn is not skipped. The audit share is decided from the evaluation id: a turn whose id hash falls in the bottom 10% gets the review anyway.
- **FR-010 Configuration**: `PARALLAX_JEV_MODE_TURN_SIGNALS` (`off` | `shadow`, unset is `off`); `PARALLAX_JEV_SEND_USER_TEXT` (`true` | `false`, unset is `false`; `true` with the signals site off is a startup error); unparseable values and unknown `PARALLAX_JEV_*` names are startup errors; `PARALLAX_JEV_MODE_TURN_REVIEW` accepts `on` only with a pinned threshold entry and a recorded order check (both criteria orders, at most 5% flips, `NEW_SERVER_DESIGN.md:304`) for the configured model. The two turn-review questions gain declared criteria for that check.
- **FR-011 Records**: one `jev_records` row per covered turn end, site `turn_signals`, joined to the checkpoint record, holding the questions asked; an `excluded` row with no request when none is asked. The checkpoint record gains `review_skipped_by` (null, `jev` or `audit`), with a migration and a record-contract update, mirrored in the OTLP export. The checkpoint record's `jev_cost_usd` sums every Jev row of the evaluation.
- **FR-012 Contracts**: the `checkpoint_turn` signal enum is bound by test to its contract; `specs/007-observability-layer/contracts/telemetry.md` line 97 changes `parallax.jev.requests` to a count (0-2) and the cost to the sum of the evaluation's Jev rows, and gains `review_skipped_by`.
- **FR-013 Amendments**: `MEMORY_LAYER.md` (017 rails) and a 017 spec note, because 017 decided the review runs at every turn end; `WATCHDOG_LAYER.md:59`, because the skip lets a Jev answer stop the review; `CORRECTIVE_SELECTION.md:89`, adding 066's uses; and the FR-005 exception.

### Key Entities

- **Turn signal row**: a `jev_records` row with site `turn_signals`: the questions asked and their answers, each signal's corrective in the label, model, outcome, cost, and the checkpoint record id.
- **Skip reason**: `review_skipped_by` on the checkpoint record: null when the review ran normally, `jev` when Jev skipped it, `audit` when the audit share kept it.
- **No-band table**: per model, the pinned no-band threshold for each turn-review question; empty until the promotion rule passes.

## Promotion rule

Quoted verbatim from the Jev roadmap §7 (shadow → on, per question):

> 1. **Positive means the class whose miss causes harm:**
>    - for U2: the review flagged or captured;
>    - for U4: the ensemble refuted or contested the claim.
> 2. **Skip gates (U2, U4):**
>    - the one-sided 95% Clopper-Pearson lower bound on recall over harmful positives must clear the target;
>    - for U4, the rate of Jev-settled claims the ensemble did not support must stay below a stated cap;
>    - the skip share's lower bound must be at least a stated minimum.
> 3. **Flags (U1, U3, U5):**
>    - precision is estimated at natural prevalence;
>    - the false-positive rate's upper bound is tight, e.g. 0 of 300 negatives flagged;
>    - at least 60 flags;
>    - a recall floor.
> 4. **Sample sizes follow from the target.** For a 95% recall target, ≥ 59 positives with 0 misses or ≥ 93 with 1 miss, on the sealed confirmation set. The tune set is separate.
> 5. **Baselines.** Results must beat always-no and always-yes, with the paired targets above, so no single number can be gamed.
> 6. **Clustering.** Intervals are bootstrapped by research run or session, and the number of clusters is reported.
> 7. **Pinning.** Thresholds are pinned to Jev version, big-model id and prompt versions. Any change invalidates promotion and is a startup error for `on`.
> 8. **After promotion.**
>    - **Audit sample:** a random 5–10% of items Jev would skip still go to the big model, weighted.
>    - **Drift check:** a rolling Wilson lower bound on recall over the last 60 audited positives. When it falls below the target, the site reverts to shadow, the revert is recorded, and the operator is told.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: with the signals site in shadow and off, the checkpoint result and record are identical except latency and `jev_cost_usd`, for confident, failing, rejected and never-returning Jev answers.
- **SC-002**: with `PARALLAX_JEV_SEND_USER_TEXT` unset, no request body contains user text; and in every mode, no request body contains recalled memory text, checked on the whole serialized body.
- **SC-003**: `PARALLAX_JEV_MODE_TURN_REVIEW=on` without a pinned threshold entry and recorded order check fails startup.
- **SC-004**: a live example asserts one joined `turn_signals` row per covered turn end and exits non-zero on failure.
- **SC-005**: with the skip on in a test, the time Jev adds before a non-skipped review is at most the turn deadline (1000 ms), and the median added time is reported.
- **SC-006**: every flag the templates build is dropped by the user-text parser, checked by building each flag through its template.

## Assumptions

- The skip stays off until the promotion rule passes for the turn-review site.
- Turn signals stay in shadow in 066; promoting them to `on` is a later feature.
- Tool-call risk classes are 067.
