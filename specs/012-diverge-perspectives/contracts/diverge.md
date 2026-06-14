# Contract: Diverge (the `diverge` tool)

A new always-in-catalog tool. Input is a problem statement; output is a server-assembled
set of distinct framings. No verdict, no gate.

## Tool input

```jsonc
{
  "problem": "the framing the caller is anchored on, stated neutrally",  // required, non-empty
  "context": "optional neutral background the passes may consult"        // optional
}
```

- `problem` empty/whitespace or oversize → `invalid_input` before any model call (FR-008).
- `context` is the only extra input a pass sees; there is **no** slot for the caller's
  stance, preferred framing, identity, or history (stance-blindness is structural, FR-005).

## Per-pass constrained-output schema (model-facing, flat + closed)

```json
{
  "type": "object", "additionalProperties": false,
  "required": ["framing", "implication"],
  "properties": {
    "framing": { "type": "string" },
    "implication": { "type": "string" }
  }
}
```

The lens is **not** in the schema — the server assigns `LENSES[i % len]` to pass `i` and
labels the perspective with it (FR-003).

## Tool output (server-assembled)

```jsonc
{
  "perspectives": [
    { "lens": "invert",     "framing": "...", "implication": "..." },
    { "lens": "actor",      "framing": "...", "implication": "..." },
    { "lens": "assumption", "framing": "...", "implication": "..." }
  ],
  "passes": 3
}
```

- `perspectives`: the **deduplicated** set, in pass order, ≤ `k` distinct (FR-004). Near-
  identical framings (token-Jaccard ≥ 0.8) are collapsed, earliest kept.
- `passes`: how many passes completed.
- **No** `verdict`, `confidence`, or chosen step — `diverge` returns framings only
  (FR-007): not `verify`'s job, not `unstick`'s.

## Tool description (routing text — draft)

> Break out of a single framing of a problem. Runs parallel stance-blind passes, each
> attacking the problem from a distinct angle (invert the goal, change whose problem it
> is, shift the time horizon, deny the load-bearing assumption, reframe the problem
> class), and returns a deduplicated set of genuinely different framings — each a one-line
> reframing plus what it changes, labeled with the angle that produced it. Use when you
> are anchored or tunnel-visioned and need real alternatives, not a more confident version
> of the framing you already hold. For judging whether a claim is true use `verify`; for
> committing to one next step use `unstick`.

## Invariants

- The per-pass schema is flat + closed; the output is server-assembled.
- Stance-blind: only `problem` + optional `context` reach a pass.
- Dedup is deterministic and server-side (no model/embedder hop).
- Output is a set of framings — never a verdict or a single committed step.
- Always in the catalog; no env gate (FR-009).
