# Contract: `PARALLAX_EFFORT_*` environment surface

**Feature**: 022 | **Date**: 2026-07-25

Extends the `PARALLAX_MODEL_*` contract in
[`specs/018-model-routing/contracts/config.md`](../../018-model-routing/contracts/config.md).
Same shape, same precedence, different property.

## Variables

| Form | Example | Scope |
|---|---|---|
| `PARALLAX_EFFORT_<CALL_SITE>` | `PARALLAX_EFFORT_VERIFY` | One call site |
| `PARALLAX_EFFORT_<TIER>` | `PARALLAX_EFFORT_BULK` | Every call site in that tier |

Call-site and tier suffixes are exactly those of the model namespace:
`VERIFY`, `UNSTICK`, `DIVERGE`, `DECIDE`, `ELICIT`, `GROUNDED_VERIFY`,
`CHECK_TRANSLATE`, `RESEARCH_SCOPE`, `RESEARCH_EXTRACT`, `RESEARCH_VERIFY`,
`RESEARCH_SYNTHESIZE`, `CHECKPOINT_REVIEW`; tiers `BULK` and `JUDGMENT`.

## Values

`low` · `medium` · `high` · `max` · `xhigh` — case-insensitive, surrounding
whitespace ignored.

**Absent is not `high`.** An unset call site sends no `effort` field at all. The
provider's own default is `high`, so the two behave alike, but only unset is
*provably* unchanged on the wire — which is what Principle VI requires of a new
capability.

## Resolution

Most specific wins, and **model and effort resolve independently**:

```text
PARALLAX_EFFORT_<SITE>  →  PARALLAX_EFFORT_<TIER>  →  unset (no field sent)
```

A call site may take its model from a tier and its effort from its own variable,
or either from the default. Collapsing the two lookups would mean the bulk tier
could not carry a cheap effort without also naming a model.

## Validation

Both are startup errors that name the offending variable:

- a name in the `PARALLAX_EFFORT_*` namespace that matches no call site or tier;
- a value outside the five levels, including empty or whitespace. The message
  lists the accepted levels.

A setting that silently does nothing is worse than a refusal to start: the
operator believes a call site changed when it did not.

## Client pooling

One client per distinct **`(model, effort)`** pair, not per model. Two call
sites on one model at different efforts need separate clients — sharing one
would send the first site's effort on the second's calls. Two sites agreeing on
both share one client, as they did before this feature.

## Request body

`effort` joins `format` under `output_config`; it never replaces it.

```jsonc
// unset — byte-identical to pre-022
"output_config": { "format": { "type": "json_schema", "schema": { … } } }

// PARALLAX_EFFORT_RESEARCH_EXTRACT=low
"output_config": { "format": { … }, "effort": "low" }
```

## Interaction with `MAX_TOKENS`

None. Effort is a behavioural signal about how much reasoning to spend; the
output ceiling is unchanged and still bounds thinking plus answer together.
