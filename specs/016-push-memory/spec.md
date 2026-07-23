# Feature Specification: Push Memory

**Feature Branch**: `016-push-memory`

**Created**: 2026-07-23

**Status**: Draft

**Input**: User description: "Push memory — proactive recall surfaced at prompt time. The design corpus (`MEMORY_LAYER.md`) names push as 'what turns memory from unused to load-bearing': the old server's manual-recall capability died at 0 uses because the model had to remember to ask. Parallax's `recall` is the pull half; this feature adds the push half — when the user starts a turn, the server surfaces the few stored memories relevant to that turn into the assistant's context, without being asked, through the same installable harness integration the checkpoint layer uses."

## Clarifications

### Session 2026-07-23

- Q: Is auto-capture in scope, and in what form? → A: Decided via `decide`
  (dogfooded, two passes under permuted option order per the order-bias
  experiment's calibration protocol). Automatic capture was rejected
  decisively in both passes (34/100 both orders). Push-only vs a
  prompted-capture label scored as a measured coin flip (80/72 in one
  order, 72/80 in the other — a symmetric flip at margin 8, inside the
  experiment's instability band), so the tie resolves by Constitution VII
  (YAGNI): **scope is push-only**; whether the surfaced-context label's
  wording mentions that `save` exists is a plan-time template choice, not
  scope.
- Q: What is the hard time budget for the prompt-time evaluation? → A:
  **500 ms** — decided via `decide` (margin 30, in the experiment's stable
  band, no confirmation pass needed): matches the validated pre-action gate
  budget for the equivalent workload, absorbs embed tail latency, and stays
  imperceptible against normal time-to-first-token. On timeout the
  evaluation abandons silently (fail-open).
- Q: What are the suppression semantics? → A: **Once per session** — a
  surfaced memory never repeats within the same session, regardless of
  duration. Decided via `decide` with the permuted confirmation pass
  (first pass margin 6 — flip band — triggered the check; the winner held
  from the opposite position, margin 14). The 30-minute time window was
  rejected in both passes (its alarm-fatigue rationale belongs to
  interrupting flags, not passive context); the relevance-spike exception
  lost on added rule complexity vs. a cheap manual `recall` covering the
  rare long-session compression case. New session ⇒ suppression resets.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Relevant prior knowledge arrives without asking (Priority: P1)

A user has knowledge stored from earlier sessions — a lesson ("the staging
deploy fails unless the cache is cleared first"), a fact ("all new code uses
only the standard library"), a skill ("the working approach for debugging the
flaky login test"). In a later session they start a turn whose task is
related to one of these. Before the assistant begins working, the relevant
stored memory is surfaced into its context — clearly labeled as a stored
memory, quoting the content with its identity and trust standing — so the
assistant applies what is already known instead of re-deriving it, repeating
a known failure, or asking the user something they already answered.

**Why this priority**: This is the feature. The design corpus's finding is
that memory whose recall is manual goes unused — the stored knowledge exists
today and is reachable only if the model thinks to call `recall`, which is
exactly the dependence on self-diagnosis the corpus says to remove. Every
other story protects this one.

**Independent Test**: Seed one trusted memory, start a turn whose prompt is
related to it, and confirm the surfaced context quotes that memory with its
identity and trust label. Delivers value standing alone: one seeded memory +
one related turn = prior knowledge applied.

**Acceptance Scenarios**:

1. **Given** a trusted stored lesson about a task and memory configured with
   the integration installed, **When** the user submits a prompt about that
   task, **Then** the assistant's context receives the lesson before it
   responds — labeled as a stored memory, quoting the content verbatim with
   the memory's identity and trust standing.
2. **Given** several stored memories of varying relevance to the prompt,
   **When** the turn starts, **Then** only the most relevant few are
   surfaced, most relevant first, capped at a small fixed number.
3. **Given** a surfaced memory the assistant judges inapplicable to the
   actual task, **When** it responds, **Then** nothing has forced it to use
   the memory — surfacing is advisory context, never an instruction.

---

### User Story 2 - Push never degrades a session (Priority: P2)

A user runs sessions in every configuration: memory off, memory on with
nothing relevant stored, a failing memory backend, long sessions returning
to the same topic repeatedly. Push must be invisible except when it has
something genuinely relevant to say: no injection when nothing clears the
relevance bar, no repeated re-surfacing of the same memory turn after turn,
no untrusted content ever pushed, no new errors or delays when the backend
is absent or failing, and no perceptible slowdown of turn start.

**Why this priority**: The corpus names the failure mode directly — surface
the wrong memories and you poison the current reasoning; push too much and
the surfaced context becomes the noise it was meant to cut through. A push
layer that costs sessions anything when it has nothing to offer will be
uninstalled, and deservedly.

**Independent Test**: Run prompt-time evaluations with an unrelated prompt
(expect nothing surfaced), with memory unconfigured (expect behavior
identical to today), with an injected backend failure (expect silent
no-op within the time budget), and twice on the same topic (expect the
second surfacing suppressed).

**Acceptance Scenarios**:

1. **Given** stored memories none of which are relevant to the prompt,
   **When** the turn starts, **Then** nothing is surfaced — no placeholder,
   no "no relevant memories" notice, nothing.
2. **Given** memory is not configured or the integration is not installed,
   **When** any turn starts, **Then** behavior is byte-identical to the
   current release.
3. **Given** the memory backend fails mid-evaluation, **When** the turn
   starts, **Then** the turn proceeds normally with nothing surfaced and no
   error shown; the failure is recorded, not raised.
4. **Given** a memory was surfaced earlier in the same session, **When**
   any later related turn starts in that session, **Then** that memory is
   not surfaced again.
5. **Given** stored memories that are untrusted (external, unverified),
   **When** any turn starts, **Then** untrusted memories are never surfaced,
   regardless of relevance.
6. **Given** any prompt, **When** the evaluation runs, **Then** it completes
   within a hard time budget or is abandoned silently — turn start is never
   perceptibly delayed.

---

### User Story 3 - Every push evaluation is auditable (Priority: P3)

An operator tuning push wants to know whether it is precise or noisy: how
often evaluations surface something, what they surfaced, and how long they
took. Every prompt-time evaluation records exactly one audit row — surfaced
memory identities or silence, latency, and the degraded/fail-open cases —
so push precision (surfacings per evaluation, and which memories recur) is
measurable from day one with no additional instrumentation.

**Why this priority**: The push-vs-noise balance is a named open question in
the design corpus; the audit trail is what turns tuning it from guesswork
into measurement. It observes the feature rather than delivering it.

**Independent Test**: Run evaluations that surface, stay silent, and
fail open; confirm each leaves exactly one audit row whose recorded outcome
matches what the session observed.

**Acceptance Scenarios**:

1. **Given** any completed prompt-time evaluation, **When** the operator
   inspects the audit records, **Then** exactly one row exists for it,
   recording what was surfaced (or silence), the evaluation latency, and
   whether it degraded.
2. **Given** a week of sessions, **When** the operator aggregates the rows,
   **Then** surfacing rate and per-memory surfacing counts are computable
   directly from the records.

---

### Edge Cases

- **A poisoned or wrong memory gets pushed.** Push amplifies the memory
  layer's poisoning blast radius: surfaced content enters the assistant's
  working context unprompted. Three structural mitigations bound it: only
  trusted memories (first-hand, or verified-before-stored) are ever pushed;
  surfaced content is labeled as stored memory — advisory, never phrased as
  instruction; and the label carries the memory's identity so a bad memory
  can be contested and deleted on sight.
- **The same topic every turn.** Once-per-session suppression: a long
  session on one topic sees each relevant memory exactly once, not every
  turn. The rare loss case — an early surfacing compressed out of a very
  long session's context — is covered by a manual `recall`, not a
  re-surfacing rule.
- **Empty or trivial prompt.** Nothing to assess relevance against —
  silence, recorded as an evaluation that surfaced nothing.
- **Very long prompt.** Relevance assessment reads a bounded portion; the
  evaluation never grows unbounded with prompt size.
- **Store grows large.** The cap and relevance bar hold regardless of store
  size; a large store affects only evaluation time, which the hard budget
  bounds.
- **Topical blind spot (known, inherited).** Relevance is meaning-based; a
  memory phrased as a rule ("never use word X") may not rank near a prompt
  that merely violates it, as the 015 dogfood measured for recall. Push
  inherits this: it surfaces what the turn is *about*, and a rule the turn
  is not about may stay unsurfaced. Named limitation, not silently ignored;
  the enforcement half is 015's job at end of turn.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: When memory is configured and the integration is installed,
  the system MUST evaluate each turn-starting prompt against stored
  memories and surface the relevant trusted ones into the assistant's
  context before the assistant begins its response.
- **FR-002**: Surfaced content MUST be clearly labeled as stored memory and
  MUST carry, per memory: the content verbatim, the memory's identity, and
  its trust standing. Surfacing is advisory — nothing may phrase it as an
  instruction or require the assistant to act on it.
- **FR-003**: Only memories above a relevance bar MAY be surfaced, capped at
  a small fixed number ordered most-relevant-first. When nothing clears the
  bar, the system MUST surface nothing at all.
- **FR-004**: Untrusted memories MUST never be surfaced, regardless of
  relevance.
- **FR-005**: A memory surfaced during a session MUST NOT be surfaced
  again within that same session, regardless of session duration; a new
  session resets suppression.
- **FR-006**: With memory unconfigured or the integration not installed,
  behavior MUST be unchanged from the current release.
- **FR-007**: The evaluation MUST fail open — any failure resolves to
  surfacing nothing, never to an error or a blocked turn — and MUST
  complete or abandon within a hard **500 ms** budget so turn start is
  never perceptibly delayed.
- **FR-008**: Every prompt-time evaluation MUST write exactly one audit
  record: the surfaced memory identities (or silence), latency, and
  whether the evaluation degraded.
- **FR-009**: The existing pull surface (`recall`, `save`, `forget`) MUST
  be unchanged: same behavior, same results, with and without push active.
- **FR-010**: The push evaluation MUST NOT add any model passes — relevance
  selection is deterministic over stored data, in line with settling
  checkable things without a probabilistic judge.

### Key Entities

- **Push evaluation**: one prompt-time assessment — the turn's prompt,
  the selected memories (possibly none), latency, and outcome.
- **Surfaced memory**: a trusted stored memory delivered into the turn's
  context — content verbatim, identity, trust standing, relevance rank.
- **Suppression set**: the per-session record of already-surfaced memory
  identities; membership lasts the session's lifetime and resets with a
  new session.
- **Audit record**: the one-row-per-evaluation trail — surfaced identities
  or silence, latency, degradation flag.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With one seeded trusted memory and a related prompt, the turn
  starts with that memory surfaced — content, identity, and trust visible —
  demonstrable live end-to-end (seed → new turn → surfaced → applied) in a
  single session.
- **SC-002**: With the same store and an unrelated prompt, nothing is
  surfaced; across the feature's unrelated-prompt test scenarios, zero
  false surfacings.
- **SC-003**: With memory unconfigured or the integration absent, all
  existing behavior is unchanged: the full current test suite passes
  identically before and after the feature.
- **SC-004**: 100% of evaluations complete or abandon within the hard
  500 ms budget; an injected backend failure still produces a normal turn
  and an audit row.
- **SC-005**: Every evaluation — surfaced, silent, or degraded — leaves
  exactly one audit record from which surfacing rate and per-memory counts
  are computable without additional instrumentation.
- **SC-006**: In a session with repeated related turns, a given memory is
  surfaced at most once for the session's entire duration.

## Assumptions

- **Push only; capture stays manual.** This feature is the read-path push.
  Auto-capture (turning session outcomes into candidate memories) is
  deliberately out of scope: the corpus warns that storing everything is as
  bad as storing nothing, so capture belongs with the consolidation levers
  (merge, decay, eviction) as their own future feature. This narrows the
  catalog item as originally sketched ("push + auto-capture") — named here,
  not silently.
- **Delivery rides the existing installable integration.** Push is
  harness-triggered at turn start, the same opt-in sensor plane the
  checkpoint layer uses: installing the integration is the explicit consent
  to inject context into sessions. No new consent surface, no new gate
  beyond the existing memory-capability gate.
- **Every turn start, not session start.** Evaluation runs per turn (each
  prompt is a new relevance context), with once-per-session suppression
  preventing repetition. A session-start-only variant would miss topic
  shifts mid-session.
- **Deterministic relevance.** Selection uses the same stored-data ranking
  the pull path uses (relevance, recency, trust) — no model judgment in the
  loop, honoring the deterministic-over-probabilistic principle and keeping
  the evaluation inside a prompt-time latency budget.
- **Trust model is the existing one.** First-hand and verified memories are
  pushable; untrusted are not. No new trust states.
