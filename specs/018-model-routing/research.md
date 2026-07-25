# Research: Per-Hop Model Routing (018)

Phase 0 decisions. Each records what was chosen, why, and what was rejected.
Facts about the current code were read from the tree at `f9565b7`, not recalled.

## Starting facts (verified in-tree)

- `Config::anthropic_model` is cloned into exactly two places: `AnthropicClient.model`
  (`src/client/anthropic.rs:53`) and `Parallax.model` (`src/server.rs:186`, `:274`).
- `ModelClient::complete(&self, prompt, schema)` carries **no** model parameter
  (`src/traits/client.rs:36`). The model is a property of the client instance.
- `run_recorded(tool_id, model: String, ct, work)` already threads an attributed
  model per tool (`src/server.rs:807`), and the work future returns
  `(T, u64, u64)` — value, input tokens, output tokens.
- Multi-model invocations **already exist**: `surface` and `checkpoint_action`
  attribute to `embedder.model_id()` (Voyage) while other paths attribute to the
  Anthropic model, and the pricing table already carries `voyage-*` rows. The
  existing comment — "Attribution: the embed lookup is the only metered call on
  this path" — is a hand-picked single attribution, which is exactly the pattern
  this feature has to generalize.
- `invocation_records` is a flat 10-column table (`src/storage/sqlite.rs:38`).
- 017 established the pragma-guarded `ALTER TABLE` migration (`sqlite.rs:186`).
- `MAX_TOKENS` is a hard-coded `4096` (`src/client/anthropic.rs:25`), and the
  request body sends no `thinking` field (`:94`).

## D1 — Routing surface: `PARALLAX_MODEL_*`, resolved most-specific-first

**Decision**: a reserved `PARALLAX_MODEL_*` environment namespace holding two tier
settings (`PARALLAX_MODEL_BULK`, `PARALLAX_MODEL_JUDGMENT`) and one optional
per-call-site setting each (`PARALLAX_MODEL_RESEARCH_EXTRACT`,
`PARALLAX_MODEL_VERIFY`, …). Resolution per call site: its own setting, else its
tier's setting, else `ANTHROPIC_MODEL`. Every variable in the namespace whose
suffix is not a known tier or call site is a startup error naming the variable.

**Rationale**: matches the clarified decision (spec Clarifications, 2026-07-24) and
the project's existing config convention — environment only, loud on a
present-but-unusable value, no silent fallback. The reserved namespace is what makes
FR-006a implementable: a typo like `PARALLAX_MODEL_EXTRCT` is *visible* precisely
because the server owns the whole prefix and can reject a suffix it does not know.
Without the namespace rule a misspelled route is unobservable — the operator's only
symptom is a bill that never drops.

**Alternatives considered**: per-call-site only — twelve variables to express the
one common case, rejected at clarification. Tiers only — no escape hatch;
re-tiering one call site becomes a code change. A single JSON-valued variable
(`PARALLAX_MODEL_ROUTES={"verify":"..."}`) — one variable and trivially extensible,
but it breaks the one-setting-one-concern shape every other Parallax variable has,
and a malformed JSON blob produces a worse error than a named unknown suffix.

## D2 — One client per distinct model; the `ModelClient` trait is untouched

**Decision**: build an `Arc<dyn ModelClient>` per **distinct resolved model id** and
hand each call site the one it resolves to. `complete(&self, prompt, schema)` keeps
its exact signature.

