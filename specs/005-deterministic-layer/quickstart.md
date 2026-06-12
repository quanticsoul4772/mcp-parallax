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

## Inspect

```bash
sqlite3 ./data/parallax.db "SELECT tool, outcome, latency_ms, cost_usd FROM invocation_records WHERE tool = 'check' ORDER BY created_at DESC LIMIT 10;"
```
