# Contract: Grounded Compute-Settle (grounded_verify pass + output changes)

No tool input changes. One per-pass schema extension and two optional output fields.
The abstain path and the judgment path are byte-identical to 010.

## Per-pass constrained-output schema (extends 010)

Four flat nullable fields added; the model fills them only for an in-class computable
claim (else null). Flat + closed preserved.

```json
{
  "type": "object", "additionalProperties": false,
  "required": ["verdict", "findings", "missing_evidence", "needs_computation",
               "compute_property", "compute_match_literal", "compute_operator",
               "compute_threshold"],
  "properties": {
    "verdict": { "type": "string", "enum": ["supported", "refuted"] },
    "findings": { "type": "array", "items": { "type": "string" } },
    "missing_evidence": { "type": "array", "items": { "type": "string" } },
    "needs_computation": { "type": "boolean" },
    "compute_property": { "type": ["string", "null"], "enum": ["lines", "bytes", "matches", null] },
    "compute_match_literal": { "type": ["string", "null"] },
    "compute_operator": { "type": ["string", "null"], "enum": [">", ">=", "<", "<=", "==", "!=", null] },
    "compute_threshold": { "type": ["integer", "null"] }
  }
}
```

## Server-assembled output (extends 010)

```jsonc
{
  "verdict": "supported | refuted | inconclusive",   // 010 set, unchanged
  "confidence": 1.0,
  "findings": ["counted 1224 lines"],                 // server note on a settle
  "missing_evidence": [],
  "manifest": [ /* 008/009, unchanged */ ],
  "reason": "...",                                    // present only when inconclusive (010)
  "executed_form": "1224 > 1000",                     // NEW: present only when settled
  "engine_result": "true"                              // NEW: present only when settled
}
```

### When each verdict is returned (server mapping)

| Condition | Verdict | extra fields |
|---|---|---|
| not a `needs_computation` majority | `supported`/`refuted` (010 judgment) | — |
| `needs_computation` majority, agreed in-class single-source spec | `supported`/`refuted` (counted + engine) | `executed_form`, `engine_result` |
| `needs_computation` majority, no agreed in-class single-source spec | `inconclusive` (010 abstain) | `reason` |

The compute verdict is settled by `arithmetic::evaluate` over a server-counted value —
never a model estimate, never a model verdict.

### Reproduction (the 010 carry-over)

```jsonc
{ "claim": "src/server.rs is over 1000 lines", "locators": [ { "path": "src/server.rs" } ] }
// 010: => { "verdict": "inconclusive", "reason": "computable property — route to `check`" }
// 011: => { "verdict": "supported", "executed_form": "1224 > 1000", "engine_result": "true" }
```

## Invariants

- `verify`'s schema and verdict set are untouched (its pass schema is separate).
- The abstain path (no agreed in-class single-source spec) and the judgment path are
  byte-identical to 010 — `executed_form`/`engine_result` absent.
- `grounded_verify`'s root-confinement, locators, byte/locator ceilings, and manifest are
  unchanged. The count runs over verbatim source content, never the framed evidence.
- The value is server-counted and the verdict is the engine's; the model only identifies
  the property/operator/threshold.
