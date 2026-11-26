# Experiment proposal: decide run-to-run stability, second measurement

**Status: SHELVED on 2026-11-17, before any run. Nothing here was run and no
money was spent on it.** The text below is kept as written, as a record of a
design that should not be run in this form.

Why it was shelved:

- **Its second question could not justify what its results table said it
  would.** Agreement between identical runs shows that `decide` repeats
  itself for one exact wording and option order. The first measurement had
  already shown three runs agreeing on one option in one order and on the
  other option in another order, and likewise with one context sentence
  removed. The proposal listed this under "Limits" and then ignored it in
  "What each result would justify".
- **An independent review found the bars could not do their job.** Simulated
  on the first measurement's data, the three-run bar passed in every
  simulated experiment, because unanimous inputs swamp the close ones, and
  its measure was circular (the three runs were drawn from the ten that
  define the majority). The gap bar could be met by a band holding a handful
  of inputs with no disagreements at all. Several criteria ("clearly worse",
  "a useful share") were not numbers.
- **No design now on the table needs these numbers.**

If this is ever reopened, start from: the distribution of recorded gaps
across all recorded calls, which costs no model call and was not looked at
before sizing; option order varied within each input's runs; a minimum
number of inputs per band; and a balance check before any paid run.

Written 2026-11-17. Follows `claudedocs/experiments/decide-stability/` (the
first measurement) and `specs/054-decide-reliability/` (what was shipped on
it: a corrected description, and nothing else).

## Why

The first measurement showed that `decide` can name a different winner when
the same input is sent twice (5 of 10 inputs). It was too small to support a
design: ten inputs, chosen in bands by their recorded gap, half with only
five runs. Three designs built on it were rejected in review. Any change to
what `decide` computes, or to how many passes it runs, was deferred to a
larger measurement. This is the proposal for that measurement.

It may end the same way. "The data supports no design" is a named outcome
below, not a failure of the experiment.

## Questions, fixed before any data

- **Q1. At what first-run gap, if any, does a single run become reliable?**
  For a run with top-two gap *g*, how often does another identical run name a
  different winner, as a function of *g*?
- **Q2. How much do three agreeing runs tell you?** When three identical runs
  name the same winner, how often is that the winner of the majority of ten
  runs? And how often do three runs agree at all?

Out of scope, on purpose: sensitivity to option order and to wording. The
first measurement showed both exist. They need their own designs; adding them
here would double the call count without answering Q1 or Q2.

## Inputs

- **Population**: recorded successful `decide` calls from the operator's agent
  sessions with 2 to 5 options (about 785 after removing duplicates). Unlike
  the first measurement there is no requirement on context length and no
  banding by gap.
- **Sampling**: simple random sample with a fixed, recorded seed, so the
  result describes the real mix of calls. The seed and the selection
  script are committed; the input texts are not.
- **Per input**: 10 identical runs, unchanged input, same option order.
- **Privacy**: the inputs are real decisions from other projects.
  They are sent to the same provider API those projects' agents already sent
  them to, and nowhere else. Only labels, scores, methodology names and token
  counts are committed, as in the first measurement.

## Size and price

Measured on the first measurement's 281 completed calls: 1,430 input and 345
output tokens per call on average. At `claude-opus-4-8`'s price in
`src/telemetry.rs` (5 dollars per million input tokens, 25 per million
output) that is about 1.6 cents per call.

| Stage | Inputs | Calls | Expected price | Hard cap |
|---|---|---|---|---|
| 1 | 40 | 400 | about 6.30 dollars | 8 dollars |
| 2 (only under the rule below) | 40 more | 400 | about 6.30 dollars | 8 dollars |

**These prices hold only for that model.** The model is the one the operator's
server routes `decide` to, which is not known to the author:

> **Model for this run: ________________**

If it is not `claude-opus-4-8`, the price is recomputed from the project's
price table before the run.

**Stage 2 runs only if** stage 1 shows a gap band in which the disagreement
rate is under 10% but its interval is too wide to rely on (upper bound above
20%). It has its own price. The first
measurement's overrun was in its second stage; this one is a separate run.

## Measures

For each input: the winner of each run, the majority winner of its ten runs,
and each run's top-two gap.

- **For Q1**: over all ordered pairs of identical runs of the same input,
  the share naming different winners, grouped by the first run's gap in bands
  fixed now: 0 to 8, 9 to 16, 17 to 30, 31 to 50, over 50. Intervals are
  computed by resampling inputs, not pairs, because pairs within an input are
  not independent.
- **For Q2**: over all sets of three runs of the same input, the share that
  agree; and among those that agree, the share whose winner is the input's
  majority winner. Same resampling.
- **Reported alongside**: how many inputs changed winner at all; the
  distribution of first-run gaps in the sample (this is also the first
  unbiased estimate of how often real calls are close).

## What each result would justify, decided now

| Result | What it supports |
|---|---|
| Q1: some band has disagreement under 5% with its interval's upper bound under 10%, and lower bands are clearly worse | A statement in the description, and possibly a label, that leads above that gap have been reliable. A specification would follow; nothing is built from this document. |
| Q1: disagreement falls with the gap but no band meets that bar | The description gains the measured rates per band. No label, no threshold. |
| Q1: no clear relation between gap and disagreement | The gap is not a usable signal. Nothing keyed on the gap is ever built; the design document records it. |
| Q2: three agreeing runs match the majority of ten at least 95% of the time (interval lower bound at least 90%), and three runs agree in a useful share of cases | An operator-enabled three-pass mode becomes worth specifying, with its two departures from the design rules named there. |
| Q2: below that | Multi-pass at three is not worth its cost. Recorded; not built. |
| Any | The limits below are stated with the result. No result here changes `decide` by itself. |

## Safeguards

Each is a property of the probe program, checked by a test or by reading it,
before a run.

1. **Call cap.** The probe takes the maximum number of calls as an argument
   and stops when it is reached.
2. **Cost cap.** After every call it adds that call's cost, computed from the
   returned token counts and the project's price table for the model in use,
   and stops before the next call if the total has reached the cap.
3. **Stop on the first billing error**, and on any error it does not
   recognise. The first probe kept going through 179 billing errors.
4. **It uses the server's own routing** for the `decide` call site, and
   records the model and effort in every output line. The first probe
   bypassed routing and recorded neither.
5. **Price stated first.** The call count, the model, the expected price and
   the cap are stated before the run.
6. **It is not left in `examples/`.** It lives outside the build, and its
   source is committed as text with the results.
7. **The operator's live server shares this account.** The cap is set with
   that in mind.

## Limits

- One model, on one day. Nothing transfers to another model.
- Ten runs per input estimates an input's majority winner imperfectly; an
  input that splits 6 to 4 can show 7 to 3 the other way.
- Recorded calls are the calls agents chose to make. They are not every
  decision made.
- Identical-input runs measure repeatability for that exact wording. They say
  nothing about whether the wording was fair to each option, which the first
  measurement showed also moves the result.

## What happens to this document

If run, the results, the scripts and an analysis note are
committed beside it, and the outcome is recorded in the design document
whichever way it falls.
