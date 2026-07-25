# Feature Specification: Research Confidence Aggregation

**Feature Branch**: `021-research-confidence-aggregation`

**Created**: 2026-07-25

**Status**: Draft

**Input**: User description: "Research's overall confidence collapses to exactly 0 on correct, well-supported answers. Fix the confidence aggregation."

## Context

The `research` tool returns one top-level `confidence` value in 0..=1 alongside its
answer, per-claim findings, gaps, and sources. Callers are other language models
deciding how much to trust the answer.

On two separate live runs the tool reported **confidence exactly 0 for a factually
correct answer** whose every claim had survived refute-biased verification at a
per-claim confidence of roughly 0.78. The reported number was not conservative; it
was wrong in a way that destroys the signal, because a caller that learns the number
reads 0 on correct answers stops reading it at all.

The cause is the aggregation, not the verification. Today:

```text
overall_confidence = mean(per_claim_confidences) * coverage
coverage           = settled / total_sub_questions
settled            = sub_questions.len() - gaps.len()      (saturating)
```

Three distinct defects sit in those three lines.

**D1 — the subtraction measures nothing.** Sub-questions are the falsifiable
questions the scope phase produced; gaps are free-form short phrases the synthesis
pass is prompted to write. No entry in one corresponds to an entry in the other, so
subtracting the lengths is not a coverage measurement. The synthesis pass can emit
seven gaps that all concern a single sub-question and zero the coverage of a run
that settled the other six. Because the gap cap exceeds the sub-question cap, a zero
result is reachable by construction.

**D2 — coverage annihilates rather than attenuates.** It is a bare multiplier that
reaches exactly 0. A confidence of exactly 0 asserts certainty of falsehood about an
answer that was correct, and makes the top-level number return the same value for a
well-supported answer and a demonstrably wrong one.

**D3 — the penalty is computed from a list the caller never sees.** `settled` is
derived from the gap list *before* it is truncated to the published maximum. A run
can be penalised for gaps that never appear in its own output.

### Decided design

Two design questions were settled before this spec, each by a `decide` pass whose
recommendation was then put through a `verify` confirmation pass. Both are recorded
here as decided, with the reasoning, so they are not re-litigated in `/speckit-clarify`.

**Decision 1 — what coverage measures.** Gaps MUST be emitted keyed to a
sub-question, and coverage derived from which sub-questions no gap claims. Scored
85, margin 30.

*Rejected alternative:* deriving the claim-to-sub-question association server-side by
deterministic text matching. A `verify` pass refuted this 3/3. Lexical overlap is
neither necessary nor sufficient for answerhood — "1.97.1 was released on July 16"
settles "which version is current?" while sharing no content words, whereas "the
changelog does not state which version is current" shares nearly all of them while
settling nothing. Determinism delivers reproducibility, not validity: whether one
natural-language string answers another is a semantic entailment relation, not a
syntactic property, so such a rule is reproducibly wrong. The error is biased rather
than noisy, systematically under-crediting the terse factual answers (versions,
dates, identifiers) that the scope phase is designed to elicit and over-crediting
verbose restatements; and because claims are extracted from pages fetched for this
same question, shared domain vocabulary drives overlap toward saturation. **Any
variant that infers the association from surface text is out of scope**, including
counting grounded citations, since citations resolve to sources rather than to
sub-questions.

Keying gaps to a sub-question confines model self-report to the artifact the server
already depends on for this number today, and keeps the aggregate server-computed.

**Decision 2 — the shape of the reported number.** A separate `coverage` field MUST
be added to the output, and `confidence` MUST carry the findings' mean support
alone.

*Rejected alternative:* keeping one multiplied number but blending the multiplier so
it attenuates without annihilating. A `verify` pass refuted the premise 3/3. The
blend also changes the field's stated definition — it deletes the invariant that
zero coverage forces zero confidence. It is the *less* detectable change, silently
rescaling an unchanged field name, where a new key is something a value-reading
caller can notice. Information runs the other way from the argument for it: with a
separate field the previous value stays exactly recoverable as
`confidence * coverage`, whereas under the blend coverage is unpublished and the
previous value cannot be reconstructed at all. And the blend introduces a free
parameter derived from nothing, so two deployments emit incomparable values under
the same contract version.

## Clarifications

### Session 2026-07-25

