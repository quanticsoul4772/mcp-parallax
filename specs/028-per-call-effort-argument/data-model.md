# Phase 1 Data Model: Per-Call Reasoning Effort

**Feature**: 028 | **Date**: 2026-07-26

## Entities

### Effort level

Already exists as `routing::Effort` — `Low | Medium | High | Max | XHigh`, with
`as_str`/`parse`. Unchanged by this feature.

`Option<Effort>` is the carrier. `None` is a distinct state meaning *send no
effort field*, never a synonym for a level. This is the invariant the existing
byte-identity wire test guards, and it now has to hold one layer higher.

### Effort override

New. `Option<Effort>` supplied by the caller on one invocation.

| Property | Value |
| --- | --- |
| Lifetime | The single invocation that supplied it |
| Default | Absent — the configured layers apply |
| Validation | Must parse to a known level; an unknown string is a typed caller error (FR-006) |
| Persistence | None. It is not stored, cached, or carried to the next call (FR-004) |

### Pass count

Exists as `Mode.ensemble_k: u8`, fixed at registration. Becomes a **default** with
a per-run override.

| Property | Value |
| --- | --- |
| Range | ≥ 1, and ≤ the configured value (FR-012a) |
| Default | `VERIFY_ENSEMBLE_K`, default 3 |
| Reported | Always, on every result that has one (FR-013) |

### Concurrency

Exists as `research::pipeline::Deps.concurrency: usize`, set from
`RESEARCH_CONCURRENCY`. Gains a per-call source inside the existing `Constraints`
struct, beside `max_sources`, `budget_tokens` and `deadline_ms`.

| Property | Value |
| --- | --- |
| Range | ≥ 1 |
| Effective value | `min(requested, configured)` — clamped, never raised (FR-015, D3) |
| Default | `RESEARCH_CONCURRENCY`, default 8 |

## Resolution

Effort resolves most-specific-first across four layers (FR-002):

```text
per-call argument
  └─ else PARALLAX_EFFORT_<SITE>
       └─ else PARALLAX_EFFORT_<TIER>
            └─ else absent  →  no effort field on the wire
```

The lower three layers are already resolved at startup into
`ResolvedRoute.effort`. This feature adds only the top layer, applied at the
point the client is selected:

```text
client = pool.for_site_with_effort(site, per_call_override)
```

The override is passed **bare**, not pre-composed with the configured level.
`None` short-circuits to the site's existing array entry, which guarantees the
silent path returns the identical `Arc` rather than an equal one — composing
first would look up a map entry that merely happens to be configured alike.

Pass count and concurrency each resolve across two layers — per-call, else
configured — with the ceiling applied after resolution.

## Client pool shape (D1)

Today: `by_site: [Arc<dyn ModelClient>; 12]`, built once, one entry per call site.

After: the per-site array is kept for the default path, plus a lookup keyed
`(model, Option<Effort>)` populated eagerly with the full cross product of
distinct routed models × six effort states.

| Path | Lookup | Allocation |
| --- | --- | --- |
| No override | `by_site[site]` — unchanged | none |
| Override | `by_key[(model_for(site), override)]` | none |

Every entry shares one `reqwest::Client`, so the cross product costs a `String`
and an `Option<Effort>` per entry rather than a connection pool. The map is
populated from a total function over a finite domain, so **every lookup hits** —
there is no miss arm to design, which is what keeps `for_site_with_effort`
infallible like `for_site`.

## Storage

`invocation_records` gains one column:

```sql
ALTER TABLE invocation_records ADD COLUMN effort TEXT
```

Additive and pragma-guarded, the same shape as `depth` (019). Semantics:

| Column value | Meaning |
| --- | --- |
| `'low'` … `'xhigh'` | The caller overrode the effort for this invocation |
| `NULL` | No override — the configured layers applied |

The column records **the override, not the effective level** (D4). An invocation
spanning several call sites can use several configured efforts, which one column
cannot represent; the override is single-valued by construction, and it is the
only part of the effort that configuration does not already explain.

NULL is also what rows written before the migration read back as, which is the
truthful answer for them: no override could have been supplied.

`InvocationRecord` gains `effort: Option<String>`, mirroring `depth`. OTLP
inherits it with no separate work, since traces and metrics are derived from the
same records at the same exit points.

## State transitions

None. Every value here is resolved once per invocation and discarded when it
ends. Nothing in this feature has a lifecycle.

## Validation rules

| Rule | Source | Failure mode |
| --- | --- | --- |
| Effort string parses to a known level | FR-006 | Typed caller input error, distinct from a provider rejection |
| Pass count ≥ 1 | existing `validate_ensemble_k` | Caller input error |
| Pass count ≤ configured | FR-012a | Caller input error — a raise is a request the server does not offer |
| Concurrency ≥ 1 | existing config validation | Caller input error |
| Concurrency ≤ configured | FR-015, D3 | **Clamped, not rejected** — the effective value is recorded |

The asymmetry between the last two rows is deliberate and is the one judgment
call in this feature: exceeding the pass count is rejected because each extra
pass is a whole additional model call the caller is asking the operator to buy,
while exceeding concurrency is clamped because it is advice about how to run work
already authorised. D3 records the reasoning and the alternative.
