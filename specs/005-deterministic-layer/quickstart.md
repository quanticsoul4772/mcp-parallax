# Quickstart: Deterministic Layer

## Enable

Nothing to enable — `check` is always in the catalog. The engines run
in-process with no network, no filesystem, and no code execution; the only
credential involved is the existing `ANTHROPIC_API_KEY` (translation).

## Use

```json
// arithmetic — true claim
{ "claim": "A 37% reduction from 1840 ms leaves about 1159 ms." }

// arithmetic — false claim
{ "claim": "2^32 is about 2.1 billion, so it fits in a signed 32-bit integer." }

// constraints — impossibility assertion (expect a witness if it's wrong)
{ "claim": "You cannot seat A, B and C in a row such that A is left of B, B is left of C, and C is left of A." }

// honest decline
{ "claim": "Rust is more elegant than C++." }
```

Every verdict carries `formal_form` and `engine_result` — audit what was
actually executed. A refuted impossibility claim carries the solver's
`witness`. `not_checkable` is the honest outcome for judgment calls; route
those to `verify`.

## Spike (no key needed — gates the solver dependency)

```bash
cargo run --example spike_z3     # S1: bundled build time + sat/unsat/witness/timeout round trip
```

## Acceptance (live; needs ANTHROPIC_API_KEY only)

```bash
cargo run --release --example acceptance_check
```

≥ 20 ground-truth claims (SC-001 100% verdict accuracy), ≥ 6 uncheckable
claims (SC-002 100% declined), auditability (SC-003), determinism (SC-007).
Results recorded below when run.

### Results (2026-06-12, claude-opus-4-8)

Four live runs; the first three each exposed a translation-quality defect
that was fixed at the engine/prompt level (never by widening the verdict
mapping):

- **Run 1 — FAIL, 16/21.** Two confidently wrong refutations from exact
  `==` over float-producing arithmetic (`0.15 * 240 == 36` is false in
  f64), and three constraint claims with inverted polarity (`asserted` set
  to what the model computed, not what the claim states). Fix: the
  arithmetic engine now rejects `==`/`!=` in float-producing expressions
  as a retryable violation forcing the tolerance form (pure-integer
  equality stays exact).
- **Run 2 — FAIL, 16/21 on 19 completed.** The equality guard recovered
  one claim via the violation-fed retry; two claims double-failed because
  the guard message gave no concrete rewrite, and the same three polarity
  errors remained (a prompt edit had silently failed to apply). SC-002
  7/7, SC-003 19/19, SC-007 true.
- **Run 3 — FAIL, 17/21 on 18 completed.** Polarity cues landed: all
  constraint claims correct. But the abstract guidance ("use a tolerance")
  made three exact-value claims double-fail and one get wrongly declined.
- **Run 4 — PASS.** Guard message quotes the rejected expression and names
  a literal bound (`<= 0.0001`); prompt states exact percentage/power
  claims are checkable. SC-001 21/21, SC-002 7/7 declines, SC-003 21/21
  responses carry `formal_form` + `engine_result`, SC-007 determinism
  true (forms varied across the repeat; engine results identical).

## Inspect

```bash
sqlite3 ./data/parallax.db "SELECT tool, outcome, latency_ms, cost_usd FROM invocation_records WHERE tool = 'check' ORDER BY created_at DESC LIMIT 10;"
```
