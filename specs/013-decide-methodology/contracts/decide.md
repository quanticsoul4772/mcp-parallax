# Contract: Decide (the `decide` tool)

A new always-on tool. Input is a decision plus ≥2 options; output is a server-assembled
recommendation with its scored rationale. No verdict, no gate.

## Tool input

```jsonc
{
  "decision": "the question to settle, stated neutrally",   // required, non-empty
  "options": ["option A", "option B", "..."],               // required, length >= 2
  "context": "optional neutral criteria/background"          // optional
}
```

- `decision` empty/whitespace or oversize, or `options.len() < 2` → `invalid_input` before
  any model call (FR-008). No fabricated comparison.
- `context` is the only extra subject input; there is **no** slot for the caller's
  preferred option or stance (stance-blind).

## Per-pass constrained-output schema (model-facing, flat + closed)

```json
{
  "type": "object", "additionalProperties": false,
  "required": ["methodology", "option_scores", "option_rationales", "deciding_factors"],
  "properties": {
    "methodology": { "type": "string", "enum": ["weigh", "causal", "probabilistic"] },
    "option_scores": { "type": "array", "items": { "type": "integer" } },
    "option_rationales": { "type": "array", "items": { "type": "string" } },
    "deciding_factors": { "type": "array", "items": { "type": "string" } }
  }
}
```

Per-option data is **parallel scalar arrays** index-aligned to the input `options` (an
array of objects is illegal under the flat-schema gate). The server validates
`option_scores.len() == option_rationales.len() == options.len()`; a mismatch is a failed
pass. The model **does not** name the winner — it only scores.

## Tool output (server-assembled)

```jsonc
{
  "recommended": "option B",
  "runner_up": "option A",
  "runner_up_reason": "scored 30 below option B: higher upfront cost for the same outcome",
  "confidence": 0.65,                       // server-derived from the score margin
  "methodology": "weigh",
  "deciding_factors": ["cost", "time-to-value", "reversibility"],
  "assessments": [
    { "option": "option A", "score": 55, "rationale": "..." },
    { "option": "option B", "score": 85, "rationale": "..." }
  ]
}
```

- `recommended` / `runner_up` = the top two by score; `confidence` = `0.5 + 0.5·min(margin,
  100)/100` (margin = top − runner-up); a tie → `0.5`, a 100-point lead → `1.0`.
- **No** `verdict` (not `verify`), **no** `next_step` (not `unstick`). The choice is
  `argmax(scores)` — deterministic server math, not a model gut call.

## Tool description (routing text — draft)

> Choose among two or more options under tradeoffs, with the reasoning shown. Applies an
> explicit decision methodology (weigh named criteria, trace what each option causes, or
> reason under uncertainty), scores every option, and returns the recommended option, the
> runner-up and why it lost, the deciding factors, the methodology used, and a confidence
> calibrated to how close the call is. The choice is computed from the scores, not asserted
> — never a menu handed back, never a hidden gut pick. For judging whether a claim is true
> use `verify`; for one next step when you're looping use `unstick`; for a computable
> comparison use `check`.

## Invariants

- The per-pass schema is flat + closed (scalar enum + arrays of scalars); the output is
  server-assembled.
- Stance-blind: only `decision` + `options` + optional `context` reach the pass.
- The recommendation is `argmax(scores)` and the confidence is a fixed function of the
  margin — both deterministic, server-side.
- `< 2` options is rejected; output is a recommendation, never a verdict or a next step.
- Always in the catalog; no env gate (FR-009).
