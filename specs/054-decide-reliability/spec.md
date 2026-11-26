# Feature Specification: Decide Stops Claiming Calibrated Confidence

**Feature Branch**: `054-decide-reliability`

**Created**: 2026-11-17

**Status**: Draft (third version; the first two were rewritten after independent review)

**Input**: User description: "decide reliability: make the decide tool report truthfully how reliable its scores are."

## Why this exists

`decide` scores each option in one model pass, ranks them, and reports a
confidence computed from the gap between the top two scores. Its description
tells callers this is "a confidence calibrated to how close the call is", and
the design document says gaps of 17 points or more were "perfectly stable".
Callers read these numbers before nearly every choice.

A measurement shows the stability statement is false: winners reversed
between identical calls at gaps of 25, 31, and 28 to 38 points. The
measurement is too small to say how much a gap does tell you. So this feature
changes words, not numbers: the tool and its documents stop claiming a
reliability that has not been shown, and say in plain terms how to read a
result.

**This feature does not change what `decide` computes or returns.** It does
not make `decide` more reliable.

### The measurement

10 recorded `decide` inputs re-run on 2026-11-17 with the server's default
model setting (`claude-opus-4-8`; the model name comes from configuration and
is not recorded in the result files). 281 calls completed; 179 more failed
for account billing reasons and are excluded. The table uses 221 of the 281;
60 single reorder and removal runs from the first stage are not used.

"Identical runs" means the same input sent again unchanged. "Less common
winner" means a top option other than the one most often on top for that
input, judged within this sample (for three inputs that is a 6-to-4 or 3-to-2
split, so the label is itself uncertain).

| Finding | Number |
|---|---|
| Inputs whose top option changed between identical runs | 5 of 10 |
| Difference between two identical runs of one option's score | 4 points median, 13 at the 90th percentile, 54 at worst (860 pairs) |
| One input's top-two gap over five identical runs | 33, 38, 28, 6, 16, with the winner changing |
| Identical runs with a gap of 15 or less that named the less common winner | 9 of 45 |
| Identical runs with a gap over 15 that named the less common winner | 5 of 30 |
| Pairs of identical runs naming different winners, by the first run's gap: 8 or less / 9 to 16 / 17 to 38 / over 38 | 86 of 183 (47%) / 54 of 232 (23%) / 34 of 107 (32%) / 0 of 28 (three inputs) |
| On one input, identical calls gave | one option by 38 points, and the other option by 33 |
| One context sentence removed: winner reversed in 5 of 5 runs | 3 of the 13 removals that have 5 runs (2 of 5 inputs); two further removals have only 4 and 2 runs |
| Options reordered: one order gave one winner 5 of 5 times, another order the other winner 5 of 5 times | 1 of 5 inputs (13 distinct reorderings; the two-option input has only one) |

**Limits of this sample.**

- 10 inputs, one model. Inputs were chosen in bands by their recorded gap, so
  pooled counts describe those bands, not the real mix of calls.
- Five inputs have 10 identical runs and five have 5. One input showed one
  winner in its first 5 runs and three different winners after 10; 5 runs is
  too few to call an input stable. The four inputs with the largest recorded
  gaps are all among those with only 5 runs.
- The wording and ordering rows rest on the first five inputs in the plan
  (recorded gaps 4, 4, 8, 8, 12), because billing failures cut off the rest.
- Runs cluster within inputs: 10 of the 14 less-common-winner runs come from
  three inputs, and every score difference of 40 or more comes from one
  option of one input.

**What it establishes:** gaps of 17 or more are not reliably stable; results
can change with option order and with one sentence of context.

**What it does not establish:** how reliability varies with the gap. The pair
counts lean toward larger gaps being steadier (47% at 8 or less, none above
38) but they are clustered in 10 inputs and are not proof either way. Nor any
error rate, any threshold, or anything about other models.

