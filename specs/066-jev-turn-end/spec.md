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

At each turn end, two turn-level failures the calling model cannot see in itself (`WATCHDOG_LAYER.md:151,153`) are checked. Code runs the cheap detector the design corpus names for each signal first, and Jev is asked only about what the detector leaves open. Nothing the user wrote and nothing a tool returned is sent off the machine. The results are recorded; nothing is flagged and the turn's result does not change.

- `pushback_reversal`: code finds the structure (a real user prompt between the previous final message and this one, with no tool call after it) and runs the existing pair miner over the previous and final assistant messages. Only when it finds a candidate pair is Jev asked whether the final message reverses the earlier one. Jev sees only the two assistant messages, never the user's text (`NEW_SERVER_DESIGN.md:81,193,198-200`). This differs from 064's `contradiction` question: it is gated on the pushback structure and compares only the previous and final positions.
- `unverified_fact`: code pulls the entities in the final message (numbers, dates, paths, identifiers, names) and looks each one up verbatim in the turn's tool inputs, full tool outputs and the turn's user prompt, all on the machine. Entities found there are supported. Only claims with unmatched entities are sent to Jev, and only the claim text from the final message.

**Why this priority**: recording these is the measurement any later promotion needs.

**Independent Test**: with the signals in shadow, a stand-in Jev answering yes leaves the checkpoint result and record unchanged (cost set aside) and writes one `turn_signals` row recording the detector results and Jev's answers.

**Acceptance Scenarios**:

1. **Given** the signals site is `shadow`, **When** a non-continuation turn with a final message ends, **Then** one `turn_signals` row records the detector results and any Jev answers, whether or not the review ran.
2. **Given** a continuation turn, **When** the turn ends, **Then** no row is written.
3. **Given** every entity in the final message is found in the turn's evidence and the pair miner finds no candidate, **When** the turn ends, **Then** the row is `excluded`, no request is sent, and the row records the detector results.
4. **Given** any turn, **When** a request is sent, **Then** its body contains no user text and no tool output.

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

The new site is off unless set, and every turn end the site covers leaves a record.

**Why this priority**: off by default is a constitution requirement; records are how the shadow data is read later.

**Independent Test**: configuration error tests, and a records test joining every signal row to its checkpoint record.

**Acceptance Scenarios**:

1. **Given** `PARALLAX_JEV_MODE_TURN_SIGNALS` is unset, **When** the server starts, **Then** the site is off and no signal call is made.
2. **Given** the site is `shadow` without `TYPESAFE_API_KEY`, **When** the server starts, **Then** startup fails naming the variable.
3. **Given** an unparseable value or an unknown `PARALLAX_JEV_*` name, **When** the server starts, **Then** startup fails naming the variable.

### Edge Cases

