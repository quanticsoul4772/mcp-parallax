# Jev 064 measurements, 2026-11-24 (T038, base rate)

## Source

The operator's live Parallax database (`DATABASE_PATH`, under
`%APPDATA%\Claude\parallax.db`), read-only queries. Records span
2026-06-12 to 2026-11-25 (UTC).

## Base rate by boundary

| Boundary | Evaluations | Review ran | Signal fired |
|---|---|---|---|
| turn | 761 | 648 | 5 |
| batch | 11,032 | 0 | 158 |
| action | 7,997 | 0 | 14 |

## Turn-review positives

The plan counts a reviewed turn as positive when the review flagged a
contradiction or violation, or captured something.

- Flags: 5 of the 648 reviewed turns have verdict `flag`; the other 643 are
  `silence`. `delivered_keys` is non-empty on those 5 only.
- Captures: `checkpoint_records` does not record captures. The `memories`
  table holds 3 end-of-turn auto-captures (`origin` starting
  `auto-capture:`), one on 2026-08-31 and two on 2026-11-15.
- Overlap between flags and captures is unknown, so positives are at most 8
  in 648 reviews, about 1.2%.

## What it means for promotion (plan §7)

- 95% recall target: at least 59 positives with 0 misses. At 1.2% that is
  about 4,800 reviewed turns; at the past rate of about 3.9 reviews a day
  (648 in 166 days), more than 3 years.
- 90% recall target: 29 positives with 0 misses, about 2,400 reviewed turns,
  about 1.7 years at the past rate.
- Research-claims promotion (065) is not limited by this rate: each research
  run yields many claims.

## Live check after the restart

- The `checkpoint_turn` Stop hook is in the user-level Claude Code settings,
  so turn reviews now run in every project. The past rate was collected
  under a narrower hook, so the future rate should be higher, by an amount
  not yet known.
- The first live shadow row was written 2026-11-25 04:21 UTC: site
  `turn_review`, outcome `success`, 139 ms.

## Decision

Wait, and re-measure the turn-review rate under the user-level hook around
2026-12-08, then decide what happens to the 066 turn-review skip. The plan is
unchanged. User's pick: "Wait, re-measure (82)".

## T039: harness report on the live acceptance records (SC-005)

Run on main at `3c1308d` (PR #162) with
`DATABASE_PATH=./data/jev-acceptance.db PARALLAX_BACKEND=claude_cli cargo run --release --example jev_backtest -- report`:

```text
== turn_review (6 rows)
  success: 6
  labeled: 6; cost: $0.000099
  confirm set: 4 rows, sealed (not looked at)
  tune items: 2 (2 scored, 0 unscored and never skipped, of which 0 abandoned because verify finished first; 0 without a cluster); left out: 0 unlabeled
  evidence for recall, its bound and the dry run: 2 scored, clustered items (0 unscored harmful items are not counted)
  harmful (plan 7.1): 2 of 2; needed for promotion: 59
  skip threshold: none meets the target; nothing is skipped
  at the threshold: skipped 0 (harmful 0), sent to the big model 2 (harmful 2)
  skip recall: 1.0000; one-sided lower bound 0.2236; cluster bootstrap 1.0000..1.0000
  skip share: 0.0000; one-sided lower bound 0.0000
  harmful among skipped: undefined; one-sided upper bound undefined
  baselines: always skip: recall 0.0000, skipped 2; never skip: recall 1.0000, skipped 0
  clusters: 2
  promotion dry run on the tune set (not a promotion): fails
    - 2 harmful positives; at least 59 needed
    - recall lower bound 0.2236 is below the target 0.95
    - nothing skipped: does not beat the never-skip baseline
  not checked by the dry run:
    - skip-share minimum (plan 7.2): the plan sets no number yet
    - wrong-settle cap (plan 7.2): the plan sets no number yet
== research_claims (167 rows)
  excluded: 34
  success: 133
  labeled: 167; cost: $0.007981
  confirm set: 52 rows, sealed (not looked at)
  tune items: 115 (94 scored, 21 unscored and never skipped, of which 0 abandoned because verify finished first; 0 without a cluster); left out: 0 unlabeled
  evidence for recall, its bound and the dry run: 94 scored, clustered items (1 unscored harmful items are not counted)
  harmful (plan 7.1): 3 of 115; needed for promotion: 59
  skip threshold: none meets the target; nothing is skipped
  at the threshold: skipped 0 (harmful 0), sent to the big model 115 (harmful 3)
  skip recall: 1.0000; one-sided lower bound 0.2236; cluster bootstrap 1.0000..1.0000
  skip share: 0.0000; one-sided lower bound 0.0000
  harmful among skipped: undefined; one-sided upper bound undefined
  baselines: always skip: recall 0.0000, skipped 115; never skip: recall 1.0000, skipped 0
  clusters: 1
  promotion dry run on the tune set (not a promotion): fails
    - 2 harmful positives; at least 59 needed
    - recall lower bound 0.2236 is below the target 0.95
    - nothing skipped: does not beat the never-skip baseline
  not checked by the dry run:
    - skip-share minimum (plan 7.2): the plan sets no number yet
    - wrong-settle cap (plan 7.2): the plan sets no number yet
```

The SC-006 run (reproducing T038's tables from saved inputs) waits for
T038's live measurements.

## T038: Jev noise and option order

Source: the acceptance database (`./data/jev-acceptance.db`), research-claim
rows only, all from one research run on one question. Turn-review states are
not stored, so turn noise cannot be measured from stored rows. Commands, run
on main at `6e2ecac`:
`DATABASE_PATH=./data/jev-acceptance.db PARALLAX_BACKEND=claude_cli cargo run --release --example jev_backtest -- noise 20`
(pilot), then `noise 100`, then `order 100`.

| Measurement | Rows | Calls | Failed | Result |
|---|---|---|---|---|
| Noise, pilot | 20 (40 question-row pairs) | 100 | 0 | mean SD 0.0014; 0 decisions flipped across 5 calls |
| Noise | 100 (204 question-row pairs) | 500 | 0 | mean SD 0.0016; 0 decisions flipped across 5 calls |
| Option order | 100; 102 `supports` decisions per order | 600 | 0 | 0 flips under each of the 5 other orders; McNemar 0 |

Limits:

- One topic: every row comes from a single research run.
- The spread is on the probability scale, not the logit scale the plan uses
  to set the middle band, and the rows are not limited to the held-out tune
  split.
- The client writes each choice question's options in the order of its list
  (`src/client/typesafe.rs:322-332`), and the harness reorders that list, so
  each order does reach the request body. The only client test covers the
  default order, so it would also pass if the order were fixed.
- Whether TypeSafe's server keeps JSON key order is unknown. If it reads the
  options as an unordered map, zero flips would be expected whatever Jev's
  first-option bias.

## Still to measure in T038

- Big-model self-agreement.
- Hand-labeled correctness (at least 60 items).
- The injection back-test on `jev-1.14.0`.