**Rationale**: the model is already a property of the client instance, so routing is
a construction-time concern, not a call-time one. Leaving the trait alone means no
existing implementation, no `MockModelClient` expectation, and no test that talks
through the seam has to change — which is what keeps SC-004 ("existing suite passes
without modification to its expectations") achievable rather than aspirational. It
also satisfies Principle IV without spending any of its budget.

**Alternatives considered**: adding a `model: &str` parameter to `complete` — would
touch every implementation, every mock, and every call site, for a value the client
already owns; rejected. A single client that switches model per request — collapses
the pool but makes the client stateful in a way that invites a data race between the
model field and the in-flight request.

## D3 — Per-model usage accumulator replaces the `(u64, u64)` pair

**Decision**: introduce `ModelUsage`, an ordered map from model id to
`(input_tokens, output_tokens)`. The `run_recorded` work future returns
`(T, ModelUsage)`; research's `RunMeter` accumulates into one behind its existing
lock; a one-line constructor covers the eleven single-model call sites.

**Rationale**: this is the smallest change that makes FR-007 true. Cost is only
computable per model, so tokens have to arrive at the record already separated —
summing first and dividing later is not recoverable. Keeping the accumulator ordered
(a `BTreeMap`) makes serialization deterministic, which matters because the JSON
lands in a stored column that tests compare.

**Alternatives considered**: threading a second `(model, tokens)` tuple alongside the
existing pair — works for two models, breaks at three. Recording usage as a side
effect into the storage layer from inside each hop — spreads the write across the
call sites and loses the single-exit-point guarantee that
`InvocationRecord::publish` exists to provide.

## D4 — Additive columns; existing column meanings preserved

**Decision**: `invocation_records` gains two nullable `TEXT` columns, `models` (JSON
array of participating model ids, sorted) and `usage_by_model` (JSON array of
`{model, input_tokens, output_tokens, pricing_known}`), via the pragma-guarded
`ALTER TABLE` pattern 017 established. The four existing columns keep their names
and their meaning: `input_tokens`/`output_tokens` are the sums, `cost_usd` is the
sum of per-model costs, and `model` is the attributed model (D5).

**Rationale**: preserves the one-record-per-invocation invariant the clarification
settled on, so every existing query, the 001 record contract, and the 007 span model
keep working untouched. Rows written before this feature have `NULL` in the new
columns and read as single-model records, which is the documented assumption. Making
the columns nullable rather than defaulted avoids inventing a JSON value for history
that was never measured.

**Alternatives considered**: one row per call site — rejected at clarification; it
would break the invariant, multiply row counts, and force a revisit of every
per-invocation query. A separate `invocation_model_usage` child table — properly
normalized, but it turns every read of a record into a join for a payload that is
one to three entries, and SQLite JSON columns are already the project's idiom for
bounded structured detail (`checkpoint_records.signals_fired`).

## D5 — Attributed model = dominant by **measured tokens**, not estimated cost

**Decision**: the record's `model` column and the span's `gen_ai.request.model` carry
the participating model with the greatest `input + output` tokens; ties break
lexicographically. When usage is empty — a cancelled or failed invocation, which
records zero tokens today — the attribution falls back to the model the tool was
entered with, exactly as now.

**Rationale**: single-valued attribution is still needed because
`parallax.invocations` is a counter keyed by model and adding once per participating
model would double-count invocations. Dominance must be computed from something
measured, not something estimated: FR-012 permits `cost_usd` to be a conservative
fallback for an unknown model, so a cost-dominant rule could hand the headline to a
model that merely lacks a price row. Token counts come from the provider response
and carry no such distortion. In the single-model case — every invocation today, and
every invocation for an operator who configures nothing — dominance is trivially the
only model, so the column is byte-identical to what the server writes now (SC-004).

**Alternatives considered**: cost-dominant — better matches a spend dashboard's
intent, but derives attribution from an estimate that may itself be a fallback.
Entry-call-site attribution — stable and explainable, but for `research` the entry
hop (scope) is the cheapest of four and would be a misleading headline. A literal
`"multiple"` sentinel — honest, but destroys the metric dimension for every routed
invocation.

## D6 — Telemetry: split the per-model instruments, keep one span

**Decision**: amend the 007 contract as follows. The span stays one per invocation
and keeps `gen_ai.request.model` (now the D5 attributed model), gaining
`parallax.models` (string[], the full sorted set) and `parallax.cost_estimated`
(bool, true when any participating model priced off the fallback). Metrics:
`parallax.cost` and `gen_ai.client.token.usage` are recorded **once per
participating model** with that model's own attribute; `parallax.invocations` and
`parallax.invocation.duration` are recorded once per invocation, using the attributed
model.

**Rationale**: `parallax.cost` and `gen_ai.client.token.usage` are already keyed by
`gen_ai.request.model`, so splitting them per model is what those instruments always
meant — the sum is unchanged and the granularity is new. The counters that measure
*invocations* must not be split or they stop counting invocations. Every instrument
is byte-identical in the single-model case, which is what lets FR-002 and SC-004 be
tested rather than asserted. `parallax.cost_estimated` is how FR-012's
"distinguishable from a known price" requirement reaches an operator who reads
telemetry instead of the database.

**Alternatives considered**: child spans per call site — the natural home for
per-model detail, but 007 explicitly deferred intra-invocation child spans and
un-deferring that is a much larger contract change than this feature needs. Dropping
`gen_ai.request.model` from `parallax.invocations` — removes the double-count problem
by removing the dimension, losing per-model invocation counts operators have today.

## D7 — One request shape for every family: omit `thinking`, raise the output budget

> **Deferral discharged by 022 (2026-07-25), under a different mechanism.** This
> decision deferred per-family thinking suppression "to be decided on measured
> cost". Suppression is still not sent: `thinking: {"type": "disabled"}` remains
> rejected by Fable 5, so omitting the field is still the one universally
> accepted shape. The supported control turned out to be
> `output_config.effort`, which governs all output tokens including thinking and
> which every routed family accepts. 022 exposes it per call site, off by
> default. The "measured cost" condition now applies to choosing levels, which
> 022 deliberately leaves to the operator rather than shipping a recommended
> default on evidence nobody has. See `specs/022-per-call-site-effort/`.

**Decision**: raise `MAX_TOKENS` from 4096 so a verdict cannot be truncated by
reasoning that shares the budget, and continue to send no `thinking` field. Raise
`REQUEST_TIMEOUT_MS`'s default in step with it.

**Rationale**: the families disagree about `thinking` in a way that admits exactly
one universally-accepted shape. Claude 5 models reason by default and charge that
reasoning against `max_tokens`, so 4096 can be consumed before the JSON verdict is
emitted — the failure mode is `AppError::Truncation` on a request that looks valid.
Opus 5 and Sonnet 5 would accept `thinking: {"type": "disabled"}`, but Fable 5
rejects it with a 400 at any effort, so suppression cannot be the single shape.
Omitting the field is accepted everywhere. The timeout has to move with the budget:
a larger output ceiling on a thinking-by-default model can exceed the current 30 s
default, converting a truncation into a timeout rather than fixing it.

**Measurement procedure** *(added 2026-07-24 after `/speckit-analyze` flagged "measured
against a real call" as unfalsifiable)*. The budget has a deterministic floor and an
empirical ceiling, and only the ceiling needs a live call:

1. **Answer floor — no network required.** The largest mode schema bounds its own
   output: research synthesis allows 8 000 answer characters plus ten gaps of 500
   (`MAX_ANSWER_CHARS`, `MAX_GAPS`, `MAX_GAP_CHARS` in `src/research/mod.rs`), so
   ~13 000 characters ≈ **~3.5k tokens** of answer before any reasoning. Every other
   mode schema is smaller. This is computed from the schemas, not observed.
2. **Provisional budget.** Set `MAX_TOKENS` to at least **4× the floor**, leaving the
   remainder for adaptive thinking on families that reason by default.
3. **Provisional timeout.** Set the `REQUEST_TIMEOUT_MS` default to at least **3× the
   slowest single call** observed while establishing the budget.
4. **Acceptance is the family sweep (T050), not a separate harness.** The values are
   correct when the sweep produces **zero `AppError::Truncation` and zero
   `AppError::Timeout`** outcomes across one run of each tool on each completion
   family. If either appears, raise the offending value and re-run the sweep.

This makes both tasks falsifiable — the floor is checkable by arithmetic, and the
acceptance criterion is an outcome count over work the plan already schedules. Steps 2
and 3 set provisional values that step 4 validates; that is an iteration, not a
circular dependency.

**Alternatives considered**: a per-family capability table driving `thinking`
suppression — measurably cheaper on the judgment tier, since suppressing adaptive
thinking on Opus 5 avoids reasoning tokens on a flat verdict schema, but it adds a
family table that must track provider behavior and a branch that can be wrong per
family. **Named deferral**: per-family thinking suppression is deferred to a
follow-up, to be decided on measured cost rather than prediction. This is a
deliberate, named narrowing under Principle I, not a silent omission — the feature
still satisfies FR-013 and FR-014, just not at the cheapest possible price.

## D10 — The client pool lives in `src/client/pool.rs`, not in routing or the server

*(Added 2026-07-24 after `/speckit-analyze` raised module placement as a Principle VII
pressure point.)*

**Decision**: a new `src/client/pool.rs` owns pool construction — a function from the
resolved model ids plus `&Config` to a `BTreeMap<String, Arc<dyn ModelClient>>`.
`src/routing.rs` stays pure config logic with no client dependency, and `src/server.rs`
gains only the call, the per-call-site `Arc` hand-off, and the startup table.

**Rationale**: `src/server.rs` is 1397 lines today. Principle VII scopes its ≤500-line
target to *new* modules, so adding to it is not a violation, but four tasks piling onto
the largest file in the tree is exactly the signal the principle says to read. Routing
is the wrong home for the opposite reason: D2 justifies that module by its purity —
resolution is testable with no client, no network, and no config beyond the environment
— and having it construct `AnthropicClient` would trade that away for nothing. The pool
is a client-layer concern, so it belongs beside the client. The module is small and has
one job: dedupe model ids and build one client each.

**Alternatives considered**: pool in `server.rs` — fewest files, but grows the file the
analysis flagged and mixes "how to build a client" into the composition root. Pool in
`routing.rs` — one fewer module, but inverts the dependency and makes the pure routing
tests need a client fixture. Splitting `server.rs` more aggressively — out of scope
under Principle VII's scope discipline; this feature is not a refactor.

## D8 — Pricing table refresh, and how "unknown price" becomes visible

**Decision**: add the current model rows — `claude-opus-5` (5.00/25.00),
`claude-sonnet-5` (3.00/15.00), `claude-fable-5` (10.00/50.00) — and return a
`pricing_known` flag alongside the rate lookup. The flag is stored per model in
`usage_by_model` and aggregated onto the span as `parallax.cost_estimated`.

**Rationale**: FR-011 requires the list to cover what the server ships knowing about,
and the gap is not theoretical — `claude-fable-5` bills at double the Opus-tier
fallback, so an operator routing to it today would see spend under-reported by half.
`claude-opus-5` happens to match the fallback exactly, which is precisely why the
flag matters: without it, "correct by coincidence" and "correct by lookup" are
indistinguishable. The conservative Opus-tier fallback itself is kept unchanged, so
an unrecognised future model still over-reports rather than under-reports (FR-012).

**Alternatives considered**: rejecting unknown model ids at startup — would make
every price knowable, but breaks routing to any model the provider ships after this
build, contradicting the spec's forward-compatibility edge case. Loading prices from
a config file — removes the maintenance task from the release cycle but adds a file
format, a parse path, and a new class of startup failure for a table that changes a
few times a year.

## D9 — Startup routing table on the diagnostic stream

**Decision**: one `tracing::info!` event at startup, after config resolution and
before the server serves, listing every call site with its resolved model and the
setting that supplied it (per-site override, tier, or the default).

**Rationale**: implements FR-005 and makes SC-007 checkable. stderr is the only legal
sink (Principle III — stdout is the JSON-RPC channel), and structured `tracing` is
the established form. Emitting before serving means a misconfiguration is visible
before any spend, which is the point of the requirement.

**Alternatives considered**: a diagnostic MCP tool — rejected at clarification;
it would add a catalog entry aimed at the operator while the catalog is read by the
model. Logging only the non-default routes — shorter, but an operator confirming
"did my route take effect" is exactly the person who needs to see the full resolved
table, including the sites that fell through to the default.
