# Phase 0 Research: Per-Call Reasoning Effort

**Feature**: 028 | **Date**: 2026-07-26

Four decisions. D1 is the one the spec flagged as central and deliberately left
open.

## D1 — How a per-call effort reaches the HTTP client

**Decision**: pre-build **every `(distinct routed model, effort level)`
combination** eagerly at startup, over **one shared `reqwest::Client`**, and
expose `ClientPool::for_site_with_effort(site, override) -> Arc<dyn ModelClient>`.
With no override the existing per-site array is returned unchanged.

**Rationale**:

The spec named two options and asked the plan to pick one. Reading the code
showed a third that dominates both.

The bound is small and knowable at startup. Effort has exactly six states — five
levels plus absent — and the routed model set is at most twelve and in practice
one to four. The full cross product is therefore ≤72 clients and realistically
under 12. An `AnthropicClient` is a `reqwest::Client` plus three small fields;
once the HTTP client is shared, each additional entry costs a `String` and an
`Option<Effort>`.

That collapses the fork the spec posed:

| Option | Trait seam | Per-call cost | Verdict |
| --- | --- | --- | --- |
| Completion seam gains a parameter | Broken — ~30 call sites plus every mock | none | Rejected: reverses 018 D2 for no gain over the chosen option |
| Client built per call on the override path | Preserved | one client + one connection pool per call | Rejected: pays forever for a bound that is small and static |
| **Eager cross product, shared HTTP client** | **Preserved** | **none — an array/map lookup** | **Chosen** |

The lazy-cache variant (build on miss, memoise under a lock) was also considered
and rejected: it buys nothing over eager construction when the full set is this
small, and it costs interior mutability, a lock on the hot path, and a factory
closure retained in the pool. Eager construction keeps `ClientPool` a plain
immutable value, which is what makes `for_site` infallible today.

**Alternatives considered**: the two the spec named (above); the lazy memoising
cache (above); threading effort through mode structs instead of the pool
(rejected — the mode would have to hold a client factory, pushing routing
knowledge into every mode, which is what the pool exists to prevent).

**Consequence for `AnthropicClient`**: it currently calls `reqwest::Client::new()`
per construction, so each of the ≤72 entries would own a separate connection
pool. A constructor taking an existing `reqwest::Client` is required. This is an
improvement independent of this feature — today's 1–4 clients already duplicate
connection pools unnecessarily.

## D2 — How a per-call pass count reaches a mode

**Decision**: the pass count moves from a value fixed at registration to a value
resolved per run. `Mode.ensemble_k` (`src/modes/mod.rs:39`) stays as the
**default**; the run path takes `Option<u8>` and resolves override-else-default.

**Rationale**: registration happens once at startup, so a count baked in there
cannot vary per call by construction. Resolving at run is the minimal change that
makes it vary, and it keeps the configured value as the fallback exactly as the
env layer already behaves for effort.

The resolved count is then returned in the result (FR-013). Because it is
reported unconditionally, the same code path produces it whether or not the
caller supplied one — there is no conditional branch whose untested arm could
report the wrong number.

**Alternatives considered**: re-registering modes per call (absurd — registration
builds schemas); a separate "ensemble override" side-channel (more surface, same
effect).

## D3 — How the concurrency ceiling is enforced

**Decision**: clamp the caller's value to the configured ceiling —
`effective = min(requested, configured)` — and record the effective value.

**Rationale**: FR-015 permits lowering and forbids raising. Two enforcements are
possible and the difference is user-visible:

- **Clamp**: a caller asking for more than the ceiling gets the ceiling. The call
  proceeds. Simple, and matches how `max_sources` already behaves against tier
  caps.
- **Reject**: a caller asking for more than the ceiling gets a typed error. Louder,
  and arguably more honest.

Clamp is chosen because raising concurrency is not a *request that fails*, it is a
request the server is entitled to decline while still doing the work — the caller
asked for the research, and the concurrency was advice about how to run it. A
rejection would fail an otherwise valid research run over a performance hint.

**The tension is named, not hidden.** Principle III forbids fallbacks that hide
failures, and a silent clamp resembles one. Two things keep it on the right side:
the ceiling is *specified behaviour* the caller can read in the contract, not an
error being swallowed; and the effective value is on the record, so **the
operator** can see what ran. If review disagrees, the switch to rejection is a
small, local change and this entry is the place to revisit.

**The caller is deliberately not told, and that is where concurrency differs from
the pass count.** Concurrency is a hint about how to run work already authorised;
it does not change what the answer means, so a caller that got eight instead of
sixteen holds the same result it would otherwise have held. The pass count *is*
the basis for the confidence, so a caller not told the count would misread the
number. FR-013 reports one and this decision does not report the other for that
reason — not by oversight, and not because the two were treated inconsistently.

**Alternatives considered**: reject (above); honour the caller and rely on the
operator noticing (rejected — that is exactly the egress giveaway FR-015 exists
to prevent).

## D4 — How the effort reaches the invocation record

**Decision**: one nullable `effort TEXT` column on `invocation_records`, added by
the same pragma-guarded additive `ALTER TABLE` used for `depth`
(`src/storage/sqlite.rs:275`), recording **the per-call override only**. NULL
means no override was supplied and the configured layers applied (FR-007a).

**Scope: the override, not the effective level.** An invocation that fans out
across call sites can use several *configured* efforts — a `research` run spans
four independently routable sites, so `PARALLAX_EFFORT_RESEARCH_SCOPE=high` with
`PARALLAX_EFFORT_BULK=low` is two efforts inside one invocation. One column
cannot represent that. The record already solved the same problem for models with
`models: Vec<String>` and `usage_by_model` (`telemetry.rs:197-199`), and the
first draft of this decision overlooked it by asserting that a fanned-out
invocation uses a single effort — true of an override, false of the configured
layers. Recording the override alone is single-valued by construction, and it is
exactly the quantity the record needs to carry: the configured levels are
constant for the deployment and already printed in the startup table.

**Rationale**: this is the fourth migration of exactly this shape (017, 018, 019),
so the pattern is proven and rows written before it read back as `None` rather
than a guessed default. Telemetry needs no separate work: OTLP traces and metrics
are derived from the same records at the same exit points, which is the property
`specs/007-observability-layer/contracts/telemetry.md` requires so the two
surfaces cannot disagree.

**Alternatives considered**: a per-site `efforts` collection mirroring `models`
(rejected — a bare list is ambiguous without site keys, and keying it
reintroduces nesting to carry values the operator configured and can already
read); a JSON blob of "call parameters" (rejected — unqueryable, and the existing
columns set the precedent); recording effort per model call rather than per
invocation (rejected — it would store the configured level repeatedly for every
call a phase makes).

## Cross-cutting: what stays untouched

- **The `ModelClient` trait.** D1 preserves it, so 018 D2 stands and every mock
  compiles unchanged.
- **The env namespaces.** `PARALLAX_EFFORT_*` remains the default layer. This
  feature adds a layer above it and removes nothing.
- **The silent path.** With no argument supplied and nothing configured, the
  request body is byte-identical to today (FR-003). The existing wire test that
  fails if an `effort` key appears keeps guarding it.
