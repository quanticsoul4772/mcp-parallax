# Contract: research result additions

Present only when `PARALLAX_JEV_MODE_RESEARCH_INJECTION=on`; omitted otherwise, so off and shadow results are byte-identical to today's.

Per source:

```json
"jev_injection": {
  "screened": true,
  "probability": 0.91,
  "warned": true,
  "reason": null
}
```

- `reason` is one of `timeout`, `classifier_provider`, `provider_rejected`, `validation_failure`, `redaction_failed`, `disabled` when `screened` is false, otherwise null.
- `warned` is true only when `screened` is true and `probability` ≥ the pinned threshold (0.35 for `jev-1.14.0`).

Stats:

```json
"jev_warned_pages": 1,
"jev_unscreened_pages": 0,
"jev_warned_list": [{ "id": "s1", "url": "https://example.com/a" }]
```

- `jev_warned_list` lists every kept page Jev warned on, cited or not, like 063's `flagged_pages`.
- 063's `injection_patterns`, `pages_flagged` and `flagged_pages` are unchanged in every mode.
- The tool description states that the warning is advisory, that "no warning" does not mean clean, and the measured rates at the threshold.
- The output schema marks every addition optional; a schema test binds the field names to this contract.
