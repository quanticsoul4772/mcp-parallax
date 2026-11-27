# Feature Specification: Jev at turn end

**Feature Branch**: `066-jev-turn-end`

**Created**: 2026-11-26

**Status**: Draft

**Input**: the Jev roadmap §6.2: the U2 end-of-turn review skip, `on` only once the promotion rule passes; U1 turn signals `pushback_reversal` and `unverified_fact`, shadow only. The promotion rule is quoted under Promotion rule below.

## Clarifications

### Session 2026-11-26

- Q: How long is the turn-signal cooldown? → A: None while the signals only record; every occurrence is recorded. The cooldown length is set when the signals are promoted.
- Q: What share of turns always gets the review when the skip would skip it? → A: 10%, the top of the promotion rule's 5-10% range.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Turn signals recorded (Priority: P1)

At each turn end, two turn-level failures the calling model cannot see in itself (`WATCHDOG_LAYER.md`, "What it watches for": the "Sycophantic flip" and "Hallucination / ungrounded claim" rows) are checked by the cheap detectors the design corpus names, in code, on the machine. No Jev call is made for them and nothing is sent anywhere. The results are recorded with anchors back to the transcript; nothing is flagged and the turn's result does not change.

- `pushback_reversal`: a real user prompt sits between the previous final message and this one, no tool call follows it, and the existing pair miner (lexical overlap plus opposing polarity, run without stored memories) finds a candidate pair between the previous and final assistant messages.
- `unverified_fact`: entities in the final message (file paths, version strings, identifiers and hashes of 7 or more characters, numbers of 3 or more digits) are looked up verbatim, after normalization, in the turn's tool inputs, tool outputs and user prompt. Entities not found are recorded with their kind.

**Why this priority**: the recorded fire rates and anchors are what a later feature needs to design and back-test Jev questions for these signals and to label them.

**Independent Test**: with `CHECKPOINT_TURN_SIGNALS=on`, fixture transcripts produce one `turn_signal_records` row per turn end with the expected detector results, and the checkpoint result and record are unchanged.

**Acceptance Scenarios**:

1. **Given** `CHECKPOINT_TURN_SIGNALS=on`, **When** a non-continuation turn with a final message ends, **Then** one row records each signal's result or exclusion reason, the transcript anchors, and each unmatched entity's kind, whether or not the review ran.
2. **Given** a continuation turn, **When** the turn ends, **Then** no row is written, and the previous final message is updated.
3. **Given** the turn start is not found in the transcript read, **When** the turn ends, **Then** both signals record the exclusion reason `turn start not found`.
4. **Given** the signals are on, **When** a turn ends, **Then** no new request leaves the machine.

---

### User Story 2 - Review skip, off until measured (Priority: P2)

When the 064 turn-review questions are confidently "no" and nothing blocks the skip (FR-009), the end-of-turn review can be skipped, saving a big-model call. The skip is built and tested now and stays off: `on` is refused at startup.

**Why this priority**: the skip saves the most of any Jev use at turn end, but promotion needs data not yet collected and a drift check not yet built.

**Independent Test**: with the skip enabled in a test through injected thresholds, each skip condition made false or missing in turn makes the review run.

**Acceptance Scenarios**:

1. **Given** the skip is enabled in a test, memory is configured, both 064 answers are below their thresholds, no memory, preference or transcript candidate exists, the state is not truncated and the turn is outside the audit share, **When** the turn ends, **Then** the review does not run, `review_skipped_by` is `jev`, and the 064 row's `applied` is 1.
2. **Given** any of those conditions fails or an answer is missing, **When** the turn ends, **Then** the review runs as today.
3. **Given** a turn in the audit share that the skip would have skipped, **When** the turn ends, **Then** the review runs and `review_skipped_by` is `audit`.
4. **Given** `PARALLAX_JEV_MODE_TURN_REVIEW=on`, **When** the server starts, **Then** startup fails naming the variable, until a pinned threshold entry for the Jev model, big-model id and question versions, a recorded order check, and the drift check all exist.

---

