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
