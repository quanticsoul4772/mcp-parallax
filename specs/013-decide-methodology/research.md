# Research: Decide — Methodology-Driven Choice

Phase 0 decisions. The clarification settled the shape (model scores → server picks the
top; single pass; confidence from the score margin). These resolve the mechanism against
the flat+closed constrained-output contract and the existing mode machinery.

## D1 — Per-option scores under a flat+closed schema: parallel scalar arrays

**Decision**: the model returns the per-option assessment as **three index-aligned scalar
arrays**, not an array of objects:

- `option_scores`: array of integer (0–100), one per input option, in option order.
- `option_rationales`: array of string, one per input option — why it scored as it did.
- `deciding_factors`: array of string — the factors/criteria the methodology used.

Plus one scalar:

- `methodology`: enum `weigh | causal | probabilistic` (the surfaced frame).

**Rationale**: the flat-schema gate (`assert_flat`, Constitution II) forbids arrays of
objects — only scalars, scalar enums, and arrays of scalars are legal. A per-option
`[{option, score, rationale}]` would be rejected at boot. Parallel scalar arrays are the
flat-compliant encoding of a variable-length per-option structure; the server **zips**
them with the input option labels (by index) into the assessment it ranks. The model
scores option *i* (the prompt lists the options in order); the server validates that
`option_scores.len() == option_rationales.len() == options.len()` — a mismatch is a
failed pass, never a silent realign.

`methodology` is a **proper scalar enum** (not a nullable string): a non-null enum is
grammar-enforced and flat-legal (verify's `VerdictKind` is the precedent). The 011 H1
lesson — avoid `Option<enum>` (which `schemars` renders as `anyOf`) — does not bite here
because the field is required and non-null.

**Alternatives**: array of objects (rejected — not flat+closed, fails at boot); a single
delimited string the server parses (rejected — brittle free-text parsing, forbidden by
Constitution II); JSON-in-a-string (rejected — same).

## D2 — The recommendation is deterministic server math over the scores

**Decision**: the server ranks the zipped assessments by `score` descending. The
**highest** is the recommendation; the **next-highest** is the runner-up. The "why it
lost" reason is composed server-side from the runner-up's own rationale plus the margin
("scored N below {winner}: {runner-up rationale}"). The model **never** names the winner —
it only scores; the pick is `argmax(scores)`.

**Tie handling**: a tie on the top score resolves deterministically by **input order**
(the earlier option wins) and surfaces as the lowest confidence (D3) — the output says the
call is a near-tie rather than inventing a separating factor (spec edge case).

**Rationale**: this is the deterministic-over-probabilistic principle applied to the
*assembly* (Constitution V) — the choice is a pure function of the model's scores, so it
cannot be an unexamined preference asserted directly (FR-004). Argmax + input-order
tiebreak is total and stable.

## D3 — Confidence from the score margin

**Decision**: confidence is server-derived from the margin between the top score and the
runner-up: `confidence = 0.5 + 0.5 * min(margin, SCALE) / SCALE`, where `margin =
top_score − runner_up_score` and `SCALE = 100` (the score range). Clamped to `[0.5, 1.0]`.

- Margin 0 (a tie) → **0.5** (a coin flip between the top two — the honest floor for a
  binary-ish call).
- A 50-point lead → **0.75**; a 100-point lead → **1.0** (dominant winner).

**Rationale**: a deterministic, monotonic map from "how close the call is" to a confidence
value (FR-005, SC-001/SC-002). It is calibrated to the *margin*, not model self-report and
not a constant. `0.5` floor reflects that having picked *an* option from ≥2 is never worse
than a coin flip. The `0.5 + 0.5·…` form is the v1 mapping; `SCALE`/floor live as named
constants (Scope discipline), tunable without a config var.

**Alternatives**: confidence = margin/SCALE (rejected — a clear win at 40-point margin
would read 0.4, miscalibrated low); softmax over all scores (rejected — overkill, and the
decision is the top-two gap, not the full distribution); cross-pass agreement (rejected by
the clarification — single pass, no ensemble).

## D4 — Single pass, no ensemble, no quorum

**Decision**: `Decide` runs **one** model call (the clarification), validates it, and
server-assembles the result. It does **not** use `aggregate_core` or the k-pass ensemble.
If the single pass fails (refusal/timeout/validation), the error propagates directly —
there is no quorum to fall back on (one pass is the whole signal). `unstick` is the
precedent for a single-pass corrective.

**Rationale**: the clarification chose single-pass with margin-confidence; an ensemble
would source confidence from agreement instead, which the clarification rejected. One pass
keeps cost at one call and the calibration purely from the scores.

## D5 — Testing: the calibration is offline; only score/methodology *quality* is live

**Decision**: because the pick and confidence are **server math over the model's scores**,
SC-001 (dominant → high confidence), SC-002 (close → lower confidence), SC-003 (output
completeness), and SC-005 (< 2 options rejected) are **fully offline-testable** — a mocked
model returns a chosen score vector and the server's deterministic assembly is asserted.
Only **SC-004** (the model picks the methodology that *fits* the decision's shape) and the
qualitative sensibility of real scores are **live** properties (a mock cannot judge fit),
confirmed by a small live dogfood.

**Rationale**: unlike `verify`/`diverge` (whose headline SC-001 is an emergent live
property), Decide's calibration is deterministic, so most of its success criteria are
provable offline. This narrows the live surface to methodology-fit alone.

## D6 — Output surface

**Decision**: `DecideResult` (server-assembled, may be nested like `grounded_verify`'s
manifest):

- `recommended` (string — the winning option label), `runner_up` (string),
  `runner_up_reason` (string — server-composed), `confidence` (f64), `methodology`
  (string — surfaced), `deciding_factors` (string[]),
- `assessments`: `[{option, score, rationale}]` — the full per-option breakdown
  (server-zipped from the parallel arrays), for audit.

No `verdict` (not `verify`), no single `next_step` (not `unstick`). The output is a
recommendation with its full scored rationale.