### User Story 3 - Configuration and records (Priority: P3)

The signals are off unless set, and records have published contracts.

**Why this priority**: off by default is the project convention; contracts keep records and docs from drifting.

**Independent Test**: configuration tests, and binding tests between each table and its schema.

**Acceptance Scenarios**:

1. **Given** `CHECKPOINT_TURN_SIGNALS` is unset, **When** a turn ends, **Then** no transcript turn parsing runs and no row is written.
2. **Given** an unparseable value, or a `PARALLAX_JEV_*` name outside the known set, **When** the server starts, **Then** startup fails naming the variable.

### Edge Cases

- Continuation turns write no row but update the previous final message.
- The turn start is the last line the transcript spike identifies as a real user prompt; subagent lines, hook-injected context (including memory text from `surface`), interruption markers, meta lines, compaction summaries and lines containing a Parallax flag prefix anywhere are skipped.
- With no previous final message held (the first turn, or after a restart), `pushback_reversal` records `no previous final message`.
- A transcript read failure records `transcript unreadable` for both signals and changes nothing else.
- A signal that fires on several turns of one session is recorded each time.
- With memory off, the skip never applies; the signals are still checked.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001 Transcript spike**: before parser work, a spike records the shapes in Claude Code transcripts that 066 reads: user prompts (string content and text blocks), subagent lines, hook-injected context, interruption markers, meta lines, compaction summaries, tool inputs, tool results, and Stop-hook block reasons. Fixtures are scrubbed: no user names, no paths containing an account name, no session ids, no real prompt text; a test checks the fixtures for these.
- **FR-002 Turn data**: the transcript reader's `read` returns the window and the raw tail lines from one read (one session check, one snapshot). A separate function, `parse_turn` in `src/checkpoint/turn_view.rs`, reads those lines at turn end only and returns the turn start (or not found), its line id, the user prompt, and the turn's tool inputs and outputs, raw and kept on the machine. `parse_lines`, `TrajectoryEntry` and `TrajectoryWindow` are unchanged.
- **FR-003 Detectors**: `pushback_reversal` and `unverified_fact` run per User Story 1 at every non-continuation turn end with a final message, independent of whether the review runs. No Jev call is made for them.
- **FR-004 Nothing sent**: the signals send nothing off the machine. The only Jev request at turn end remains 064's turn-review request, which carries only assistant-written text, redacted, and never the user's messages or tool results as sources. A test plants sentinel strings in fixture user prompts and tool results and checks they appear in no request body.
- **FR-005 Previous final message**: held in memory in the checkpoint dependencies, updated at every turn end including continuation turns; nothing about it is written to disk.
- **FR-006 Entities**: file paths (slashes and JSON escaping normalized), version strings, code-shaped identifiers and hashes of 7 or more characters (a digit, an underscore, or a capital after the first character; research D6), and numbers of 3 or more digits matched on word boundaries. No names. ISO dates (`YYYY-MM-DD`) are skipped; other date forms are not recognized as dates, so a bare year counts as a number. The roadmap §6.2 kept dates and numbers away from Jev; with no Jev call, numbers are settled by verbatim match and ISO dates are left out because they cannot be matched reliably against the turn's evidence.
- **FR-007 Recommendations**: each recorded signal names a corrective: `pushback_reversal` → `verify`; `unverified_fact` → `grounded_verify` when configured, else `research` when configured, else none. Nothing is shown.
- **FR-008 Cooldown**: none while the signals only record.
- **FR-009 Review skip**: per User Story 2. The 064 turn-review questions become version 2 with declared criteria (needed for the order check); version-1 rows do not count toward promotion. Thresholds come from a pinned table keyed by Jev model, big-model id and question versions, empty in 066. The skip applies only with memory configured. Any memory, preference or transcript candidate blocks it. While the skip is off, Jev and the review run concurrently as today; when on, Jev answers first under the turn deadline. The audit share is decided from the evaluation id: a turn whose id hash falls in the bottom 10% gets the review anyway. The evaluation id and thresholds can be injected through the checkpoint dependencies for tests.
- **FR-010 Configuration**: `CHECKPOINT_TURN_SIGNALS` (`off` | `on`, unset is `off`); unparseable values and `PARALLAX_JEV_*` names outside the known set are startup errors; `PARALLAX_JEV_MODE_TURN_REVIEW=on` is refused until a pinned threshold entry, a recorded order check on version 2 showing a flip rate of 5% or less at the pinned threshold (`NEW_SERVER_DESIGN.md` §5), and the drift check (promotion rule 8, deferred to the promotion feature) exist. The `on` startup line uses turn-review wording.
- **FR-011 Records**: a new table `turn_signal_records`, one row per covered turn end, joined to the checkpoint record by `eval_id`: per signal, the result or exclusion reason and the recommendation; the transcript reference and the line ids of the turn start and the final message; each unmatched entity's kind and a hash of its text. Its schema is published and bound to the table by a two-way test. The checkpoint record gains `review_skipped_by` (null, `jev` or `audit`), with a migration and a record-contract update, mirrored in the OTLP export.
- **FR-012 Contracts**: the `checkpoint_turn` signal enum is bound by test to its contract; `specs/007-observability-layer/contracts/telemetry.md` gains `review_skipped_by`. The Jev request count and cost per turn are unchanged (one turn-review row).
- **FR-013 Amendments**: `MEMORY_LAYER.md` (017 rails) and a 017 spec note, because 017 decided the review runs at every turn end; `WATCHDOG_LAYER.md:59`, because the skip lets a Jev answer stop the review and the signals run at every turn end outside the review's gate; `CORRECTIVE_SELECTION.md:89`, adding 066's uses; `NEW_SERVER_DESIGN.md` §10's configuration table, adding `CHECKPOINT_TURN_SIGNALS` as operator-owned; the CHANGELOG.
- **FR-014 Flag text**: every flag template (review, preference, batch, hold, surface) exports a fixed prefix; the turn-start finder skips any line containing one anywhere.