**Two further cautions about the runs themselves.** The model and effort are
not recorded in the result files, and the probe used the server's default
model directly, bypassing per-site routing; if the deployed server routes
`decide` elsewhere, this is not a measurement of the deployed tool. Four of
the 75 identical runs used about twice the input tokens of their siblings,
which matches the tool's retry after a rejected answer; this is inferred from
token counts, which is why FR-010 keeps them.

## Scope

**In:** the text that tells callers and maintainers what `decide`'s
confidence means, and the evidence for changing it.

**Out:** the confidence formula, the result's fields, the prompt, the number
of passes. All unchanged.

Two kinds of document are treated differently, on purpose:

- **Live text** describes the tool as it is now and is corrected in place.
- **Historical records** (the earlier `decide` specification and its
  research, plan, data model, tasks and quickstart; the earlier experiment's
  analysis; past change-log entries; the design document's earlier amendment)
  are not rewritten. FR-008 and FR-009 name the few that get one dated line
  pointing here; the design document gets a new dated amendment.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The description tells callers how to read a result (Priority: P1)

A caller reading the tool catalog sees a `decide` description that no longer
says the confidence is calibrated, says what the confidence is, states what
has been observed about repeat calls, a removed context sentence and option
order, and asks the caller
to report the lead in points with the recommendation. It sets no threshold
and asks for no extra calls.

**Why this priority**: Callers are models that choose and interpret tools
from this text, and it is the only part every caller reads.

**Independent Test**: The served description contains the three required
sentences below word for word, does not contain the forbidden word stem, and
equals the registered description.

**Acceptance Scenarios**:

1. **Given** the tool catalog, **When** the `decide` description is read, **Then** it does not contain the forbidden word stem in FR-001.
2. **Given** the tool catalog, **When** the `decide` description is read, **Then** it contains each required sentence in FR-002 exactly.
3. **Given** the result schema, **When** the confidence field's description is read, **Then** it is the text in FR-003 exactly.
4. **Given** the served description and the registered description, **When** compared by test, **Then** they are equal, and the registered one passes scenarios 1 and 2.

---

### User Story 2 - Nothing about the tool's behaviour changes (Priority: P1)

A caller sends the same call as before and receives the same result shape,
the same confidence for the same scores, and the server makes the same model
request.

**Why this priority**: This is a words-only change. A behaviour
change slipping in under it would be outside this feature's scope.

**Independent Test**: No test that asserts `decide`'s computed values is
edited, and all of them pass unchanged.

**Acceptance Scenarios**:

1. **Given** the existing tests that pin `decide`'s ranking, confidence values and request, **When** the suite runs after this feature, **Then** they pass with their assertions unedited.
2. **Given** the result contract file, **When** compared before and after, **Then** its property names, types and constraints are unchanged; only description text differs.

---

### User Story 3 - The record is corrected and the evidence is in the repository (Priority: P2)

A maintainer finds, in the design document, a dated amendment that withdraws
what the measurement contradicts; finds the live documents consistent with
it; and can recompute the measurement table from committed files.

**Why this priority**: The design document is the project's source of truth.
A withdrawn claim left standing invites the next feature to build on it, as
two memory features already did.

**Independent Test**: Each item in FR-006 to FR-010 is done as written, and
running the committed script prints the table.

**Acceptance Scenarios**:

1. **Given** the design document, **When** read after this feature, **Then** it has a new dated amendment containing every statement FR-006 lists, and its earlier text is unedited.
2. **Given** the evidence folder, **When** its script is run as its note describes, **Then** it prints every number in the measurement table.
3. **Given** the live-text locations in FR-007, **When** searched for the forbidden word stem, **Then** it is not found.

### Edge Cases

- **A historical document still says the confidence is calibrated**: expected. The search in FR-007 covers live text only; FR-008 lists the historical files that get a note instead.
- **Other tools' text uses the same word** (the verify prompt is called the "calibrated profile"): out of scope. The search is limited to the `decide` locations FR-007 names.
- **"Small" is not defined in the description**: deliberately. The sample supports no threshold, so the caller reports the lead in points and the reader judges.

## Requirements *(mandatory)*

### Functional Requirements

**The description (User Story 1)**

- **FR-001**: The `decide` tool description, in both places it is written (the registered constant and the served literal), MUST NOT contain the word stem `calibrat` in any letter case.
- **FR-002**: The `decide` tool description MUST contain each of these sentences exactly:
  1. `Confidence restates this run's top-two score gap (0.5 + gap / 200).`
  2. `Scores come from one pass: an identical call can give different scores and a different winner (seen on one input with leads of 38 points one way and 33 the other), and the winner has changed when one sentence of context was removed and when the options were reordered.`
  3. `Report the lead in points alongside the recommendation, and say when it is small.`
- **FR-003**: The confidence field's description MUST read exactly: `This run's top-two score gap rescaled to [0.5, 1.0] (0.5 + gap / 200). An identical call can score differently.`
- **FR-004**: Three tests are required. One holds the served description equal to the registered description. One checks the registered description against FR-001 and FR-002 by exact string; equality alone is not sufficient, since both copies could keep the old claim. One holds the confidence field's description in the derived result schema equal to FR-003 and equal to the same field's description in the contract file, because the existing contract test compares names and constraints only.

**No behaviour change (User Story 2)**

- **FR-005**: The confidence formula, ranking, result fields, model-facing prompt, model output shape and model request MUST NOT change. No assertion in an existing test of `decide`'s computed values may be edited. Existing tests pin the confidence values and ranking but not the prompt text or the model's output schema, so one new test MUST hold equal to their text before this feature: the prompt as built for a fixed input, with and without the retry wording, and the schema the request carries. Beyond that, "no behaviour change" is established by review of the diff, and the pull request says so.

**The record (User Story 3)**

- **FR-006**: The design document MUST gain a new dated amendment, leaving earlier text unedited, that states all of the following:
  1. "Margin ≥ 17 was perfectly stable" is withdrawn as a general claim. It stands only as the observed result of the earlier study (70 authored fixtures, 250 calls). The two studies used different inputs: authored fixtures there, recorded real calls here.
  2. "The score margin encodes all of it" is withdrawn for the same reason.
  3. The statement that margin-derived confidence "understates" instability at low margins is replaced by: the confidence is the gap rescaled; winners reversed here at margins up to 38; this sample cannot say how reliability varies with the margin.
  4. "Order permutation is not empirically required" is marked as questioned by one of five inputs here, and not overturned.
  5. The catalog's entry that Decide corrects "miscalibration", in this document's primitives table and in the same table of `NEXT_REASONING_SERVER.md`, is not met by a single pass: `decide` makes the scoring explicit, and does not calibrate.
  6. The earlier amendment's candidate, a too-close-to-call label keyed on the gap, is not built, because this sample could not show the gap separates reliable results from unreliable ones.
  7. The model setting used and that it is not recorded in the result files, both sample sizes, and the limits of this sample.
- **FR-007**: These live-text locations MUST be corrected in place and afterwards not contain the word stem `calibrat` in any letter case, with two named exceptions: the new test that searches for the stem necessarily contains it, and the `check` tool's description in the server file says "no calibration" about a different tool and is left alone. The locations: the `decide` module's source file (its opening documentation, the registered description, the confidence field's documentation, and its comments including those in its test module); the served description and the comment above it; the earlier `decide` contract file's confidence description (property names and constraints untouched); the earlier `decide` contract note's quoted description; the project guide's `decide` statements; the opening comment of the offline `decide` acceptance example; and the comment and name of the `decide` integration test that carries the word. Renaming that test changes no assertion. The README's `decide` row and the contract file's opening comment already say "derived from the score margin", which is true, and are left alone.
- **FR-008**: Two historical records MUST each gain one dated line that points to this specification and makes no claim of its own, and MUST NOT otherwise be edited: the earlier `decide` specification and the earlier order-bias experiment's analysis. The earlier specification's other documents are not touched. Past change-log entries are left as written; this feature adds one entry under Unreleased, in the Changed section, saying the description changed and no behaviour did. The earlier contract note and contract file are treated as live, unlike the rest of that specification, because the server's tests and callers read them.
- **FR-009**: The four places that cite a "stable band" for a choice made with `decide` (the memory push module's comment; the push-memory specification and its research; the memory-consolidation specification) MUST be given a recorded disposition: the choices stand as accepted decisions; the words "stable band" are removed from the live code comment; the three documents gain the same dated line as FR-008. Whether to revisit those choices, and the four clarifications in the same two specifications that were settled by a `decide` confirmation pass, is a named open question, not decided here.
- **FR-010**: An evidence folder MUST be committed at `claudedocs/experiments/decide-stability/` holding: the per-call results for all 460 attempted calls (case id, variant, option ordering, scores, methodology, token counts; failed calls reduced to their id, variant, option ordering and the word "billing", with provider text and request identifiers removed; the redaction MUST stop with an error if any failed call's error text is not the provider's billing error, so that no other kind of failure is relabelled); a redacted plan (per case: id, number of options, number of context sentences, recorded scores, recorded gap; no decision, option or context text); one script that prints the measurement table from those two files alone; the probe program's source as a non-buildable record with its header corrected; and a note stating what is withheld and why, the dates on the files, that the model and effort are not recorded and routing was bypassed, and why the 60 first-stage single reorder and removal runs are not used in the table (one run per variant, where the table's reorder and removal rows need five).
- **FR-011**: The temporary probe program MUST NOT remain in the examples folder.

### Key Entities

- **Forbidden word stem / required sentences**: the exact strings in FR-001 and FR-002.
- **Live text**: the locations in FR-007.
- **Historical record**: the files in FR-008 and FR-009.
- **Evidence folder**: the files in FR-010.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Across the live-text locations in FR-007 the forbidden word stem occurs only in the two named exceptions (the new test's search string, and the `check` tool's description).
- **SC-002**: The three required sentences each occur exactly once in the registered description and once in the served description.
- **SC-003**: 0 assertions of `decide`'s computed values are edited, and the full suite passes.
- **SC-004**: Running the committed script reproduces every number in the measurement table from the two committed data files. The script's printed output is committed beside it, so the table in this specification can be compared with it line by line.
- **SC-005**: Each of the seven statements in FR-006 can be pointed to in the new amendment.
- **SC-006**: Each file named in FR-008 and FR-009 carries the dated line.

## Assumptions

- Delivered as one pull request.
- Nothing sent to `decide`'s own model changes and no computed value changes, so building and testing need no live model call. The project keeps a note that any model-facing prompt or schema change must pass the live example before a pull request. The description is read by calling models, and no live `decide` example exists (the existing one is offline, against a substituted model). So whether one live `decide` call is made before the pull request is decided on its own, with its price stated beforehand.
- The redaction in FR-010 is done by a script whose output can be inspected before commit, because the unredacted plan contains text from other projects. Case ids are checked to be the plain labels c00 to c09 and nothing derived from a project.
- The description grows from four sentences to seven; callers read it once per session.
- The third required sentence asks calling agents to report the lead in points. Nothing in the server can test that an agent does so; it is guidance, and the specification claims no more for it.

## Deferred (named, not built here)

- **Changing the confidence number.** A fixed value was chosen first; once the evidence was stated correctly, the number was left alone. Any change needs a larger measurement first.
- **Running more than one pass.** Needs its own specification and measurement. Two findings to carry into it: under three identical passes a "separated" style label was produced for opposite answers when the option order or one context sentence changed; and it would depart from two design rules (repeated passes should differ; the caller owns the pass count) that must be named and justified there.
- **Detecting sensitivity to wording or order.**
- **A reliability label or a too-close-to-call label on the result.**
- **Revisiting the two memory-feature choices made with `decide` at a 30-point gap** (FR-009).
