# Phase 0 Research: Preference Enforcement at the Checkpoint

All Technical Context unknowns resolved. Each decision below is grounded in the
shipped checkpoint/memory code (read directly during planning) and the design
corpus (`PREFERENCE_ELICITATION.md`, `WATCHDOG_LAYER.md` 2026-06-12 amendment,
Constitution 1.0.0).

## D1 — One hop, two judgments (how FR-010 is honored)

**Decision**: Extend the existing end-of-turn review hop to judge both
self-contradiction (existing) and preference violation (new) in a single
constrained pass. The hop's output schema gains three flat fields
(`violates`, `violated_preference`, `violation_basis`); the prompt gains a
numbered preference-candidate section alongside the existing contradiction
pairs.

**Rationale**: Spec FR-010 forbids a second model pass. The existing hop
already receives turn evidence and is decline-biased; a second section is the
same shape of work. Cost and latency stay at today's ceiling (one embed + at
most one hop).

**Alternatives considered**: (a) A second dedicated violation hop — rejected:
violates FR-010, doubles worst-case cost/latency, and creates two verdicts to
reconcile. (b) Deterministic violation checking (regex/substring against
preference text) — rejected: preferences are natural language ("prefer
readability over micro-optimization"); only wording bans would be checkable,
and Constitution V reserves LLM judgment for exactly this non-mechanical case.
The deterministic parts (mining, identity, provenance, wording) stay pure.

## D2 — Candidate population: reuse `gate::is_constraint`

**Decision**: Preference candidates are recalled memories with
`is_constraint(memory)` true — kind `Lesson` or `Fact` AND trusted
(`FirstHand` or `Verified`) — at or above the existing `REVIEW_RECALL_FLOOR`
(0.45 cosine). No stored marker, no new kind, no save-surface change.

**Rationale**: Clarification Q1 (spec, Session 2026-07-21) chose "no marker —
the judgment pass decides". `is_constraint` is precisely the population the
gate already enforces at action time; reusing it keeps one definition of
"enforceable memory" across both boundaries. Untrusted memories are excluded
structurally (spec FR-005), not by judgment.

**Alternatives considered**: caller-set preference marker on `save` — rejected
by clarification and by 003's precedent (trust is "derived, never caller-set";
a caller-set enforceability bit has the same self-report problem).
Server-derived marker at save time — rejected: adds a model pass to `save` and
a stored classification that can go stale; the per-turn judgment sees current
context instead.

## D3 — Judged evidence: final message + bounded window summary

**Decision**: The hop's preference section presents (a) each candidate
preference's content verbatim, (b) the turn's final message (truncated to a
fixed cap so the prompt stays bounded), and (c) the existing deterministic
tool-activity summary of the bounded transcript window (`summarize_calls`) —
the same evidence style the contradiction section already uses.

**Rationale**: Clarification Q2 chose "final message + bounded trajectory
tail": wording preferences are violated in the final message; process
preferences ("run the gate before claiming done") are violated by what the
window shows the turn *did* — e.g. the final message claims completion while
the summary shows zero test-tool activity. The window summary is already pure,
bounded, and computed for the contradiction path, so no new reads (spec edge
case: "no unbounded input").

**Alternatives considered**: full transcript text in the prompt — rejected:
unbounded, duplicates what `WINDOW_*` bounds exist to prevent. Final message
only — rejected by clarification Q2 (drops the process-preference class).

## D4 — Cooldown identity: the memory id

**Decision**: A violation signal's cooldown identity is the stored memory's
id: `signal_key = "preference_violation:" + fnv1a64(memory.id)`.

**Rationale**: FR-010's cooldown must survive wording drift between turns. The
contradiction path had to map the model's echo back to mined statement pairs
(review finding 7) because statements have no stable id; preferences DO have
one — the memory id — which is stable across turns, sessions, and re-phrasings
of the flag. One preference can therefore flag at most once per cooldown
window regardless of how each violation reads.

**Alternatives considered**: identity from the echoed preference text —
rejected: re-introduces the wording-drift hole the memory id closes for free.

## D5 — Provenance is server-assembled, never trusted from the echo

