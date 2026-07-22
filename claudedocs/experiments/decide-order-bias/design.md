# Experiment: decide order-bias (2026-07-21)

Does permuting option order flip `decide`'s argmax winner beyond sampling
noise? Tests the gap between the design corpus's judge-bias contract
(`NEW_SERVER_DESIGN.md` §4: "permute order of options/evidence and aggregate
across permutations" — a hard contract on every Decide) and the shipped 013
implementation (a single scored pass over options in caller order).

## Hypotheses

- **H1 (flip):** permuting option order changes the winner on near-tie
  decisions at a rate above the identical-order retest rate.
- **H2 (margin predicts stability):** flip probability rises as the reported
  score margin falls.
- **H3 (positional score bias):** the same option scores higher in position 1
  than later (primacy), independent of flips.

## Design

The confound: `decide` is a sampled pass — identical-order calls can disagree.
Every problem therefore carries a **test-retest arm**; the order effect is
`r1 − r0` where `r0` = identical-order flip rate, `r1` = permuted-order flip
rate, paired per problem.

**Precondition (verified in `src/modes/decide.rs` before running):** the
prompt lists options "in this order" from the caller array — permuting the
array permutes the prompt.

### Fixtures (`fixtures.json`, 40 problems)

| Group | N | Arms (calls) | Purpose |
|---|---|---|---|
| `near2` — near-tie, 2 options | 20 | orig ×2, reversed (3) | primary population |
| `near4` — near-tie, 4 options | 10 | orig ×2, reversed, rotated (4) | more positions → stronger masking effect? |
| `dominated` — clear winner, 2 options | 10 | orig ×2, reversed (3) | negative control |

Rules: options content-named (labels travel with content); option text
byte-identical across arms; decision shapes span weigh/causal/probabilistic
(coverage, not a powered factor). 130 calls total.

### Metrics

1. `r0` — retest flip rate (noise floor)
2. `r1` — permuted flip rate (first call vs each permutation)
3. Order effect `r1 − r0`, sign test on discordant problems + bootstrap CI
4. Positional Δ — mean score gain for the same option in position 1 vs later,
   paired across arms
5. Flip vs first-call margin (terciles)
6. Reported confidence vs observed stability
7. Control group: expect `r1 ≈ r0 ≈ 0`

### Pre-registered decision criteria

- `r1 − r0 ≥ 10` points on near-ties (or sign-test p < 0.05) → order bias
  confirmed → candidate feature 016: permuted-pass decide (k=2,
  agreement-based confidence).
- `r1 ≈ r0`, both low → single pass empirically adequate → amend the corpus
  with the measured null (Constitution I).
- `r0` itself high → sampling instability dominates; margin calibration is
  unreliable regardless of order — follow-up is multi-sample aggregation, not
  permutation.
- If H2 holds in any case: document the margin band below which results are
  order-noise.

### Threats to validity

Single model (recorded from the server env; per-model finding); authored
near-ties may be unnaturally symmetric; no temperature control through the
tool (absorbed by `r0`); 40 problems powers only coarse effects (≥10-point
differences), not small ones.

### Mechanics

`runner.py` speaks MCP stdio directly to `target\release\mcp-parallax.exe`
(4 worker connections, each with a scratch `DATABASE_PATH` so the live DB is
untouched; env taken from the user's parallax MCP config, never printed).
Raw per-call rows in `results.jsonl`; `analysis.md` holds the computed
metrics and verdict.
