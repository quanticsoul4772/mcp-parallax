# Data Model: Per-Hop Model Routing (018)

## 1. Call sites and tiers (`src/routing.rs`, new)

The twelve routable call sites and their fixed tier membership. Tier membership is
compiled in; only the model each tier uses is operator-set.

| Call site id | Tier | Env suffix |
|---|---|---|
| `verify` | judgment | `PARALLAX_MODEL_VERIFY` |
| `unstick` | judgment | `PARALLAX_MODEL_UNSTICK` |
| `diverge` | judgment | `PARALLAX_MODEL_DIVERGE` |
| `decide` | judgment | `PARALLAX_MODEL_DECIDE` |
| `elicit` | judgment | `PARALLAX_MODEL_ELICIT` |
| `grounded_verify` | judgment | `PARALLAX_MODEL_GROUNDED_VERIFY` |
| `check_translate` | judgment | `PARALLAX_MODEL_CHECK_TRANSLATE` |
| `research_scope` | judgment | `PARALLAX_MODEL_RESEARCH_SCOPE` |
| `research_extract` | **bulk** | `PARALLAX_MODEL_RESEARCH_EXTRACT` |
| `research_verify` | judgment | `PARALLAX_MODEL_RESEARCH_VERIFY` |
| `research_synthesize` | judgment | `PARALLAX_MODEL_RESEARCH_SYNTHESIZE` |
| `checkpoint_review` | judgment | `PARALLAX_MODEL_CHECKPOINT_REVIEW` |

```text
Tier        = Bulk | Judgment                  (snake_case on the wire/log)
CallSite    = enum of the twelve above         (stable string ids)
RouteSource = Site | Tier | Default            (which setting supplied the model)
ResolvedRoute { site: CallSite, model: String, source: RouteSource }
RoutingTable { routes: [ResolvedRoute; 12] }   (complete; every site always resolves)
```

Resolution is a pure function of the environment — no I/O, no model call:

```text
resolve(site) = env("PARALLAX_MODEL_" + site.suffix)   -> (model, Site)
             |= env("PARALLAX_MODEL_" + site.tier)     -> (model, Tier)
             |= config.anthropic_model                 -> (model, Default)
```

**Validation (all at startup, all loud — FR-006, FR-006a):**

- A `PARALLAX_MODEL_*` variable whose suffix is neither a tier nor a call-site id is
  a `ConfigError` naming the variable. This is the only defense against a misspelled
  route, so the check scans the environment rather than only reading known names.
- A recognised variable present but empty or whitespace-only is a `ConfigError`
  naming the variable and its value. Never a fallback to the default.
- Nothing else is validated: an unrecognised *model id* is legal (D8) and prices
  conservatively.

## 2. Client pool (`src/server.rs` construction)

```text
ClientPool { by_model: BTreeMap<String, Arc<dyn ModelClient>> }
```

Built from `RoutingTable`: one `AnthropicClient` per **distinct** model id, so two
call sites routed alike share one client (FR-004). Each call site's dependency
struct holds the `Arc` it resolved to. The `ModelClient` trait is unchanged (D2) —
`ResearchDeps.model_client` becomes four fields (one per research call site) rather
than gaining a model parameter.

## 3. Per-model usage (`src/telemetry.rs`)

```text
Usage      { input_tokens: u64, output_tokens: u64 }
ModelUsage { by_model: BTreeMap<String, Usage> }      // ordered => deterministic JSON
```

Constructors and operations:

- `ModelUsage::single(model, input, output)` — the eleven single-model call sites.
- `ModelUsage::add(&mut self, model, input, output)` — accumulate; research's
  `RunMeter` wraps one behind its existing lock.
- `totals() -> (u64, u64)` — the sums that fill the existing columns.
- `dominant() -> Option<&str>` — greatest `input + output`, ties lexicographic (D5).
- `cost_usd() -> (f64, bool)` — summed cost, plus whether every model priced off a
  known row (`false` => at least one fell back).

`RunMeter::total()` keeps summing across all models: research budgets and deadlines
are denominated in tokens, not currency, so ceiling behavior is unchanged.

## 4. Invocation record (`src/telemetry.rs`, `src/storage/sqlite.rs`)

Two nullable columns added by pragma-guarded `ALTER TABLE` (017's pattern):

```sql
ALTER TABLE invocation_records ADD COLUMN models         TEXT;  -- JSON string[]
ALTER TABLE invocation_records ADD COLUMN usage_by_model TEXT;  -- JSON object[]
```

```text
InvocationRecord {
  id, session_id, tool,                        -- unchanged
  model:          String,   -- ATTRIBUTED model (D5): dominant by measured tokens
  input_tokens:   u64,      -- unchanged meaning: sum across models
  output_tokens:  u64,      -- unchanged meaning: sum across models
  cost_usd:       f64,      -- now sum over models of that model's own rate (FR-008)
  models:         Vec<String>,      -- NEW, sorted, participants only (FR-015b)
  usage_by_model: Vec<ModelUsageRow>,-- NEW
  cost_estimated: bool,     -- NEW (derived, not stored separately): any fallback price
  latency_ms, outcome, created_at    -- unchanged
}

ModelUsageRow { model: String, input_tokens: u64, output_tokens: u64,
                pricing_known: bool }
```

Read-back rules:

- A row written before this feature has `models`/`usage_by_model` `NULL`. It reads as
  a single-model record whose one participant is the `model` column — no backfill.
- A single-model invocation writes `models = ["<the model>"]` and one usage row, and
  its four legacy columns are byte-identical to what the server writes today
  (FR-009a, SC-004).
- Only models that actually ran appear (FR-015b); a call site that never executed and
  a call site whose model failed both contribute nothing.

## 5. Telemetry surface (amends `specs/007-observability-layer/contracts/telemetry.md`)

Span `parallax.{tool}` — still exactly one per invocation:

| Attribute | Change |
|---|---|
| `gen_ai.request.model` | unchanged name; now the D5 attributed model |
| `parallax.models` | **new** string[] — every participant, sorted |
| `parallax.cost_estimated` | **new** bool — a participating model priced off the fallback |
| everything else | unchanged |

Metrics:

| Instrument | Recording change |
|---|---|
| `parallax.cost` | once **per participating model**, that model's own cost and `gen_ai.request.model`. Sum unchanged. |
| `gen_ai.client.token.usage` | once **per participating model** (the instrument is already model-keyed) |
| `parallax.invocations` | unchanged — once per invocation, attributed model (splitting would stop it counting invocations) |
| `parallax.invocation.duration` | unchanged |
| `parallax.checkpoint.*` | unchanged |

Every instrument is byte-identical in the single-model case.

## 6. Pricing (`src/telemetry.rs`)

`PRICING_PER_MTOK` gains `claude-opus-5` (5.00/25.00), `claude-sonnet-5`
(3.00/15.00), `claude-fable-5` (10.00/50.00). The lookup returns
`(input_rate, output_rate, pricing_known)`; `FALLBACK_PRICING` stays Opus-tier and
sets `pricing_known = false` so an unknown model over-reports rather than
under-reports (FR-012).

## 7. Request shape (`src/client/anthropic.rs`)

`MAX_TOKENS` rises from 4096; no `thinking` field is sent (D7). `REQUEST_TIMEOUT_MS`'s
default rises with it, because a larger budget on a thinking-by-default family can
outrun the current 30 s ceiling and convert a truncation into a timeout. Concrete
values are set during implementation against a measured call, not guessed here.
