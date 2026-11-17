# Research: Decide Stops Claiming Calibrated Confidence

No open technical unknowns. This file records the decisions and what was
rejected, because the path here went through three rewritten specifications.

## D1 — Change words, not numbers

**Decision**: the confidence formula, fields and prompt are unchanged; only
descriptions and documents change.

**Rationale**: the measurement (10 inputs, one model) proves the "margin ≥ 17
was perfectly stable" statement false and shows results can change with
wording and option order. It does not show how reliability varies with the
gap. Any new number, label or threshold would be fitted to a sample that
cannot support it.

**Alternatives rejected**, each after independent review:

- *Gap-triggered auto-repeat* (two more passes when the first gap is 15 or
  less): replayed on the recorded runs it reported disagreement on well under
  half of the unstable inputs and never repeated on the worst one.
- *A "within noise" label at a 15-point threshold*: runs at or under 15 and
  runs over 15 named the less common winner at about the same rate.
- *Operator-enabled three identical passes with a "separated" label*: the
  label was produced for opposite answers when option order or one context
  sentence changed; it also departs from two design rules.
- *Fixed confidence of 0.5 plus a gap field*: 0.5 already means "tie"; the
  data does not support "coin flip"; the gap field restates the old number.
- *"Confidence is not a measure of reliability" and a 40-point guidance
  figure*: the first overclaims in the other direction; the second rests on
  one input and moves with which runs are counted.

## D2 — How the description is held to its text

**Decision**: four tests.

1. Unit, in `src/modes/decide.rs`: `DECIDE_DESCRIPTION` contains each FR-002
   sentence exactly once and does not contain the stem `calibrat`
   (case-insensitive).
2. Unit, same file: the derived `DecideResult` schema's `confidence`
   description equals the FR-003 text, and equals the `confidence`
   description in `specs/013-decide-methodology/contracts/decide.tool.json`.
3. Integration, in `tests/integration.rs`: the `decide` description served in
   the tool list equals `DECIDE_DESCRIPTION`. The two copies are identical
   today, so this test passes before any change; to see it fail for its
   stated reason the constant is edited first, the failure recorded, and the
   literal edited second.
4. Unit: `PROMPT_TEMPLATE` and the derived `DecidePass` schema equal their
   text before this feature (pinned as literals in the test). Nothing pins
   them today; the existing prompt test only counts placeholders.

**Rationale**: equality between the two copies alone would pass if both kept
the old claim; the exact-string test on the constant closes that. The
existing contract test compares property names and constraints only, so the
field description needs its own check.

**Alternative rejected**: one permanent test scanning several source and
document files for the stem. This project's history (specs 040 to 045) is a
list of scans that passed for reasons unrelated to what they claimed. The
wider list is checked once, by a recorded search, in the task list.

## D3 — The derived schema's description text

`schemars` builds a field's `description` from its doc comment and keeps the
line breaks of a wrapped comment (the existing contract file shows newlines
inside several descriptions). The FR-003 text must therefore sit on one
source line, about 121 columns, or be given as an explicit description
attribute. Test 2 compares after the derive and would catch a wrap.

## D4 — Live text versus historical record

**Decision**: correct in place only text that describes the tool as it is now
or that tests and callers read; leave dated records as written and add one
dated line to the few named in FR-008 and FR-009.

**Rationale**: the earlier `decide` specification, its research and the
earlier experiment's analysis are accurate records of what was decided and
observed then. Rewriting them would erase why the tool was built as it was.
Two files under that specification are the exception. The contract file is
read by a test. The contract note is not read by any test; it is treated as
live only because it quotes the description as current, and the correction
there is limited to replacing that quoted block whole with the new text. The
note's other staleness (it is headed draft and its input block is out of
date) is not this feature's to fix.

## D5 — The design amendment

**Decision**: a new dated block appended after the 2026-07-21/22 amendment in
`docs/design/NEW_SERVER_DESIGN.md`, containing the seven statements of
FR-006. The catalog rows that list "miscalibration" for Decide (that file and
`NEXT_REASONING_SERVER.md`) are not edited; the amendment names them.

**Rationale**: the corpus is amended by dated blocks; the earlier amendment's
own wording ("an interim directional effect at n = 10 was refuted with
power") shows why an edit-in-place would lose the sequence of evidence.

## D6 — Evidence and privacy

**Decision**: a redaction script reads the two result files and the plan and
writes `results.jsonl` (all 460 attempted calls; scores, orderings,
methodology, token counts; failures reduced to the word "billing") and
`plan-redacted.json` (per case: id, option count, sentence count, recorded
scores, recorded gap). No decision, option or context text, no provider error
text, no request identifiers. The owner sees both output files before they
are committed.

**Rationale**: the inputs are real decisions from the owner's other projects.
The numbers in the specification's table need only ids, orderings and scores.

**Verification the redaction worked**: the script stops with an error if any
failed call's error text is not the provider's billing error; it checks that
every case id is one of `c00` to `c09`; the task list includes a search of
the two output files for any string value outside the known short
vocabulary; and the owner inspects both files.

## D7 — The probe program

**Decision**: its source is kept in the evidence folder as `probe.rs.txt`
(not compiled, not under `examples/`), with its header changed from
"TEMPORARY, not committed" to a statement of what it was used for and a
warning that running it makes paid model calls. The file under `examples/` is
deleted by the owner.

**Rationale**: without it nobody can see how the runs were made (that it used
the default model directly and bypassed routing). Left under `examples/` it
is buildable code that spends money when run.

## D8 — Live acceptance

**Decision**: none is required to build or test. Whether to make one live
`decide` call before the pull request is put to the owner with its price.

**Rationale**: nothing sent to `decide`'s model changes. The project's note
says any model-facing prompt or schema change must pass the live example
before a pull request. The description is read by calling models, and no live
`decide` example exists (the existing acceptance example is offline). The
owner is asked with that stated.
