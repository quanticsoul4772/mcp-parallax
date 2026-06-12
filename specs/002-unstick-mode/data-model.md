# Data Model: Unstick Mode

**Date**: 2026-06-12 · **Source**: spec.md Key Entities + research.md D4

## 1. Registry entry

| Field | Value |
|---|---|
| `id` | `"unstick"` |
| `description` | the routing text (catalog does the selection work) |
| `prompt_template` | calibrated single-step profile; placeholders `<<goal>>`, `<<blocked>>`, `<<tried>>` only — blindness is structural, as with verify |
| `output_schema` | schemars-derived from `NextStep` (unsanitized; sanitized form derived at registration) |
| `ensemble_k` | `1` (D1) |

## 2. UnstickParams (tool input)

| Field | Type | Validation |
|---|---|---|
| `goal` | string | required; non-empty after trim (else `invalid_input` before any model call) |
| `blocked` | string | required; non-empty after trim (same) |
| `tried` | array of string, optional | attempts already made; combined input length (goal + blocked + tried) ≤ `VERIFY_MAX_CLAIM_CHARS` (reused as the generic input bound — no new env var) |

## 3. NextStep (tool output — also the per-pass model schema; k=1 so they coincide)

| Field | Type | Grammar-enforced | Code-enforced |
|---|---|---|---|
| `next_step` | string | ✅ single string field — no array for alternatives | non-empty after trim; not a normalized restatement of any `tried` item (case-folded, trimmed, punctuation-insensitive) |
| `rationale` | string | ✅ | — |
| `watch_for` | string \| null | ✅ nullable scalar | — |

Flat, closed, no numeric fields. Violations → `validation_failure`, never a
returned result.

## 4. Invocation record

Unchanged from core (`data-model.md` §5 of 001): the `tool` column now also
takes `"unstick"`. No migration.

## 5. Outcome taxonomy

Unchanged. Unstick maps failures through the identical `AppError → Outcome`
path (FR-005).

## Relationships

```text
Config ────────────────► AnthropicClient (shared)
   │                            ▲ 1 × complete(prompt, sanitized(schema))
   ▼                            │
ModeRegistry ── "verify"  ── verify::run  (k=3 ensemble)   ─┐
            └── "unstick" ── unstick::run (single pass)    ─┤ run_recorded()
                                                            ▼ single exit
                                                      InvocationRecord → Storage
```