- Q: Should refuted claims affect the reported confidence, now that the coverage multiplier is gone? → A: No — confidence stays the mean over surviving findings, and the refutation rate is published as its own field alongside confidence and coverage. Decided by a `decide` pass, 92 with margin 30. Folding refuted claims into confidence was rejected at 28 as reintroducing exactly the indiscriminate fold that removing the coverage multiplier was meant to end. Leaving the rate merely derivable from the existing claim counts was rejected at 62: callers read fields rather than computing rates, so a theoretically-available signal is an unavailable one.
- Q: Does the sub-question a gap concerns reach the caller, and in what shape? → A: Publish both — gaps stay plain strings, and a separate published list reports each scoped sub-question with whether it was settled. Decided by a `decide` pass, 86 with margin 31. Turning gaps into objects was rejected as a breaking type change for callers that read them as text, and because gaps raised by the grounding gate have no sub-question to name and would have to be given a false one. Keeping the association wholly internal was rejected at 28: it fails the requirement that coverage be checkable from the output alone, and repeats the failure mode the earlier field-shape decision ruled out — a changed meaning with nothing visible in the output to signal it.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A correct answer reports a confidence that reflects its support (Priority: P1)

A calling model asks a research question whose authoritative answer lives on one
canonical page. Every claim extracted is correct and survives refute-biased
verification. The caller receives a confidence that reflects how well-supported
those claims are, and can act on it.

**Why this priority**: This is the defect. Until the reported number distinguishes a
correct answer from a wrong one, every other property of the tool is invisible to
the caller, who has been trained to ignore the field.

**Independent Test**: Run a research question whose claims all verify as supported
and whose scope produced sub-questions, and confirm the reported confidence is
greater than zero and tracks the per-claim support.

**Acceptance Scenarios**:

1. **Given** a run whose claims are all supported at high per-claim confidence,
   **When** the synthesis reports gaps against some sub-questions,
   **Then** the reported confidence is greater than zero and reflects the claims'
   support.
2. **Given** a run in which no claim was supported, **When** the answer is
   assembled, **Then** confidence is zero — the value remains available for the case
   it actually describes.
3. **Given** two runs with identical per-claim support but different numbers of
   unsettled sub-questions, **When** both complete, **Then** their reported
   confidence values are equal, and they are distinguished by coverage instead.
4. **Given** a run in which verification refuted most of the claims it checked,
   **When** the answer is returned, **Then** confidence reports the support of the
   claims the answer does assert, and the refutation rate reports how much fell
   away.

---

### User Story 2 - A caller can see breadth of resolution separately from support (Priority: P2)

A calling model needs to know not just how well-supported the stated claims are, but
how much of what it asked was addressed. Those are different questions and a single
number cannot answer both.

**Why this priority**: Separating the two is what makes P1 safe. Without a published
coverage figure, fixing confidence would discard the breadth signal rather than
relocate it.

**Independent Test**: Run a question whose scope produces sub-questions of which only
some are settled, and confirm the output reports the settled proportion as its own
value, independent of the claims' support.

**Acceptance Scenarios**:

1. **Given** a run whose scope produced sub-questions and whose synthesis reported
   gaps against some of them, **When** the answer is returned, **Then** coverage
   reports the proportion of sub-questions no gap claims.
2. **Given** a run in which every sub-question is claimed by at least one gap,
   **When** the answer is returned, **Then** coverage is zero while confidence
   continues to report the claims' support.
3. **Given** a run whose scope produced no sub-questions, **When** the answer is
   returned, **Then** coverage reports a defined value rather than being absent or
   undefined.

---

### User Story 3 - The penalty matches what the caller can see (Priority: P3)

A caller reading the output can reconcile the reported coverage against the gaps
listed in that same output.

**Why this priority**: Smallest of the three and independent of the other two, but it
is the difference between a figure a caller can audit and one it must take on faith.

**Independent Test**: Force a run to produce more gaps than the published maximum and
confirm the reported coverage is consistent with the gaps actually returned.

**Acceptance Scenarios**:

1. **Given** a synthesis that reports more gaps than the output permits, **When** the
   answer is returned, **Then** the coverage figure is derived from the gaps the
   caller receives, not from the discarded ones.

---

### Edge Cases

- A run where the scope phase produced no sub-questions: coverage must be defined
  rather than a division by zero.
- A run where no claim survived verification: confidence zero is correct and must
  remain reachable, since it is the case the value genuinely describes.
- A run where every verified claim was refuted: the refutation rate is total while
  confidence is zero for want of surviving findings. Both values must be defined.
- A run that verified no claims at all: the refutation rate must be defined rather
  than a division by zero.
- A synthesis that reports a gap keyed to a sub-question that does not exist, or
  keyed out of range: the association must be discarded rather than corrupting the
  count or failing the run.
- Several gaps keyed to the same sub-question: that sub-question counts as unsettled
  once, never multiple times — this is the specific arithmetic that produced the
  observed collapse.
- A run stopped early by its budget or deadline: coverage must remain meaningful, as
  an early stop is exactly when breadth of resolution matters most to the caller.
- A gap the synthesis cannot attribute to any single sub-question (for example, one
  raised by the grounding gate rather than by an unanswered question): it must be
  reportable without silently suppressing coverage.
