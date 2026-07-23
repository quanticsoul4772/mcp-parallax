# Results: decide order-bias experiment

**Run**: 2026-07-21 · `claude-opus-4-8` · 130/130 calls, 0 errors · $1.68
(151,652 input / 37,018 output tokens, from the invocation records) · raw
data `results.jsonl`, computed by `analysis.py`.

## Headline

| Group | n | r0 (retest flips) | r1 (order flips) | r1 − r0 | sign test p |
|---|---|---|---|---|---|
| dominated (control) | 10 | 0/10 = 0% | 0/10 = 0% | 0 | — |
| near-tie, 2 options | 20 | 1/20 = 5% | 1/20 = 5% | **0** | 1.0 |
| near-tie, 4 options | 10 | 1/10 = 10% | 6/20 = 30% | **+20 pts** | 0.5 (b=2, c=0) |

- **Controls behave perfectly**: zero flips of any kind on dominated
  decisions. The tool is not randomly unstable.
- **2-option decisions: measured null.** Order flips (1/20) exactly equal
  retest flips (1/20); positional score boost −0.57 points (p = 0.57 — noise,
  and directionally *anti*-primacy). A single pass is empirically adequate
  at k = 2.
- **4-option decisions: effect-size criterion met, underpowered.** 30% of
  permuted pairs flipped the winner vs a 10% retest floor — over the
  pre-registered 10-point bar — but with only 10 problems the sign test
  (order-only flips b=2 vs retest-only c=0) cannot reach significance
  (p = 0.5). Directional evidence, not a confirmed effect. Flipped problems
  (n4-01 message queues, n4-04 analytics engines, n4-02 also retest-unstable)
  are genuine near-ties.

## H2 — margin predicts stability: strongly supported

Flips by first-call score margin (all 30 near-tie problems):

| Margin tercile | n | order flips | retest flips |
|---|---|---|---|
| 2–8 | 10 | 3 | 2 |
| 10–16 | 10 | 1 | 0 |
| 18–30 | 10 | 0 | 0 |

**Every instability of either kind lives at margin ≤ 16, and almost all at
≤ 8.** Margin ≥ 18 was perfectly stable across 40 paired comparisons. The
margin-derived confidence already encodes fragility — but weakly at the
surface: mean reported confidence was 0.575 on order-stable vs 0.531 on
order-flipped problems, a separation callers will not notice.

## H3 — positional primacy: null

No positional score boost in either 2-option group (near2 −0.57 pts,
p = 0.57; dominated +0.5 pts, p = 0.27).

## Verdict against the pre-registered criteria

1. **k = 2 (the common case): amend the corpus with the measured null.** The
   §4 judge-bias prescription ("permute order… aggregate") is not needed for
   two-option decide calls on this model; sampling noise, not position, is
   the residual instability — and it is already margin-visible.
2. **k ≥ 3: directional yes, underpowered.** Either (a) extend the
   experiment — 30–40 four-option near-ties (~$2–3) would power the sign
   test properly — or (b) accept the directional result and scope the fix.
3. **The actionable design insight is margin-gating, not blanket
   permutation.** All observed flips sat below margin ~16. A candidate 016:
   when `decide` has ≥ 3 options AND the first pass's margin falls below a
   threshold (≈ 15), run one permuted second pass; an argmax flip resolves to
   "too close to call — treat as a coin flip between X and Y" and agreement
   feeds confidence. Cost lands only on the close calls (here: ~⅓ of
   near-tie calls, 0% of dominated ones), and the confidence separation
   becomes explicit instead of a 0.04 nudge.

## Threats / caveats

Single model (`claude-opus-4-8`); authored fixtures (near-tie symmetry is
constructed); near4 powered only for large effects; one permutation pair per
2-option problem (reversal), two for 4-option (reversal + rotation).

---

## Power extension (2026-07-22)

The `decide` tool itself chose this step (dogfooded, with a permuted
confirmation pass after the first call landed in its own fragile margin
band): 30 additional four-option near-ties (`fixtures-ext.json`), same arms,
120 calls, $1.69, 0 errors (`results-ext.jsonl`, `analysis_ext.py`).

### Pooled four-option result (n = 40 problems, 80 permuted pairs)

| Metric | Value |
|---|---|
| r0 (identical-order retest flips) | 7/40 = **17.5%** |
| r1 (permuted-pair flips) | 15/80 = **18.8%** |
| Effect r1 − r0 | **+1.3%** |
| Discordant problems (order-only vs retest-only) | 6 vs 3 |
| Exact sign test | p = 0.51 |

**The directional k=4 effect is refuted.** The original 30%-vs-10% split was
small-sample noise; with power, permuted-order flips equal identical-order
flips. Order is not the cause of four-option instability — **sampling is**:
a 17.5% retest flip rate on near-ties is the pre-registered third branch
("r0 itself high → sampling instability dominates; the follow-up is
multi-sample aggregation, not permutation").

The margin band sharpened with the added data:

| orig1 margin | n | order flips | retest flips |
|---|---|---|---|
| 0–8 | 18 | 9 | 7 |
| 9–16 | 10 | 1 | 0 |
| ≥17 | 12 | 0 | 0 |

Max margin with any order flip: **11**. Margin ≥ 17 has never produced an
instability of any kind across the whole experiment (both runs, all groups).

### Final verdict (supersedes item 2–3 above)

1. **No order-bias mitigation is warranted at any k** — k=2 measured null,
   k=4 refuted with power. Margin-gated *permutation* is dead: re-running
   the same order flips just as often as re-running permuted.
2. **The durable finding is a calibration rule**: below margin ~12–16 a
   `decide` winner on many-option near-ties is a coin flip among the top
   scorers — regardless of order — and the reported confidence understates
   this. If anything ever ships from this experiment, it is low-margin
   *multi-sample* aggregation or an explicit too-close-to-call surface, not
   permutation.
3. Corpus §4 amendment updated to the pooled result in the same change
   (Constitution I).

Total experiment spend: $3.37 across 250 calls, 0 errors.
