# Data Model: Grounded Compute-Settle

In-memory only; no persistence change. Extends the 010 grounded entities.

## GroundedPass (extended) — per-pass constrained output

010's pass schema gains four **flat nullable** fields. The model fills them only when it
sets `needs_computation` and the claim is an in-class computable comparison; otherwise
they are null. Flat + closed preserved (nullable scalars and scalar enums are the shapes
`assert_flat` already admits).

| Field | Type | Notes |
|---|---|---|
| verdict | enum `supported`\|`refuted` | unchanged (010) |
| findings | string[] | unchanged |
| missing_evidence | string[] | unchanged (advisory, 010) |
| needs_computation | boolean | unchanged (010) — the abstain/compute trigger |
| **compute_property** | enum `lines`\|`bytes`\|`matches`, nullable | **NEW** — what to count; null unless an in-class computable claim |
| **compute_match_literal** | string, nullable | **NEW** — the literal to count; only with `matches` |
| **compute_operator** | enum `>`\|`>=`\|`<`\|`<=`\|`==`\|`!=`, nullable | **NEW** — the comparison the claim asserts |
| **compute_threshold** | integer, nullable | **NEW** — the numeric bound |

## ComputeSpec (server-internal, US1)

The aggregated, agreed computation a settle runs. Not a schema; built by the server from
the agreeing passes' fields.

| Field | Type | Notes |
|---|---|---|
| property | `Property { Lines, Bytes, Matches(String) }` | the in-class property + literal for `matches` |
| operator | `Op { Gt, Ge, Lt, Le, Eq, Ne }` | from `compute_operator` |
| threshold | `i64` | from `compute_threshold` |

- Built only if a **majority of the `needs_computation` passes** carry an identical,
  complete, in-class spec (same property, operator, threshold, and literal for
  `matches`). Disagreement or any missing/out-of-class field → no spec → abstain.

## AssembledEvidence (extended)

`assemble` today returns `{ text, manifest }` and discards the raw per-unit content. It
gains the raw content of the read units so the count runs over verbatim source, not the
header-framed `text`:

| Field | Type | Notes |
|---|---|---|
| text | String | unchanged — the header-framed evidence the passes judge |
| manifest | Vec`<ManifestEntry>` | unchanged (008/009); `bytes` per entry already present |
| **units** | Vec`<RawUnit { text: String, bytes: u64 }>` | **NEW** — verbatim per-read-unit content, in order |

- **Single-source gate**: the compute path engages only when `units.len() == 1`.

## Counting (server, pure)

Over the single raw unit's `text` (verbatim source):

- `Lines` → number of lines. **Convention**: count `\n` occurrences, plus one more if
  the content is non-empty and does not end in `\n` (a final unterminated line counts).
  Empty content → 0. Pinned by tests (LF-terminated and no-trailing-newline files).
- `Bytes` → the unit's `bytes` (the reader's byte length; already on the manifest).
- `Matches(lit)` → count of non-overlapping occurrences of `lit` in the raw `text`;
  empty literal is out-of-class → abstain.

## Settle (server) — reuse the engine

Construct `format!("{value} {op} {threshold}")` (e.g. `"1224 > 1000"`) and call
`deterministic::arithmetic::evaluate`:

- `Ok(ArithmeticOutcome { holds, result_text })` → verdict `supported` if `holds` else
  `refuted`; `executed_form` = the expression; `engine_result` = `result_text`.
- `Err(Violation)` (should not occur for a counted-int comparison, but total handling) →
  abstain with `inconclusive` (no verdict over an unsettled comparison).

## GroundedVerdict (extended output)

010's server-assembled output gains two **optional** fields, present only on a settled
compute verdict:

| Field | Type | Notes |
|---|---|---|
| verdict | `GroundedVerdictKind` | unchanged (010): `supported`/`refuted` on settle, `inconclusive` on abstain |
| ... (confidence, passes, findings, missing_evidence, manifest, reason) | | unchanged (010) |
| **executed_form** | string, optional | **NEW** — the engine's decided comparison (e.g. `1224 > 1000`); absent unless settled |
| **engine_result** | string, optional | **NEW** — the engine's raw result; absent unless settled |

On a settled verdict, `findings` carries a one-line server note naming the property and
counted value (e.g. "counted 1224 lines"); `confidence` is reported as `1.0` (a settled
deterministic result, not an agreement ratio). On abstain, the output is byte-identical
to 010.

## Aggregation (server) — full order

After 010's pass aggregation and the `needs_computation`-majority check:

1. Not a `needs_computation` majority → 010 judgment verdict (`supported`/`refuted`).
2. `needs_computation` majority, but no agreed in-class single-source `ComputeSpec`
   (disagreement, out-of-class property, missing field, or `units.len() != 1`) →
   `inconclusive` (010 abstain, route to `check`).
3. `needs_computation` majority **and** an agreed in-class single-source `ComputeSpec`
   → count → `arithmetic::evaluate` → `supported`/`refuted` with `executed_form` +
   `engine_result`.

## Configuration

No new variables. `grounded_verify` stays gated on `GROUNDED_VERIFY_ROOT`; the byte and
locator ceilings (008) are unchanged.
