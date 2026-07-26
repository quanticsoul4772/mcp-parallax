# Quickstart: Per-Call Reasoning Effort

**Feature**: 028 | **Date**: 2026-07-26

## For the calling model

Spend more reasoning on a claim that warrants it:

```json
{ "claim": "…", "effort": "max" }
```

Spend less on one that does not:

```json
{ "claim": "…", "effort": "low", "passes": 1 }
```

Nothing is edited and nothing is restarted. Omit the arguments and behaviour is
exactly what it was — the deployment's configured levels apply, and if none are
configured the request carries no effort field at all.

Two rules worth knowing:

- **`passes` may go down, not up.** Each pass is a whole model call, so raising
  the count spends the operator's budget on work they did not authorise. Asking
  for more than the configured count is an error naming the ceiling.
- **`research`'s `constraints.concurrency` is clamped, not rejected.** Asking for
  more than the ceiling gets you the ceiling and the run proceeds.

Every result from `verify`, `diverge` and `grounded_verify` states `passes` —
the field already existed; 028 makes the number the caller can influence. Read it before reading `confidence`: confidence comes from
cross-pass agreement, so a confidence over one pass is not the same claim as a
confidence over three.

## For the operator

Nothing to configure. `PARALLAX_EFFORT_*` still sets the default for any call
where the caller says nothing, and it is still off by default.

What changes is that spend is no longer fully predictable from your
configuration — a caller can raise effort on a single call. The invocation record
is where that becomes visible: a row carries the level a **caller** overrode, so
a cost configuration does not explain is attributable without reproducing the
call.

```sql
SELECT tool, model, effort, cost_usd, created_at
FROM invocation_records
ORDER BY cost_usd DESC
LIMIT 20;
```

`NULL` means no override was supplied — the configured layers applied, at
whatever level the startup routing table prints. Rows written before this
feature also read `NULL`, which is the truthful answer for them.

If you want the old guarantee back, there is no switch for it. That is the
deliberate tradeoff: the control exists because the caller is the one who knows
what a task warrants, and the ceilings on `passes` and `concurrency` are where
the operator's authority is preserved.

## Verifying 027 while you are here

027's rejection diagnosis could not be exercised without editing configuration
and restarting. Now:

```json
{ "decision": "…", "options": ["a", "b"], "effort": "low" }
```

on a call site routed to a model that rejects the parameter returns an error
naming the model, the level, and both remedies — no config edit, no restart, one
cheap call.