**Decision**: The hop returns `violated_preference` as a verbatim echo used
only for *matching*: the server maps it back to the mined candidate by best
lexical overlap (the established pattern from `review_once`'s identity
map-back) and takes the memory id, trust standing, and quoted content **from
the mined candidate**, not from the model output. The flag message is a fixed
template parameterized only by that server-held evidence plus the hop's
one-sentence basis.

**Rationale**: Spec FR-002 requires the flag to name the preference and its
provenance accurately; a hallucinated echo must not be able to mis-attribute.
Server-assembled verdicts are the layer's standing rule (006 FR-005) and the
spec's "violations must name the specific preference … so the model can fix or
push back" depends on that accuracy.

**Alternatives considered**: numeric candidate index in the hop output —
workable, but the API grammar drops numeric range constraints so out-of-range
handling is needed anyway, and overlap map-back is the pattern the module
already uses; two mapping mechanisms in one file is worse than one.

## D6 — When both judgments fire: one message, both signals

**Decision**: If the hop confirms a contradiction AND a violation in the same
turn, deliver one flag whose message concatenates the two fixed templates
(contradiction first — it is the turn's own incoherence; the violation
follows), with both signals in the result's `signals` array and both keys fed
to the cooldown independently.

**Rationale**: The turn boundary delivers at most one verdict (forced
continuation happens once); the batch boundary already concatenates multiple
signal evidences into one message, so this is the established multi-signal
delivery shape. Independent cooldown keys keep suppression per-signal
(a suppressed contradiction must not mute a fresh violation, and vice versa —
the existing `unsuppressed` filter already handles partial suppression).

## D7 — `signals_evaluated` stays configuration-honest

**Decision**: `run_turn` lists `preference_violation` in `signals_evaluated`
only when the memory capability is present (embedder configured). With memory
absent, the evaluated list stays exactly `[self_contradiction]` as today.

**Rationale**: Spec FR-006 (memory off ⇒ behavior unchanged) extends to the
audit surface: records must not claim a signal was evaluated when the
capability that feeds it is absent. This also makes spec FR-008/SC-005
mechanically checkable — "enforcement was evaluated" ≡ `preference_violation ∈
signals_evaluated`. Precedent: the gate boundary already records
`memory_conflict` as evaluated-but-inactive when memory is off; the turn
boundary distinguishes instead because, unlike the gate (whose only signal is
memory-paired), the turn boundary has a memory-independent signal and a
memory-paired one — listing both unconditionally would make the two
indistinguishable in the audit.

## D8 — Module split: new `src/checkpoint/preference.rs`

**Decision**: Preference-candidate mining, the identity function, and
violation-flag assembly land in a new pure module `preference.rs`; `review.rs`
keeps the hop (prompt assembly, schema, map-backs); `run.rs` wires them.

**Rationale**: `review.rs` is at 516 lines — already past the ≤500 target.
Principle VII treats crossing it as the signal to find the split seam; the
pure/orchestration boundary is that seam, and it mirrors how `gate.rs` and
`screen.rs` are already factored (pure deciders, wired by `run.rs`).

## D9 — Empty-final-message early exit is kept (named narrow edge)

**Decision**: `run_turn`'s existing early-silence on an empty final message is
unchanged, so a hypothetical turn with tool activity but no final text is not
enforcement-evaluated.

**Rationale**: The harness's stop hook supplies the final message; an empty
one means no committed turn output to judge wording against, and the existing
contradiction path already exits there. Changing it would alter memory-off
behavior (against FR-006). Named here so the narrowing is explicit, not
silent (Constitution I/VII).

## D10 — Corpus amendment in the same change

**Decision**: `docs/design/PREFERENCE_ELICITATION.md` gains a dated amendment:
the enforce half of the loop now exists at the end-of-turn checkpoint; the
"block vs flag-and-revise" open question is resolved to flag-and-revise, with
hold-tier escalation deferred until audit data (SC-005's precision measure)
justifies it.

**Rationale**: Constitution I requires corpus amendments in the same change
that resolves a corpus question — the doc currently reads as
capture-without-enforcement being the status quo, which this feature makes
false.