- Continuation turns never ask the signal questions.
- The turn start is the last line the transcript spike identifies as a real user prompt; subagent lines, hook-injected context (including memory text from `surface`), interruption markers, meta lines, compaction summaries and lines carrying Parallax's flag text are skipped. When none is found inside the transcript read (the last 2 MiB), the row is `excluded` with the reason `turn start not found`.
- The previous final message is the last final message the server received for the session; with none held (the first turn, or after a restart), `pushback_reversal` is excluded for that turn.
- A signal that fires on several turns of one session is recorded each time (no cooldown in shadow).
- A Jev error, timeout or rejected answer records the failure and changes nothing.
- With memory off, the skip never applies; the signals are still checked.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001 Transcript spike**: before parser work, a spike records the shapes in Claude Code transcripts that 066 reads: user prompts (string content and text blocks), subagent lines, hook-injected context, interruption markers, meta lines, compaction summaries, tool inputs, tool results, and Stop-hook block reasons, with a fixture for each.
- **FR-002 Turn data**: a separate function, `parse_turn` in `src/checkpoint/turn_view.rs`, reads the same transcript lines at turn end only (never on the batch path) and returns the turn's start, its user prompt, its tool inputs and full tool outputs, raw and kept on the machine, with "turn start not found" as a possible result. `parse_lines`, `TrajectoryEntry` and `TrajectoryWindow` are unchanged, so every existing signal, review candidate and 064 replay fingerprint is unchanged.
- **FR-003 Detectors and questions**: `pushback_reversal` and `unverified_fact` are yes/no questions, versioned, each asked only when its cheap detector leaves something open (User Story 1), in one request under the turn deadline, at every non-continuation turn end with a final message, independent of whether the review runs and of the turn-review mode. Requests share the existing limit of 4 in flight.
- **FR-004 What is sent**: for `pushback_reversal`, the previous and final assistant messages; for `unverified_fact`, the unmatched claims from the final message. Assistant text is redacted whole, then cut to fixed per-item and total caps, and truncation is recorded on the row. User text, tool inputs, tool outputs and stored memories are never sent.
- **FR-005 Previous final message**: the server holds, in memory, the last final message it received for each harness session; nothing about it is written to disk.
- **FR-006 Dates and numbers**: no exclusion. The roadmap §6.2 kept claims with dates or numbers out of `unverified_fact`; 066 drops that exclusion because code now settles them by verbatim match against the turn's evidence (`WATCHDOG_LAYER.md:153` names entity diffs for exactly this class).
- **FR-007 Recommendations**: each recorded signal names a corrective in its row's label: `pushback_reversal` → `verify` (always in the catalog); `unverified_fact` → `grounded_verify` when configured, else `research` when configured, else none. In shadow nothing is shown.
- **FR-008 Cooldown**: none while the signals are shadow only; every occurrence is recorded. The cooldown (identity: session plus signal kind) and its length come with promotion to `on`.
- **FR-009 Review skip**: per User Story 2. Thresholds come from a per-model pinned table of no-band values for the two turn-review questions, empty until the promotion rule passes. The skip applies only with memory configured. Any memory, preference or transcript candidate blocks it. While the skip is off, Jev and the review run concurrently as today; when on, Jev answers first under the turn deadline, and the review starts only if the turn is not skipped. The audit share is decided from the evaluation id: a turn whose id hash falls in the bottom 10% gets the review anyway.
- **FR-010 Configuration**: `PARALLAX_JEV_MODE_TURN_SIGNALS` (`off` | `shadow`, unset is `off`); unparseable values and unknown `PARALLAX_JEV_*` names are startup errors; `PARALLAX_JEV_MODE_TURN_REVIEW` accepts `on` only with a pinned threshold entry and a recorded order check (both criteria orders, at most 5% flips, `NEW_SERVER_DESIGN.md:304`) for the configured model. The two turn-review questions gain declared criteria for that check. The startup line names the signals site's destination and data class (assistant text and claims only).
- **FR-011 Records**: one `jev_records` row per covered turn end, site `turn_signals`, joined to the checkpoint record, holding the detector results (entities matched and unmatched, whether a candidate pair was found), the questions asked and their answers, and a hash of what was sent, never the text; an `excluded` row with no request when nothing is asked. The checkpoint record gains `review_skipped_by` (null, `jev` or `audit`), with a migration and a record-contract update, mirrored in the OTLP export. The checkpoint record's `jev_cost_usd` sums every Jev row of the evaluation.
- **FR-012 Contracts**: the `checkpoint_turn` signal enum is bound by test to its contract; `specs/007-observability-layer/contracts/telemetry.md` line 97 changes `parallax.jev.requests` to a count (0-2) and the cost to the sum of the evaluation's Jev rows, and gains `review_skipped_by`.
- **FR-013 Amendments**: `MEMORY_LAYER.md` (017 rails) and a 017 spec note, because 017 decided the review runs at every turn end; `WATCHDOG_LAYER.md:59`, because the skip lets a Jev answer stop the review and the signals run at every turn end behind their own cheap detectors rather than the review's gate; `CORRECTIVE_SELECTION.md:89`, adding 066's uses; `NEW_SERVER_DESIGN.md` §10's configuration table, adding `PARALLAX_JEV_MODE_TURN_SIGNALS` as operator-owned.
- **FR-014 Flag text**: the flag templates export fixed prefixes; the turn-start finder skips any line containing one of them anywhere in the line.

### Key Entities

- **Turn data**: what `parse_turn` returns: the turn start (or not found), the user prompt, and the turn's tool inputs and outputs, raw, never sent or stored.
- **Turn signal row**: a `jev_records` row with site `turn_signals`: detector results, the questions asked and their answers, each signal's corrective in the label, a hash of what was sent, model, outcome, cost, and the checkpoint record id.
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
- **SC-002**: no request body contains user text, tool inputs, tool outputs or recalled memory text, checked on the whole serialized body, in every mode.
- **SC-003**: `PARALLAX_JEV_MODE_TURN_REVIEW=on` without a pinned threshold entry and recorded order check fails startup.
- **SC-004**: a live example asserts one joined `turn_signals` row per covered turn end and exits non-zero on failure.
- **SC-005**: with the skip on in a test, the time Jev adds before a non-skipped review is at most the turn deadline (1000 ms), and the median added time is reported.
- **SC-006**: every flag the templates build is skipped by the turn-start finder, checked by building each flag through its template.
- **SC-007**: existing checkpoint behavior is unchanged: every existing test of the screens, review mining and 064's turn rows passes without modification.

## Assumptions

- The skip stays off until the promotion rule passes for the turn-review site.
- Turn signals stay in shadow in 066; promoting them to `on` is a later feature.
- Tool-call risk classes are 067.
