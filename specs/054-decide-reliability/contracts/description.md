# Contract: the `decide` tool description

This is the text callers receive in the tool catalog. It is written twice in
the source (a constant and a literal); a test holds the two equal.

## Before

> Choose among two or more options under tradeoffs, with the reasoning shown.
> Applies an explicit decision methodology (weigh named criteria, trace what
> each option causes, or reason under uncertainty), scores every option, and
> returns the recommended option, the runner-up and why it lost, the deciding
> factors, the methodology used, and a confidence calibrated to how close the
> call is. The choice is computed from the scores, not asserted - never a menu
> handed back, never a hidden gut pick. To judge whether a claim is true use
> verify; for one next step when you are looping use unstick; for a computable
> comparison use check.

## After

> Choose among two or more options under tradeoffs, with the reasoning shown.
> Applies an explicit decision methodology (weigh named criteria, trace what
> each option causes, or reason under uncertainty), scores every option, and
> returns the recommended option, the runner-up and why it lost, the deciding
> factors, the methodology used, and a confidence. Confidence restates this
> run's top-two score gap (0.5 + gap / 200). Scores come from one pass: an
> identical call can give different scores and a different winner (seen on one
> input with leads of 38 points one way and 33 the other), and the winner has
> changed when one sentence of context was removed and when the options were
> reordered. Report the lead in points alongside the recommendation, and say
> when it is small. The choice is computed from the scores, not asserted - never a menu
> handed back, never a hidden gut pick. To judge whether a claim is true use
> verify; for one next step when you are looping use unstick; for a computable
> comparison use check.

## What changed

- Removed: "calibrated to how close the call is".
- Added: the three required sentences of FR-002, in order, after "and a confidence."
- Nothing else. The routing sentences at the end are untouched.

## The `confidence` field description

Before: ``Confidence, server-derived from the score margin (`[0.5, 1.0]`).``

After: `This run's top-two score gap rescaled to [0.5, 1.0] (0.5 + gap / 200). An identical call can score differently.` It carries no code formatting, and is written on one source line because the schema generator keeps the line breaks of a wrapped comment.

## Not a contract

The third required sentence asks calling agents to report the lead. The
server cannot observe whether an agent does. It is guidance.