### Key Entities

- **Turn data**: what `parse_turn` returns, raw, never sent or stored.
- **Turn signal record**: a `turn_signal_records` row: per-signal results, exclusion reasons and recommendations, transcript anchors, unmatched entity kinds and hashes.
- **Skip reason**: `review_skipped_by` on the checkpoint record: null, `jev` or `audit`.
- **Pinned threshold table**: per Jev model, big-model id and question versions, the no-band thresholds for the two turn-review questions; empty in 066.

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

- **SC-001**: with the signals on and off, the checkpoint result and record are identical except latency.
- **SC-002**: sentinel strings planted in fixture user prompts and tool results appear in no request body, in every mode.
- **SC-003**: `PARALLAX_JEV_MODE_TURN_REVIEW=on` fails startup.
- **SC-004**: a live example runs fixture transcripts through `run_turn` with the signals on, asserts one joined row per covered turn end with the expected detector results, and exits non-zero on failure.
- **SC-005**: with the skip enabled in a test, the time Jev adds before a non-skipped review is at most the turn deadline (1000 ms).
- **SC-006**: every flag the templates build is skipped by the turn-start finder, checked by building each flag through its template.
- **SC-007**: every existing test of the screens, review mining and 064's turn rows passes, and the only change to those tests is the reader mock returning an empty line list. Other existing test fixtures gain only the new fields (`version`, `review_skipped_by`, `yes_no_order`), with their assertions unchanged.

## Assumptions

- The skip stays off in 066.
- Jev questions for the turn signals, their back-test, and labels come in a later feature, built on the recorded anchors.
- The drift check is built with the promotion feature.
- Tool-call risk classes are 067.
- The promotion feature fixes the skip's server wiring when it lifts the `on` refusal: today the turn-review Jev client is built only for `shadow`, a missing threshold entry falls back to no thresholds instead of failing startup, and `order_checked` has no production caller.
- The promotion feature also records a skipped turn's Jev row and span as mode `on` (both say `shadow` today) and settles `wait_ms`, which today means total Jev time on the skip path but Jev time beyond the review on the side-by-side path.
