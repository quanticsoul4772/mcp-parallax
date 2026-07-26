# Tool Surface Contract: Per-Call Reasoning Effort

**Feature**: 028 | **Date**: 2026-07-26

The exported surface. Anything not listed here is unchanged, and "unchanged"
includes the request bytes when no argument is supplied.

## Input additions

### `effort` — the seven correctives

Added to `verify`, `unstick`, `diverge`, `decide`, `elicit`, `grounded_verify`,
`check`.

```json
{
  "effort": {
    "type": ["string", "null"],
    "enum": ["low", "medium", "high", "max", "xhigh", null],
    "description": "Reasoning effort for this call alone. Omit to use the deployment's configured level. Not every model family accepts this; when one rejects it the error names the model, the level, and the remedies."
  }
}
```

Optional. Absent means the configured layers decide. Not added to `research`
(which has `depth` and `constraints`), the memory tools (whose model hop is save's trust gate, not the caller's task), or
the `checkpoint_*` tools (harness-triggered).

### `passes` — the three ensemble tools

Added to `verify`, `diverge`, `grounded_verify`.

```json
{
  "passes": {
    "type": ["integer", "null"],
    "minimum": 1,
    "description": "Independent passes to run for this call alone. May be lower than the configured count, never higher. Omit to use the configured count."
  }
}
```

### `constraints.concurrency` — `research`

Added to the existing `Constraints` object beside `max_sources`, `budget_tokens`
and `deadline_ms`.

```json
{
  "concurrency": {
    "type": ["integer", "null"],
    "minimum": 1,
    "description": "Maximum concurrent fetch/extract/verify tasks for this run. Values above the configured ceiling are reduced to it."
  }
}
```

### `recall.limit` — no change

Already present as `Option<u32>` on `RecallParams` with `MEMORY_RECALL_LIMIT` as
its default. Listed here because a confirming test is required (FR-016) and
because it is the prior art this feature generalises.

## Output additions

### `passes` — already present, no new field

`verify`, `diverge` and `grounded_verify` **already** return `passes`: the
number of passes that completed. FR-013 is satisfied by it, so no
`passes_used` field was added — an earlier draft of this contract specified one
before the existing field was found, and duplicating it would have left two
numbers to keep in step.

Note the semantics: `passes` is the count that *completed*, not the count
resolved. That is the correct denominator for a confidence derived from
cross-pass agreement, and it is what a reader needs.

**Always present**, never conditional (FR-013).

**No `effort` field is added to any output.** Effort changes what a call costs,
not how its answer should be read, so it goes on the invocation record instead
(FR-007). The pass count is different in kind: it is the basis for the confidence
in the result itself, so a reader cannot interpret the number without it.

## Record additions

`invocation_records.effort TEXT NULL` — the caller's **override**, or NULL when none was supplied and the configured layers applied.
Additive migration, `depth` pattern. OTLP inherits it, since traces and metrics
derive from the same records.

## Errors

| Condition | Result |
| --- | --- |
| `effort` is not a recognised level | Caller input error naming the field and the accepted values |
| `passes` < 1 | Caller input error |
| `passes` > configured count | Caller input error stating the configured ceiling |
| `concurrency` < 1 | Caller input error |
| `concurrency` > configured ceiling | **No error.** Reduced to the ceiling; the effective value is recorded |
| Provider rejects a valid level for the routed model | 027's enriched error: the model, the level, and both remedies. No boundary pre-check (FR-010) |

## Invariants

1. **Silence is byte-identical.** No argument supplied and nothing configured →
   the outbound body carries no `effort` key at all. The existing wire test that
   fails when the key appears continues to guard this.
2. **No persistence.** A per-call value affects exactly the invocation carrying
   it, and no concurrent invocation.
3. **Model stays operator-owned.** No tool gains a way to name a model or tier.
   The test asserting this is re-grounded to say *model* rather than *routing*,
   so it states what it actually protects (FR-008).
4. **Precedence is total.** Per-call, else site, else tier, else absent — for
   every call site, with no combination unresolved.
