# Feature Specification: Preference Enforcement at the Checkpoint

**Feature Branch**: `015-preference-enforcement`

**Created**: 2026-07-21

**Status**: Draft

**Input**: User description: "Preference enforcement via the checkpoint layer — close the enforcement gap named in PREFERENCE_ELICITATION.md. Today elicit (014) surfaces preferences and the memory layer (003) stores them verified-before-stored, but nothing enforces them: a stored preference the model ignores is the status quo ("heard you, did it anyway"). This feature makes the checkpoint layer (006) the enforcement point: when the checkpoint tools evaluate a trajectory, the server recalls trusted stored preferences relevant to the current turn via the existing recall seam (the same seam elicit already uses, gated on memory being configured) and checks the trajectory/final message against them, flagging violations through the existing silence/flag/hold verdict model. Flag-and-let-the-model-revise is the default authority (per the design doc's "likely revise, except hard bans"); the layer never rewrites output. Fail-open like the rest of the checkpoint layer, one checkpoint_records audit row per evaluation, and violations must name the specific preference violated and its provenance so the model can fix or push back. When memory is not configured, checkpoint behavior is unchanged."

## Clarifications

### Session 2026-07-21

- Q: How are enforceable preferences identified among recalled memories? → A: No marker — all recalled trusted lessons/facts are candidates; the single review pass judges whether each expresses a preference the final message violates; non-preference facts fall to silence via decline-bias.
- Q: What evidence is judged for violations — the final message alone, or the final message plus the bounded transcript tail the review already reads? → A: Final message + bounded trajectory tail. Enforcement stays at end of turn, but the judgment covers the whole turn — output wording and observable in-turn behavior — so process preferences ("always run the test gate before claiming done") are enforceable, not just wording preferences.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A stored preference violation is flagged at end of turn (Priority: P1)

