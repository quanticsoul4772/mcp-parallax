# Contract: decide pass format (model-facing)

The format sent to the model for a `decide` call with N options. Example for
N = 3, before sanitizing:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": [
    "methodology", "deciding_factors",
    "option_01_score", "option_01_rationale",
    "option_02_score", "option_02_rationale",
    "option_3_score", "option_3_rationale"
  ],
  "properties": {
    "methodology": { "type": "string", "enum": ["weigh", "causal", "probabilistic"] },
    "deciding_factors": { "type": "array", "items": { "type": "string" } },
    "option_01_score": { "type": "integer" },
    "option_01_rationale": { "type": "string" },
    "option_02_score": { "type": "integer" },
    "option_02_rationale": { "type": "string" },
    "option_3_score": { "type": "integer" },
    "option_3_rationale": { "type": "string" }
  }
}
```

Rules:

- One score and one rationale field per position 1..=N, all required; no other
  per-option fields.
- One format per N, 2 through 10, registered at startup; each passes
  `assert_flat`.
- The caller-facing `decide` contract (`specs/013-decide-methodology/contracts/decide.tool.json`)
  keeps its output schema; only the description and the `options` note change
  ("two to ten options").
