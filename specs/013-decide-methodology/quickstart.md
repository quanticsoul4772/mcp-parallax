# Quickstart: Decide — Methodology-Driven Choice

No new configuration. `decide` is always in the catalog (like `verify` / `unstick` /
`diverge`).

## Choose among options, with the work shown

```jsonc
{
  "decision": "How should we ship the migration?",
  "options": ["big-bang cutover", "incremental dual-write", "feature-flag ramp"]
}
// =>
{
  "recommended": "feature-flag ramp",
  "runner_up": "incremental dual-write",
  "runner_up_reason": "scored 15 below feature-flag ramp: similar safety but slower to fully retire the old path",
  "confidence": 0.575,                 // close-ish call → modest confidence
  "methodology": "weigh",
  "deciding_factors": ["blast radius", "rollback speed", "effort"],
  "assessments": [
    { "option": "big-bang cutover",       "score": 40, "rationale": "fast but no incremental rollback" },
    { "option": "incremental dual-write",  "score": 70, "rationale": "safe, but heavy and slow to retire" },
    { "option": "feature-flag ramp",       "score": 85, "rationale": "safe and reversible at each step" }
  ]
}
```

The model scored every option; the **server** picked the top, named the runner-up, and
computed the confidence from the score margin. A dominant winner reads high confidence; a
near-tie reads ~0.5.

## What it does not do

```jsonc
// Fewer than two options → invalid input (no fabricated comparison):
{ "decision": "...", "options": ["only one"] }   // => invalid_input

// Decide never returns a truth verdict (use `verify`) or a single next step (use `unstick`).
```

## The methodology matches the decision

- Multi-criteria decision → `methodology: "weigh"` (criteria in `deciding_factors`).
- Downstream-effects decision → `methodology: "causal"` (effects each option causes).
- Uncertainty-dominated decision → `methodology: "probabilistic"` (likelihoods).

## Validation

- Offline (`cargo test`): a dominant score vector → the top option with **high**
  confidence; a near-tie vector → **lower** confidence; the output always carries
  recommended + runner-up + reason + factors + methodology; `< 2` options is rejected; the
  per-pass schema registers flat + closed; arity mismatch (scores vs options) is a failed
  pass. All deterministic — the pick and confidence are server math over the scores.
- **Live** (dogfood): SC-004 — that the model picks the methodology that *fits* the
  decision's shape — is confirmed against the running model (a mock cannot judge fit). The
  calibration itself needs no live check.

## Validation results (full gate, 2026-06-14)

`cargo fmt --all -- --check` clean · `cargo clippy --all-features --all-targets -- -D
warnings` clean · `cargo test` **343 lib + 58 integration, 0 failed** ·
`cargo run --example acceptance_decide` ALL CHECKS PASS.

New coverage (all offline — the pick and confidence are server math over the scores):

- **Calibration:** dominant `[85,40]` → recommended top, confidence **0.725**; near-tie
  `[60,55]` → **0.525**; exact tie `[70,70]` → input-order winner at **0.5**; margin→
  confidence map at 0/50/100 → 0.5/0.75/1.0.
- **Validation (loud):** arity mismatch (scores vs options) → failed pass; an out-of-range
  score (`105`) → failed pass, **not clamped** (analyze M1); empty `deciding_factors` →
  failed pass; `< 2` options or empty decision → `invalid_input` before any model call.
- **Surface:** the methodology echoes the model's choice (`weigh`/`causal`/`probabilistic`);
  the schema registers flat + closed (scalar enum + integer/string arrays). Integration:
  full output (recommended/runner_up/reason/factors/methodology/assessments), no
  `verdict`/`next_step`, one record, single-pass token usage.

**Pending live (T010, post-rebuild):** SC-004 — the model picks the *fitting* methodology
for the decision's shape, and the rationale reads in that methodology's terms — the one
offline-impossible check (a mock can't judge fit). The `acceptance_decide` example is the
offline calibration scaffold.
