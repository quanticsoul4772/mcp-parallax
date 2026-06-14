# Data Model: Diverge — Independent Perspectives

In-memory only; no persistence change. New entities for the `diverge` mode.

## Lens (internal)

A named generative perspective assigned to a `diverge` pass.

| Field | Type | Notes |
|---|---|---|
| name | `&'static str` | `invert`, `actor`, `horizon`, `assumption`, `class`. |
| directive | `&'static str` | The instruction paragraph injected at the `<<lens>>` slot. |

- A fixed `LENSES: &[Lens]` array (research D1).
- Assignment: pass *i* uses `LENSES[i % LENSES.len()]` (research D2).
- The prompt template has a `<<lens>>` slot plus `<<problem>>` / `<<context>>` — the only
  subject inputs (stance-blindness, FR-005).

## DivergePass (per-pass constrained output) — flat + closed

What each pass is grammar-constrained to produce. Two strings; nothing for the sanitizer
to strip (Constitution II). The **lens is not a model field** — the server assigns it by
pass index and labels the perspective (FR-003).

| Field | Type | Notes |
|---|---|---|
| framing | string | the one-line reframing of the problem under this pass's lens |
| implication | string | what this framing changes / its key consequence |

- Validation: a pass with an empty/whitespace `framing` is a failed pass (not a
  perspective), mirroring verify's "refutation without a finding" rule.

## Perspective (server-assembled)

One returned framing — a `DivergePass` labeled with the lens that produced it.

| Field | Type | Notes |
|---|---|---|
| lens | string | the assigned lens name (server-labeled, FR-003) |
| framing | string | from the pass |
| implication | string | from the pass |

## DivergeResult (tool output, server-assembled)

| Field | Type | Notes |
|---|---|---|
| perspectives | `Vec<Perspective>` | the deduplicated set, in pass order (≤ k distinct) |
| passes | u32 | number of passes that completed |

- Server-assembled; not grammar-constrained (the output may be nested — like
  `grounded_verify`'s manifest). No verdict, no confidence (Diverge does not converge).

## Dedup rule (server, pure, deterministic) — research D4

Over the completed passes' perspectives, in pass order:

1. **Normalize** each `framing`: lowercase, strip punctuation, collapse whitespace → a
   token set.
2. A later perspective is a **near-duplicate** of an earlier kept one when their token-set
   **Jaccard similarity ≥ `DEDUP_THRESHOLD`** (a named constant, `0.8` in v1).
3. Keep the earlier (lower pass index) perspective; drop the later. The kept set is
   returned in pass order — stable, independent of map iteration.

- Empty input (zero completed passes) → no perspectives; the run returns the dominant
  failure class instead (research D5). A single completed pass → that one perspective.

## Aggregation (server) — collect, not vote (research D5)

`diverge` does **not** use `verify::aggregate_core`. After the `k` passes:

1. Collect each completed pass's `(lens, framing, implication)` into a `Perspective`.
2. If **zero** completed → return the dominant failure class.
3. Else dedup (the rule above) and return `DivergeResult { perspectives, passes }`.

No quorum, no majority, no confidence — a scatter tool has no minority to protect.

## Configuration

No new variables. The pass count reuses `VERIFY_ENSEMBLE_K` (or the registry's
per-mode `ensemble_k`); `DEDUP_THRESHOLD` is a code constant. No new gate (FR-009).