- More gaps produced than the output publishes: a sub-question can be reported
  unsettled while the gap text explaining why was dropped by the cap. The coverage
  figure stays consistent with the published statuses, which is what SC-003 requires;
  the explanatory text is best-effort.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST report a top-level confidence value that reflects the
  support established for the claims the answer makes, and MUST NOT reduce that value
  on account of unsettled sub-questions.
- **FR-002**: The system MUST report the proportion of scoped sub-questions that were
  settled as its own published value, distinct from confidence.
- **FR-003**: The system MUST determine which sub-questions are unsettled from gaps
  that are each keyed to a specific sub-question, and MUST NOT infer the association
  between a claim and a sub-question from the text of either.
- **FR-004**: The system MUST count a sub-question as unsettled at most once,
  regardless of how many gaps are keyed to it.
- **FR-005**: The system MUST publish, for each sub-question the run scoped, the
  sub-question and whether it was settled, so that a caller can reconcile the reported
  coverage against the output rather than taking it on trust.
- **FR-005a**: The system MUST NOT change the shape of the existing gaps list; gaps
  remain plain text entries.
- **FR-006**: The system MUST discard a gap whose key does not identify a
  sub-question of that run, without failing the invocation and without letting the
  discarded gap affect coverage.
- **FR-007**: The system MUST report a defined coverage value when a run scoped no
  sub-questions.
- **FR-008**: The system MUST continue to report confidence zero when no claim was
  supported.
- **FR-009**: The system MUST allow a gap that bears on no single sub-question — for
  example one raised by the grounding gate — to be reported to the caller without
  affecting coverage and without being attributed to a sub-question it does not
  concern.
- **FR-009a**: The system MUST exclude refuted claims from the confidence figure,
  since the answer does not assert them, and MUST publish the proportion of verified
  claims that were refuted as its own value so that the two signals stay separable.
- **FR-010**: The system MUST compute every published value named here from run data
  rather than accepting any of them as a figure supplied by the model.
- **FR-011**: The change in meaning of the existing confidence value MUST be recorded
  in the tool's published contract, since the value it takes for a given run changes.
- **FR-012**: The design corpus MUST be amended in this same change to describe both
  published values and how each is derived.

### Key Entities

- **Sub-question**: A falsifiable question the scope phase determined a good answer
  must settle. Positional within a run; the unit coverage is measured over.
- **Gap**: Something the synthesis pass reports as unresolved. Internally carries an
  association to the sub-question it concerns, or none where it concerns no single
  one; the caller receives it as plain text.
- **Sub-question status**: The published record of one scoped sub-question and
  whether the run settled it. The caller-facing basis for the coverage figure, and
  the first time a run's decomposition of the question is visible to the caller.
- **Confidence**: The published measure of how well-supported the claims the answer
  makes are.
- **Coverage**: The published measure of how much of the scoped question was settled.
- **Refutation rate**: The published proportion of verified claims that verification
  refuted. Distinguishes a run whose evidence largely held up from one whose evidence
  largely fell apart — two cases confidence alone cannot tell apart.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A run whose claims are all supported and whose scope produced
  sub-questions never reports confidence zero. Reproducing either of the two observed
  runs yields a non-zero confidence consistent with the roughly 0.78 per-claim
  support.
- **SC-002**: Two runs with identical per-claim support report identical confidence
  regardless of how many sub-questions each settled, and are distinguished from one
  another by coverage alone.
- **SC-003**: For every returned answer, the reported coverage equals the proportion
  of published sub-question statuses marked settled — checkable by a reader from the
  output alone, with no appeal to internal state.
- **SC-004**: Confidence zero occurs only when no claim was supported.
- **SC-005**: The previous combined figure remains derivable by a caller from the two
  published values, so no information available before this change is lost.
- **SC-006**: Two runs with identical surviving-claim support but different
  proportions of refuted claims report the same confidence and different refutation
  rates, so a caller can tell them apart without computing anything.

## Assumptions

- The verification pipeline, the support labelling rule (including the
  two-independent-sources bar for a confirmed claim), and per-claim confidence are
  correct as they stand and are out of scope. The observed defect is in aggregation
  alone; the per-claim confidences in both observed runs were sound.
- Reporting gaps against sub-questions is within what the synthesis pass can be asked
  for reliably, because the tool's constrained-output contract guarantees the shape
  of what it returns and prevents an omitted or malformed field.
- The tool's callers are language models that read the returned values without
  consulting a schema diff, so a changed field meaning must be visible in the output
  itself rather than only in documentation.
- Coverage of a run that scoped no sub-questions is treated as full, matching the
  existing behaviour for that case; there is nothing unsettled to report.
- The number of sub-questions per run stays small, so coverage remains a coarse
  proportion and is not expected to be finely graded.
- Each published sub-question status carries the sub-question itself, not only a
  position. Sub-questions are not part of the output today, so a positional reference
  would point at nothing the caller can see and would leave coverage as uncheckable
  as it is now.
