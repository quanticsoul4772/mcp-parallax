# Quickstart: Verification Reliability

No new configuration. Both fixes change verdict trustworthiness, not the call shape.

## `verify` — confidence becomes a real signal

Same call as always; the *k* passes now apply distinct critical lenses, so a
contestable claim no longer pins confidence at 1.0:

```jsonc
// a claim independent lenses can legitimately split on
{ "claim": "<a genuinely contestable assertion>" }
// => { "verdict": "...", "confidence": 0.67, "passes": 3 }   // graduated, not 1.0
```

A clear error still converges (`refuted`, named finding, high confidence); a
clearly-true claim still returns `supported` with no manufactured findings.

## `grounded_verify` — `inconclusive` instead of a confident wrong verdict

The dogfooded reproduction now abstains rather than confidently refuting a true,
countable claim:

```jsonc
{ "claim": "src/server.rs is over 1000 lines", "locators": [ { "path": "src/server.rs" } ] }
// => { "verdict": "inconclusive", "reason": "computable property — route to `check`" }
```

Route the countable part to the deterministic engine, which counts and decides:

```jsonc
// check, with the count supplied
{ "claim": "1224 > 1000" }   // => { "verdict": "supported", "engine": "arithmetic" }
```

A genuine judgment claim about source content is unchanged — it still runs the
stance-blind passes and returns `supported`/`refuted`.

## Validation

- Offline (`cargo test`): the *k* lens prompts differ; `aggregate_core` returns
  ≈0.67 / 0.5 / sub-quorum on constructed vote vectors; `grounded_verify` returns
  `inconclusive` when a majority of passes set `needs_computation`, and still returns a
  confident verdict when only advisory `missing_evidence` is listed (no over-abstention).
- **Live** (dogfood): SC-001 — that real contestable claims actually scatter across
  lenses to produce graduated confidence — is confirmed against the running model
  (a mock can't disagree with itself). Re-run the borderline battery that returned
  0/8 graduated and confirm a spread.

## Validation results (offline gate, 2026-06-14)

`cargo fmt --all -- --check` clean · `cargo clippy --all-features --all-targets -- -D
warnings` clean · `cargo test` **307 lib + 50 integration, 0 failed** · examples
compile. New offline coverage:

- `verify`: per-pass prompts pairwise distinct (lenses injected), the lens set is
  non-empty with unique names and cycles at `k > len`, and the prompt template
  exposes only the lens + claim + context slots (no stance). Aggregation vote-vector
  tests (2:1 → ≈0.67, tie → refuted, sub-quorum → dominant failure) unchanged.
- `grounded_verify`: majority `needs_computation` → `inconclusive` (route to
  `check`); advisory-only `missing_evidence` keeps the confident verdict (no
  over-abstention); a single `needs_computation` of three is not a majority; the
  pass schema stays flat + closed with the new boolean. Integration: the `server.rs`
  line-count reproduction returns `inconclusive` (never `refuted` at 1.0), and the
  judgment path is unchanged.

## Live dogfood results (T013, 2026-06-14, rebuilt binary)

Run against the running server (the one offline-impossible check — a mock can't
disagree with itself).

- **SC-003 (grounded_verify abstain) — PASS.** `grounded_verify` on
  `src/server.rs is over 1000 lines` (locator `src/server.rs`, the 1224-line file)
  returned `verdict: inconclusive`, `reason: "computable property — route to
  \`check\`"`. Pre-fix this returned`refuted` at confidence 1.0 — the exact bug, gone.
- **SC-001 (verify graduated confidence) — PASS (calibrated battery).** A calibrated
  6-claim battery returned confidence strictly in (0,1) on **2** of them
  (`Water boils at 100°C` → 0.67; `The first day of the week is Sunday` → 0.67),
  meeting the ≥2/6 target. Across two batteries (12 claims) **3** landed at 0.67 (2:1
  lens splits) versus the pre-fix **0/8**. The splits fall exactly where a
  scope/framing lens legitimately disagrees (sea-level-only boiling; US vs ISO week
  start; culinary vs botanical tomato).
- **Calibration note:** a first, *poorly chosen* battery of "X is better than Y"
  opinion claims hit only **1/6** — every lens refutes those identically ("'better' is
  undefined; the absolute claim fails"), so they don't split. The graduated signal is
  real but is elicited by claims contestable *in framing*, not *in opinion*. Pick
  SC-001 batteries accordingly.

**010 is complete:** all offline tasks merged (PR #19), live SC-001/SC-003 confirmed.
