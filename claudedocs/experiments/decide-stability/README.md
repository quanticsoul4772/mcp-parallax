# decide stability measurement (2026-11)

Evidence for `specs/054-decide-reliability/spec.md` and for the 2026-11-17
amendment in `docs/design/NEW_SERVER_DESIGN.md`.

## What was measured

Ten recorded real `decide` inputs were sent again to the live model:

- unchanged, 5 or 10 times each ("identical runs");
- with the options in a different order;
- with one sentence of context removed.

The question was how much the scores, and the winner, move when nothing or
little about the input changes.

## Files

| File | What it is |
|---|---|
| `results.jsonl` | One line per attempted call, 460 in all. A completed call has the case label, the variant, the option ordering (`perm`), the scores in presented order, the methodology the model chose, and token counts. A failed call has the label, variant, ordering and `"error": "billing"`. |
| `plan-redacted.json` | Per case: its label, how many options and context sentences it had, and the scores and top-two gap recorded when the call was first made. |
| `table.py` | Prints every number in the specification's measurement table from the two files above. Standard library only; no network or model calls. |
| `table-output.txt` | The output of `table.py`, saved so the specification's table can be compared with it line by line. |
| `probe.rs.txt` | The program that made the calls, kept as text. It is not compiled. Running it makes paid model calls. |
| `redact.py` | The script that produced `results.jsonl` and `plan-redacted.json` from the raw files. It is here to be read: it shows that a failed call is written as `billing` only if its error was the provider's billing error, and that it stops otherwise. Its inputs are not in the repository, so it cannot be re-run from here. |

To recompute, from the repository root:

```bash
python -I -X utf8 claudedocs/experiments/decide-stability/table.py
```

## What is withheld, and why

The decisions, options and context of the ten inputs are not here. They are
real decisions from the owner's other projects. The provider's error text and
request identifiers for the failed calls are also removed. Everything in
these files is a label (`c00` to `c09`), a variant name, an ordering, a
score, a methodology name, a count, or the word `billing`.

Anyone can recompute the numbers. Only the owner can re-run the same inputs.

## Things to know before leaning on this

- **Dates.** The result files are dated 2026-11-17.
- **Model and effort are not recorded.** The probe built its client from the
  server's default configuration, which at the time named `claude-opus-4-8`,
  and it bypassed per-site routing. If a deployment routes `decide` to
  another model or effort, this is not a measurement of that deployment.
- **179 of 460 calls failed** because the account ran out of credit partway
  through the second stage. All 179 are that one failure (`redact.py` stops
  on any other). As a result the
  reordering and removed-sentence repeats exist for only five of the ten
  inputs, the first five in plan order.
- **60 completed calls are not used in the table.** They are the first
  stage's reorder and removal runs, one run per variant. The table's
  reorder and removal rows need five runs of a variant to say anything, so
  only the second stage's repeats are used.
- **Four identical runs used about twice their siblings' input tokens.** That
  matches `decide`'s single retry after a rejected answer. It is inferred
  from the token counts; the probe did not record retries.
- **The sample is small and clustered**: ten inputs, chosen in bands by
  recorded gap; five have 10 identical runs and five have 5.

The specification states what this does and does not establish.
