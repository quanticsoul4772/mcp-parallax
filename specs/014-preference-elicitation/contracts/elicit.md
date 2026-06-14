# Contract: Elicit (the `elicit` tool)

A new always-on tool — the wrong-objective corrective. Input is a task; output is a
server-assembled surfacing of the assumed objective, governing preferences, and divergence
points. No verdict, no enforcement, no gate.

## Tool input

```jsonc
{
  "task": "what you are about to do, stated neutrally",   // required, non-empty
  "context": "optional neutral background"                 // optional
}
```

- `task` empty/whitespace or oversize → `invalid_input` before any model call (FR-008).
- `context` is the only extra caller-prose input. Stored preferences are **server-fetched**
  (recall), never caller-supplied — there is no slot to assert a preference.

## Per-pass constrained-output schema (model-facing, flat + closed)

```json
{
  "type": "object", "additionalProperties": false,
  "required": ["assumed_objective", "preference_texts", "preference_signals",
               "preference_strengths", "divergence_questions", "divergence_signals",
               "signal_level"],
  "properties": {
    "assumed_objective": { "type": "string" },
    "preference_texts": { "type": "array", "items": { "type": "string" } },
    "preference_signals": { "type": "array", "items": { "type": "string" } },
    "preference_strengths": { "type": "array", "items": { "type": "string" } },
    "divergence_questions": { "type": "array", "items": { "type": "string" } },
    "divergence_signals": { "type": "array", "items": { "type": "string" } },
    "signal_level": { "type": "string", "enum": ["low", "medium", "high"] }
  }
}
```

Per-item data is **parallel scalar arrays** (arrays of objects are illegal). The server
validates the three `preference_*` arrays are equal length, the two `divergence_*` arrays
are equal length, and every `preference_strengths` is `"revealed"`/`"stated"`; a mismatch
or bad strength is a failed pass (loud). Empty arrays are valid (low signal).

## Tool output (server-assembled)

```jsonc
{
  "assumed_objective": "Add a caching layer to speed up the endpoint",
  "governing_preferences": [
    { "preference": "Prefers minimal new infrastructure", "signal": "stored memory: avoids new services", "strength": "revealed" },
    { "preference": "Latency target is p99, not average", "signal": "the request mentions 'tail latency'", "strength": "stated" }
  ],
  "divergence_points": [
    { "question": "Is caching the goal, or is the real objective lowering p99 — which a query fix might serve without a cache?",
      "signal": "stored memory: avoids new services conflicts with 'add a cache'" }
  ],
  "signal_level": "medium",
  "memory_consulted": true
}
```

- `governing_preferences` / `divergence_points`: zipped from the parallel arrays; may be
  empty (low signal — the tool does not fabricate).
- `memory_consulted`: true when stored preferences were recalled (memory configured).
- **No** `verdict`, no chosen option, **no action/hold/modify** — surfacing only
  (FR-006/SC-005). Enforcement is `checkpoint_action`'s role.

## Tool description (routing text — draft)

> Surface the objective you're about to pursue and the preferences that should govern it,
> before you commit — the corrective for solving the assumed problem instead of the user's
> real one. Returns the objective a surface reading would assume, the governing
> preferences/constraints (each traced to its signal; revealed/stored ones outrank merely
> stated ones), and the divergence points where the assumed objective likely departs from
> the user's actual one — the questions worth resolving first. Inference, not interrogation:
> with little signal it says so rather than inventing preferences. When memory is
> configured it also consults your stored verified preferences. It surfaces only — it does
> not block or modify anything (that's the checkpoint layer).

## Invariants

- The per-pass schema is flat + closed; the output is server-assembled.
- Stance-blind: only `task` + `context` reach the model as caller prose; stored prefs are
  server-recalled.
- The tool only surfaces — no enforcement field in the output, ever.
- Always in the catalog; no env gate. Memory presence only enriches (and sets
  `memory_consulted`).
