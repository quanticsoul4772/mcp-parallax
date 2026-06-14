# Data Model: Verification Reliability

In-memory only; no persistence change. Extends the verify/grounded entities.

## Lens (US1, internal)

A named critical perspective assigned to a `verify` pass.

| Field | Type | Notes |
|---|---|---|
| name | `&'static str` | e.g. `literal`, `counterexample`, `definitional`, `evidential`, `scope`. |
| directive | `&'static str` | The instruction paragraph injected at the `<<lens>>` slot. |

- A fixed `LENSES: &[Lens]` array (research D1).
- Assignment: pass *i* uses `LENSES[i % LENSES.len()]` (research D2).
- The prompt template gains a `<<lens>>` placeholder; `claim`/`context` remain the
  only inputs about the subject (stance-blindness, D3).

## PassVerdict (`verify`, unchanged) / GroundedPass (extended)

`verify`'s per-pass schema is **unchanged**: `{ verdict: supported|refuted,
findings[] }`.

`grounded_verify`'s per-pass schema gains one flat boolean (flat + closed preserved):

| Field | Type | Notes |
|---|---|---|
| verdict | enum `supported`\|`refuted` | unchanged |
| findings | string[] | unchanged |
| missing_evidence | string[] | unchanged (008) |
| **needs_computation** | boolean | **NEW** — set when the claim's truth hinges on an exact computation of the source the pass cannot perform by reading (a precise count/measure). |

## GroundedVerdictKind (output, NEW)

`grounded_verify`'s server-assembled output verdict — distinct from the shared
per-pass `VerdictKind`.

`Supported | Refuted | Inconclusive`

- `Inconclusive` carries a short `reason` (computable → route to `check`; or decisive
  evidence missing).
- `verify`'s output verdict is **not** changed — it stays `{ supported, refuted }`
  with graduated confidence (FR-009).

## Aggregation (server)

`verify`: unchanged — majority, tie→refuted, dedup from majority side,
confidence = majority/completed, quorum `⌈k/2⌉`. Only the *inputs* (lensed prompts)
differ.

`grounded_verify` (after the existing aggregation):

1. If a **majority** of completed passes set `needs_computation` → `Inconclusive`
   (reason: computable property; route to `check`).
2. Else if the aggregated `missing_evidence` is non-empty (decisive) → `Inconclusive`
   (reason: decisive evidence missing).
3. Else → the majority `Supported`/`Refuted` (008 behavior), with the
   agreement-derived confidence.

## Configuration

No new variables. `VERIFY_ENSEMBLE_K` (default 3, quorum) unchanged; it now selects
how many lenses run.
