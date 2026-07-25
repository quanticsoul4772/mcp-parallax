# Contract Change: `research` tool output

**Feature**: 021 | **Date**: 2026-07-25

Amends `specs/004-research-layer/contracts/research.tool.json`. That file is the
authority; this records the delta and why each part of it is what it is.

## Additive — three new fields

Added to `outputSchema.properties` and to `required`.

```jsonc
"coverage": {
  "type": "number", "minimum": 0, "maximum": 1,
  "description": "Proportion of the run's scoped sub-questions that were settled. Equals the fraction of sub_question_status entries with settled=true, or 1 when the run scoped none (the list is then empty and nothing is unsettled)."
},
"refutation_rate": {
  "type": "number", "minimum": 0, "maximum": 1,
  "description": "Proportion of verified claims that verification refuted. 0 when no claim was verified."
},
"sub_question_status": {
  "type": "array",
  "items": {
    "type": "object",
    "properties": {
      "sub_question": { "type": "string" },
      "settled":      { "type": "boolean" }
    },
    "required": ["sub_question", "settled"],
    "additionalProperties": false
  },
  "description": "Each sub-question the run scoped and whether it was settled. The basis for `coverage`, published so the figure is checkable from the output."
}
```

Nested objects are permitted **here** and forbidden in the synthesis schema. This is
the MCP tool contract, which already nests `key_findings`, `disagreements`, `sources`
and `stats`. Principle II's flat-and-closed rule governs *mode* schemas — what the
model is constrained to emit — and `sub_question_status` is server-assembled. The
synthesis hop's own addition stays flat (see below).

## Redefinition — `confidence`

| | |
|---|---|
| Before | `"Verification- and coverage-grounded confidence (0..=1)."` |
| After | `"Support established for the claims the answer asserts (0..=1) — the mean confidence of the published findings. Not reduced by unsettled sub-questions; see the coverage field for breadth. Zero only when no claim was supported."` |

Type and range are unchanged; the value a given run produces changes. This is the one
part of the change a caller cannot detect by shape alone, which is why the two
companion figures are published rather than kept internal — the appearance of
`coverage` beside it is the visible signal that the field moved.

The previous value stays recoverable as `confidence * coverage` (SC-005).

## Additive — a new `stats.stop_reason` value

`malformedsynthesis` joins the enum. A synthesis whose `gaps` and `gap_targets`
disagreed in length on both attempts demotes under this reason rather than
inheriting `grounding` — the grounding gate is never reached on that path, and
reporting it as a grounding failure tells the caller the answer could not be
cited when citation was never evaluated (004 FR-007).

## Unchanged — `gaps`

Stays `array of string`. Rejected during clarification: making each gap an object
carrying its sub-question. That is a breaking type change for any caller reading gaps
as text, and gaps raised by the grounding gate have no sub-question, so the field
would have to carry a false one.

## Internal — the synthesis mode schema

Not part of the published contract; recorded here because it is the mechanism.

```jsonc
// prompts::SynthOut — flat and closed (Principle II)
{
  "answer":       { "type": "string" },                              // unchanged
  "gaps":         { "type": "array", "items": { "type": "string" } },// unchanged
  "gap_targets":  { "type": "array", "items": { "type": "integer" } }// NEW
}
```

`gap_targets[i]` is the 1-based sub-question that `gaps[i]` concerns; `0` is none.
Index-aligned parallel arrays, the idiom `decide` already uses. Length equality and
range are checked at assembly, since JSON Schema cannot state a cross-array relation
— the same reason `decide` checks its arity in code.

## Compatibility summary

| Change | Class | Caller impact |
|---|---|---|
| `coverage`, `refutation_rate`, `sub_question_status` added | Compatible | A caller ignoring them is unaffected |
| `confidence` redefined | **Behavioural** | Same type and range, different value. Documented in the description; signalled in the output by the new fields |
| `gaps` unchanged | None | — |
| `gap_targets` on the synthesis hop | Internal | None |
