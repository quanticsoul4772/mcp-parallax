# Contract: Verification Reliability (verify + grounded_verify output changes)

No tool input changes. Two output/behavior changes.

## `verify` — unchanged surface, real confidence

Input and output shapes are **identical** to 001. The only behavioral change:
the *k* passes now run under distinct lenses, so `confidence` (already
`majority / completed`) takes graduated values on contestable claims instead of
near-constant `1.0`.

```jsonc
// output (unchanged schema)
{ "verdict": "supported|refuted", "findings": ["..."], "confidence": 0.67, "passes": 3 }
```

- `verdict` set unchanged: `supported | refuted` (no `inconclusive` on `verify`).
- Aggregation math unchanged; tie→refuted unchanged.

## `grounded_verify` — `inconclusive` verdict added

Per-pass constrained-output schema gains one flat boolean:

```json
{
  "type": "object", "additionalProperties": false,
  "required": ["verdict", "findings", "missing_evidence", "needs_computation"],
  "properties": {
    "verdict": { "type": "string", "enum": ["supported", "refuted"] },
    "findings": { "type": "array", "items": { "type": "string" } },
    "missing_evidence": { "type": "array", "items": { "type": "string" } },
    "needs_computation": { "type": "boolean" }
  }
}
```

Server-assembled output verdict gains a value:

```jsonc
{
  "verdict": "supported | refuted | inconclusive",   // NEW: inconclusive
  "confidence": 1.0,
  "findings": ["..."],
  "missing_evidence": ["..."],
  "manifest": [ /* 008/009, unchanged */ ],
  "reason": "computable property — route to `check`"   // present when inconclusive
}
```

### When `inconclusive` is returned (server mapping)

| Condition | Verdict | reason |
|---|---|---|
| majority of passes set `needs_computation` | `inconclusive` | computable property — route to `check` |
| aggregated `missing_evidence` non-empty (decisive) | `inconclusive` | decisive evidence missing |
| otherwise | `supported`/`refuted` (008) | — |

### Reproduction (the dogfooded bug)

```jsonc
{ "claim": "src/server.rs is over 1000 lines", "locators": [ { "path": "src/server.rs" } ] }
// 010: => { "verdict": "inconclusive", "reason": "computable property — route to `check`" }
// (was: { "verdict": "refuted", "confidence": 1.0 } on a 1224-line file)
```

## Invariants

- `verify`'s output schema and verdict set are byte-identical to today (FR-009).
- `grounded_verify`'s root-confinement, locators (008/009), and manifest are unchanged.
- The `inconclusive` verdict is server-assembled; model passes never emit it.