A user has a standing, trusted preference on record (stated directly, or captured
from a prior session's revealed signals — e.g. "never use word X in final
messages", "all new code uses only the standard library"). During a later
session the assistant produces a turn whose final output violates that
preference. When the harness's end-of-turn checkpoint fires, the server recalls
the stored preferences relevant to this turn, detects the violation, and returns
a **flag** that quotes the violated preference verbatim and names where it came
from (its stored identity and trust standing). The assistant sees the flag,
and can revise the output — or push back explicitly if the preference does not
apply — instead of silently ignoring what the user asked for.

**Why this priority**: This is the entire point of the feature — the
capture → store → recall → **enforce** loop's missing last step. Without it, a
stored preference the model ignores is indistinguishable from no preference at
all ("heard you, did it anyway"). Everything else in this feature exists to make
this scenario safe to ship.

**Independent Test**: Seed one trusted preference into the memory store, run an
end-of-turn checkpoint evaluation whose final message plainly violates it, and
confirm the verdict is a flag that names that specific preference and its
provenance. Delivers value standing alone: one seeded preference + one
violating turn = one actionable flag.

**Acceptance Scenarios**:

1. **Given** a trusted stored preference "final messages must never contain the
   word 'delve'" and memory configured, **When** the end-of-turn checkpoint
   evaluates a turn whose final message contains "delve", **Then** the verdict
   is a flag whose reason quotes the stored preference verbatim and includes the
   stored memory's identity so the assistant can revise or contest it.
2. **Given** the same stored preference, **When** the end-of-turn checkpoint
   evaluates a turn whose final message complies with it, **Then** the verdict
   is silence — no flag is raised.
3. **Given** a violation flag was returned, **When** the assistant revises its
   message in response, **Then** the layer has never modified any output
   itself — enforcement surfaces the violation; the assistant fixes it.

---

### User Story 2 - Enforcement never degrades a session (Priority: P2)

A user runs sessions in every configuration Parallax supports: with memory
configured, without it, with a slow or failing memory backend, and under forced
continuation. Preference enforcement must be invisible except when a real
violation exists: no false alarms on compliant turns, no new errors when the
memory capability is absent or failing, no evaluation loops on continuation
turns, and no growth in the number of independent model passes the checkpoint
layer performs per evaluation.

**Why this priority**: Alarm fatigue is the checkpoint layer's named
make-or-break — a noisy or fragile enforcement signal is worse than none,
because every flag gets ignored and the layer loses its authority. The
enforcement signal is only worth shipping if it cannot hurt the sessions it
does not help.

**Independent Test**: Run the existing checkpoint evaluation suite with memory
unconfigured and confirm behavior is unchanged; inject a recall failure and
confirm the verdict is silence (fail-open) rather than an error; run a
continuation turn and confirm enforcement is skipped.

**Acceptance Scenarios**:

1. **Given** memory is not configured, **When** any checkpoint tool is invoked,
   **Then** verdicts and behavior are identical to the current release — the
   enforcement signal is silently inactive.
2. **Given** memory is configured but the recall path fails mid-evaluation,
   **When** the end-of-turn checkpoint runs, **Then** the verdict falls open to
   silence and no error reaches the harness.
3. **Given** a turn end that follows a forced continuation, **When** the
   end-of-turn checkpoint runs, **Then** preference enforcement does not
   evaluate (screening only), so a flag can never cause an unbounded
   revise-and-recheck loop.
4. **Given** stored memories that are untrusted (external, unverified),
   **When** an end-of-turn checkpoint evaluates any turn, **Then** untrusted
   memories never produce an enforcement flag.

---

### User Story 3 - Every enforcement evaluation is auditable (Priority: P3)

An operator tuning the checkpoint layer wants to know whether preference
enforcement is precise or noisy. Every end-of-turn evaluation records exactly
one audit row stating whether the enforcement signal was active, whether it
fired, and against which stored preference — so catch-rate versus noise is
measurable from day one, the same way it is for every existing checkpoint
signal.

**Why this priority**: The design corpus's hardest open problem is knowing
whether a layer helps or hurts. Without the audit trail, the P2 promise
("no alarm fatigue") is unfalsifiable. It is P3 because it observes the feature
rather than delivering or protecting it.

**Independent Test**: Run evaluations that produce a flag, a silence, and an
inactive (memory-off) outcome, and confirm each left exactly one audit row
whose recorded outcome matches the returned verdict.

**Acceptance Scenarios**:

1. **Given** any end-of-turn checkpoint evaluation completes (flag, silence, or
   fail-open), **When** the operator inspects the audit records, **Then**
   exactly one row exists for that evaluation and it reports whether preference
   enforcement was evaluated and what, if anything, fired.
2. **Given** an enforcement flag fired, **When** the operator inspects that
   evaluation's audit row, **Then** the row identifies the stored preference
   that fired it.

---

### Edge Cases

- **Preference conflicts with the current explicit instruction.** The user's
  live instruction in this session contradicts a stored preference ("just this
  once, use the informal tone"). The flag still fires — enforcement does not
  adjudicate; it surfaces the stored preference with provenance precisely so
  the assistant can push back with "the stored preference says X, but you asked
  for Y here" and proceed correctly. Flag-and-revise authority keeps this safe:
  a wrong flag costs one visible notice, never a blocked turn.
- **Stale or superseded preference.** A preference stored months ago may no
  longer hold. Recall ranking already weights recency; a stale flag is
  contestable by design (provenance is attached), and the user can delete the
  memory. No supersession logic is in scope for this feature.
- **Multiple violated preferences in one turn.** The flag must remain a single
  actionable verdict; when several trusted preferences are violated, the most
  relevant one is named. The audit row records what was evaluated.
- **Preference-shaped content the model cannot judge.** Vague or aspirational
  stored content ("be excellent") gives the judgment pass nothing concrete to
  check; uncertain judgments resolve to silence (decline-biased), never to a
  speculative flag.
- **Very long final messages.** Evaluation operates within the checkpoint
  layer's existing bounded reads; enforcement introduces no unbounded input.
- **Recall returns nothing relevant.** Silence, and the audit row still shows
  the signal was evaluated.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: During an end-of-turn checkpoint evaluation with the memory
  capability configured, the system MUST recall stored memories relevant to the
  turn's final message and evaluate every recalled **trusted** durable-knowledge
  or lesson memory as a violation candidate. No stored marker distinguishes
  preferences: the judgment pass itself decides whether a candidate expresses a
  standing preference the turn violates, and candidates that do not
  (ordinary facts) resolve to silence via FR-004's decline bias. The judged
  evidence is the turn as a whole: the final message plus the bounded
  transcript tail the end-of-turn review already reads, so both wording
  preferences (violated in the final message) and process preferences
  (violated by observable in-turn behavior) are enforceable.
- **FR-002**: When a violation is found, the system MUST return a flag verdict
  whose reason (a) quotes the violated stored preference verbatim and (b)
  names its provenance — the stored memory's identity and trust standing — so
  the assistant can revise the output or explicitly contest the preference.
- **FR-003**: Flag MUST be the maximum authority for preference enforcement:
  the enforcement signal never holds an action, never blocks a turn, and the
  layer never rewrites any output.
- **FR-004**: Enforcement judgment MUST be decline-biased: when it is uncertain
  whether the final message violates the preference, the outcome is silence. A
  compliant turn MUST produce silence.
- **FR-005**: Untrusted stored memories MUST never produce an enforcement flag.
- **FR-006**: When the memory capability is not configured, checkpoint behavior
  MUST be unchanged from the current release; the enforcement signal is
  silently inactive.
- **FR-007**: Enforcement MUST fail open: any failure in recall or violation
  judgment resolves the signal to silence and MUST NOT surface an error to the
  harness.
- **FR-008**: Every end-of-turn evaluation MUST write exactly one audit record,
  and that record MUST state whether preference enforcement was evaluated and,
  when a flag fired, which stored preference fired it.
- **FR-009**: On a turn end that follows a forced continuation, enforcement
  MUST NOT evaluate (screening only), so a flag can never cause an unbounded
  continuation loop.
- **FR-010**: Enforcement MUST NOT increase the number of independent model
  passes the end-of-turn evaluation performs; it extends the existing single
  review pass rather than adding a second.

### Key Entities

- **Stored preference**: a trusted memory (first-hand or verified provenance)
  whose content expresses a standing preference or constraint on the
  assistant's behavior or output. Not a new stored type — the enforceable
  population is the same trusted durable-knowledge/lesson population the
  checkpoint gate already treats as constraints, now also judged against the
  completed turn (final message + observable in-turn behavior).
- **Enforcement signal**: the new checkpoint signal kind — carries the quoted
  preference, the stored memory's identity, and the violation basis; resolves
  only to silence or flag.
- **Audit record**: the existing one-row-per-evaluation checkpoint record,
  extended to show whether enforcement was evaluated and what fired.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With one seeded trusted preference and a final message that
  plainly violates it, the end-of-turn checkpoint returns a flag naming that
  preference and its provenance — demonstrable live end-to-end (seed →
  violate → flag → revise) in a single session.
- **SC-002**: With the same seeded preference and a compliant final message,
  the verdict is silence; across the feature's compliant-turn test scenarios,
  zero enforcement flags fire.
- **SC-003**: With memory unconfigured, the full existing checkpoint behavior
  is unchanged: all current checkpoint scenarios produce identical verdicts
  before and after this feature.
- **SC-004**: With a recall failure injected, 100% of evaluations still return
  a verdict (silence) rather than an error, and still write their audit row.
- **SC-005**: Every end-of-turn evaluation — flag, silence, inactive, or
  fail-open — leaves exactly one audit record from which an operator can
  compute enforcement precision (flags fired vs. evaluations run) without any
  additional instrumentation.

## Assumptions

- **No new stored type.** Preferences live in the existing memory store as
  trusted durable knowledge or lessons (the same population the checkpoint
  gate treats as constraints). Introducing a dedicated preference record with
  attribute/strength/decay fields (the design doc's fuller sketch) is out of
  scope; this feature enforces what the store already holds.
- **End-of-turn is the enforcement point.** Enforcement fires when the turn
  ends — the action gate's existing constraint hold already covers the
  pending-action case and is unchanged by this feature, and the batch boundary
  remains screening-only. The judged evidence at that point is the turn as a
  whole (final message + the bounded transcript tail the review already
  reads), per the second clarification — covering both what the turn said and
  what it observably did.
- **Flag-only authority.** The design doc's open question "block vs
  flag-and-revise" is resolved to flag-and-revise for this feature, per its own
  lean ("likely revise, except hard bans"). Hard bans that justify holding a
  turn are deferred until audit data shows flags being ignored.
- **Capture is out of scope.** Passive capture of revealed preferences
  (watching edits/rejections/complaints) is a separate future feature; this
  feature enforces preferences however they were stored (directly via save, or
  seeded from elicit-surfaced findings).
- **The memory capability gate is the existing one** — presence of the
  embeddings configuration enables recall; absent, the checkpoint tools behave
  exactly as today.
