# Contract: `grounded-verify` tool

MCP tool over stdio. Present in the catalog **only** when `GROUNDED_VERIFY_ROOT`
is configured. Sibling of `verify`; distinct from `check`.

## Tool input

```jsonc
{
  "claim": "string — the claim to verify, stated neutrally",
  "locators": [
    { "path": "relative/path/within/root.rs" },
    { "path": "src/server/record.rs", "start_line": 55, "end_line": 111 }
  ]
}
```

- `claim` (required): the only caller prose the passes ever see.
- `locators` (required, non-empty, ≤ `GROUNDED_VERIFY_MAX_LOCATORS`): exact
  paths or path + 1-based inclusive line range. Globs are deferred (v1 rejects
  glob metacharacters as a literal path that won't resolve → named error).

## Tool output (structured, server-assembled)

```jsonc
{
  "verdict": "supported | refuted",
  "confidence": 0.0,                 // cross-pass agreement, 0.0..=1.0
  "findings": ["..."],               // collected across passes
  "missing_evidence": ["..."],       // union across passes; [] when complete
  "manifest": {
    "entries": [
      { "path": "src/server/record.rs", "start_line": 55, "end_line": 111, "bytes": 1840 }
    ]
  }
}
```

## Model-pass schema (constrained output, per pass — flat + closed)

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["verdict", "findings", "missing_evidence"],
  "properties": {
    "verdict": { "type": "string", "enum": ["supported", "refuted"] },
    "findings": { "type": "array", "items": { "type": "string" } },
    "missing_evidence": { "type": "array", "items": { "type": "string" } }
  }
}
```

The model authors only `verdict`, `findings`, `missing_evidence`. `confidence`
and `manifest` are server-assembled and never appear in the model's schema
(FR-012). Array length caps are enforced by the local validator (the API grammar
drops length constraints).

## Errors (all `invalid_params`, naming the offending locator — FR-009, all-or-nothing)

| Condition | Error |
|---|---|
| `locators` empty | `[invalid_input] grounded_verify requires at least one locator` |
| path resolves outside the root (traversal/symlink) | `[invalid_input] locator escapes the source root: <path>` |
| file missing | `[invalid_input] source not found: <path>` |
| file empty (0 bytes) | `[invalid_input] source is empty: <path>` |
| line range out of bounds | `[invalid_input] line range out of range for <path> (<n> lines)` |
| non-text / invalid UTF-8 | `[invalid_input] source is not text: <path>` |
| assembled bytes > ceiling | `[invalid_input] evidence exceeds <max> bytes` |
| locator count > ceiling | `[invalid_input] too many locators (max <max>)` |

Any single failure aborts the whole call; **no verdict** is rendered over a
partial set, and **no model pass runs**.

## Behavior invariants

- The evidence the passes see is exactly the verbatim concatenation of the
  resolved spans — never caller prose beyond `claim`, never model-authored text.
- Same claim + same locators + unchanged files ⇒ identical assembled evidence
  (the assembly step is deterministic; only the passes vary).
- One `InvocationRecord` per call (tool=`grounded_verify`), OTLP-exported when
  telemetry is configured.
