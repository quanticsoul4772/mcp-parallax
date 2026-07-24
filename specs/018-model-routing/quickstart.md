# Quickstart: Per-Hop Model Routing (018)

## Do nothing

Routing is off until you set something. With no `PARALLAX_MODEL_*` variable, every
call site uses `ANTHROPIC_MODEL` and the server behaves exactly as it did before this
feature — same costs, same records, same responses. Upgrading forces no migration.

## The one change worth making

Research's per-source claim extraction runs once for every fetched page and dominates
a run's token volume, but the work is transcription. Point it at a cheap model:

```jsonc
// the parallax entry's "env" block
"PARALLAX_MODEL_BULK": "claude-haiku-4-5"
```

Everything else — verification, deciding, eliciting, the checkpoint review — stays on
`ANTHROPIC_MODEL`. Restart the server and check the startup line on stderr:

```text
parallax: routing resolved (12 call sites, 2 distinct models)
  research_extract     claude-haiku-4-5  PARALLAX_MODEL_BULK
  verify               claude-opus-5     ANTHROPIC_MODEL
  ...
```

If a route you set is missing from that table, you set a name the server does not
know — and the server will have refused to start and told you which one.

## Confirming it saved money

Run the same research question before and after, then compare recorded cost:

```sql
SELECT tool, model, models, cost_usd, cost_usd
FROM invocation_records
WHERE tool = 'research'
ORDER BY created_at DESC LIMIT 2;
```

`models` lists every model that participated. `cost_usd` is the sum across them, each
priced at its own rate — not one model's rate applied to every token, which is what
made the pre-018 number wrong the moment an invocation spanned models.

For the per-model split:

```sql
SELECT json_extract(value, '$.model')         AS model,
       json_extract(value, '$.input_tokens')  AS input_tokens,
       json_extract(value, '$.output_tokens') AS output_tokens,
       json_extract(value, '$.pricing_known') AS pricing_known
FROM invocation_records, json_each(invocation_records.usage_by_model)
WHERE invocation_records.id = ?;
```

`pricing_known = 0` means that model has no price row and was costed at the
conservative Opus-tier fallback — the figure is an over-estimate, not a measurement.

## Overriding a single call site

Tiers cover the common case; override one site when you disagree with its tier:

```jsonc
"PARALLAX_MODEL_BULK": "claude-haiku-4-5",
"PARALLAX_MODEL_RESEARCH_SYNTHESIZE": "claude-sonnet-5"
```

Resolution is most-specific-first: call site, then tier, then `ANTHROPIC_MODEL`.

## What routing does not do

- **No fallback between models.** If the model you routed to is throttled or
  unavailable, that call site fails and the layer above degrades as it always has —
  research drops and counts the source, the checkpoint layer falls silent, a
  corrective returns the error. Work is never quietly re-run on the model you routed
  away from, so a provider outage cannot hand you a surprise bill.
- **No caller control.** Nothing the calling model sends can change which model
  answers. A judge the caller could choose is a judge the caller could shop for.
- **No change to any tool's request or response.** Routing is invisible from the
  caller's side.

## If you route to a Claude 5 model

Those families reason before answering unless told not to, and that reasoning shares
the output budget with the answer — so a budget sized for a bare verdict can be spent
before the JSON is emitted.

Both ceilings rose in this feature, and the output budget is derived rather than
guessed:

| Setting | Was | Now | Where the number comes from |
|---|---:|---:|---|
| output budget (`MAX_TOKENS`, compiled in) | 4 096 | 16 000 | The largest mode schema bounds its own output: research synthesis allows 8 000 answer characters plus ten 500-character gaps — ~13 000 characters, roughly **3 500 tokens** of answer before any reasoning. The budget is ≥4× that floor. |
| `REQUEST_TIMEOUT_MS` | 30 000 | 120 000 | A ceiling four times larger on a model that reasons first can outrun 30 s, which would turn a truncation into a timeout rather than fix it. |

The floor is computed from the schema constants in a test, not hard-coded, so the
relationship survives a schema change. The timeout figure is **provisional** until the
family sweep confirms zero timeouts across the shipped completion models.

If you still see truncation or timeouts after routing a call site to one of these
families, raise `REQUEST_TIMEOUT_MS` before suspecting the route.
