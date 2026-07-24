# Changelog

All notable changes to mcp-parallax are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Per Keep a Changelog 1.1.0, this is
the next-up change block; it persists verbatim until the project's next SemVer
cut, at which point the entries move into a dated `## [X.Y.Z] - YYYY-MM-DD`
block and the `[Unreleased]` header starts the next arc. Rolls up the
post-#38-merged work on `main` (#38–#42). The agent doesn't carry `ANTHROPIC_API_KEY`,
so live-dogfood freshness is not re-fired in this arc; the #42 stamps
therefore read "Mechanism re-verified" rather than "Re-verified" (see the
*Docs* entry below for the rationale).

### Added

* **Per-call-site model routing (018)** — each of the server's twelve model
  call sites can run on a model chosen for the work it does, instead of all
  twelve sharing one `ANTHROPIC_MODEL`. Two work-kind tiers (`bulk`, holding
  only research extraction — the one call site whose volume scales with the
  number of fetched sources — and `judgment`, holding the other eleven) over a
  reserved `PARALLAX_MODEL_*` namespace, with per-call-site overrides resolved
  most-specific-first. The namespace is validated as a whole: an unrecognised
  suffix is a startup error naming the variable, which is what makes a
  misspelled route visible rather than a bill that silently never drops. A
  resolved routing table is logged to stderr before serving. `ModelClient`
  keeps its exact signature — the model stays a property of the client
  instance, so routing is construction-time and the process holds one client
  per distinct resolved model. **Off by default**: unset means the pre-018
  behavior, byte-identical costs included.

  Cost accounting became per-model as a consequence: once one invocation can
  span models, a single rate applied to summed tokens is wrong under either.
  An invocation still produces exactly one audit record and one exported span;
  the record gains `models` and `usage_by_model` (two nullable columns via a
  pragma-guarded `ALTER TABLE`; pre-existing rows read back as the
  single-model records they were, no backfill), and `cost_usd` becomes the sum
  over participants at each one's own rate. The attributed model — the record's
  `model` column and the span's `gen_ai.request.model` — is the participant with
  the most **measured tokens**, deliberately not the most cost, because an
  unpriced model falls back to Opus-tier rates and would otherwise win
  attribution by merely lacking a price. `parallax.cost` and
  `gen_ai.client.token.usage` are now recorded once per participating model;
  the invocation counters are not split, since that would stop them counting
  invocations. The 007 telemetry contract is amended in-change.

  Pricing gains the Claude 5 rows. This was not theoretical: `claude-fable-5`
  bills at $10/$50 against an Opus-tier fallback of $5/$25, so a deployment
  routed there was under-reporting spend by half. A new `pricing_known` flag
  distinguishes a looked-up price from the fallback — necessary precisely
  because `claude-opus-5` matches the fallback exactly, making "correct by
  lookup" and "correct by coincidence" otherwise indistinguishable.

  The per-call output budget rises from 4 096 to 16 000 tokens, derived from
  the largest mode schema's own bounds (~3 500 tokens of answer) at ≥4×, with
  the request timeout raised alongside it: on Claude 5 families, omitting
  `thinking` runs adaptive reasoning charged against the same ceiling, so the
  old budget could truncate a verdict before its JSON was emitted. The request
  shape stays family-agnostic — no `thinking` field, which is the one form
  every family accepts. **Named deferral**: per-family thinking suppression,
  to be decided on measured cost (research D7). No tool's input or output
  schema changes, and no caller-supplied value influences which model answers.
  `SDK_LANDSCAPE.md` amended in-change; artifacts under
  `specs/018-model-routing/`.
* **Memory consolidation and auto-capture (017)** — the write-path half of
  the memory layer. Supersession and merge run on admission: a
  deterministic cosine screen (0.75) gates one budgeted decline-biased
  judgment; updates supersede (status change with attribution, never
  deletion), near-duplicates merge at ≥ 0.90 to a byte-identical survivor
  under a trust guard that doubles as the promotion path
  (re-admission). Decay is ranking-only via a reinforcement-refreshed
  recency clock. The end-of-turn review hop gains a third judgment:
  capture — a demonstrably working approach or diagnosed failure may
  propose one candidate memory per turn (capped 2/session), stored
  untrusted/quarantined and never auto-promoted; with memory configured
  the turn hop now runs every turn end. First `ALTER TABLE` migration
  (pragma-guarded, fixture-tested); new `memories.status` dimension
  filtered to active across every retrieval path; new
  `consolidation_records` audit table mirrored to OTLP. All three spec
  clarifications decided via `decide` under the margin protocol.
  `MEMORY_LAYER.md` amended in-change; artifacts under
  `specs/017-memory-consolidation/`.
* **Push memory (016)** — the push half of `MEMORY_LAYER.md`'s "effortless,
  not manual" contract: a new harness-triggered, memory-gated `surface`
  tool (invoked by an installable `UserPromptSubmit` hook) surfaces the few
  most relevant trusted stored memories into the assistant's context at
  each turn start as clearly-labeled advisory context (verbatim content +
  memory id + trust + a `forget(<id>)` contestability pointer).
  Deterministic end-to-end — no model pass; relevance floor 0.55 / cap 3;
  once-per-session suppression derived from the feature's own audit rows;
  hard 500 ms fail-open budget; new `push_records` audit table mirrored to
  OTLP. Memory-off behavior unchanged; nothing fires until the hooks
  integration is installed. All three spec clarifications were decided via
  `decide` under the order-bias experiment's margin protocol. Spec/plan/
  contracts under `specs/016-push-memory/`.
* **decide order-bias experiment** (`claudedocs/experiments/decide-order-bias/`):
  pre-registered test of the design corpus's "permute order" judge-bias clause
  against the shipped single-pass `decide` — 250 live calls over 70 fixture
  decisions with an identical-order retest arm as the noise floor, including
  a power extension the `decide` tool itself selected (dogfooded, with a
  permuted confirmation pass). Final result: **no order bias at any tested
  k** — 2 options 5%/5% (measured null), 4 options pooled n=40 18.8%/17.5%
  (p=0.51; the interim 30%-vs-10% directional effect was refuted with
  power). Durable findings: sampling instability dominates four-option
  near-ties (17.5% identical-order flips), and the score margin encodes all
  instability — every flip of any kind sat at margin ≤ 16, margin ≥ 17 was
  perfectly stable across the whole experiment. Corpus §4 amended
  in-change; margin-gated permutation is rejected as a feature.
* **Preference enforcement at the end-of-turn checkpoint (015).** The
  `checkpoint_turn` review hop now judges the turn — final message wording
  plus observable in-turn activity — against recalled **trusted** stored
  preferences (the same trusted lesson/fact population the action gate
  treats as constraints) and flags a violation quoting the stored
  preference verbatim with its provenance (memory id, trust standing), so
  the model can revise or explicitly contest it. One hop still (the two
  judgments share the layer's single model pass), flag-only authority
  (never hold, never rewrite), fail-open, cooled down by memory id, and
  byte-identical behavior when memory is unconfigured. New
  `preference_violation` signal kind on checkpoint records; no new tools,
  config, or storage schema. Closes the capture → store → recall →
  **enforce** loop from `PREFERENCE_ELICITATION.md` (amended in-change);
  spec/plan/contracts under `specs/015-preference-enforcement/`.

### Security / Dependencies

Three transitive advisories cleared via three lockfile-only commits (#38).

* `quinn-proto` 0.11.14 → 0.11.16 — high, CVSS 7.5 (RUSTSEC-2026-0185).
  Transitively pulled in via `reqwest`'s `http2` feature. Pulled in
  `chacha20 0.10.1`, `cpufeatures 0.3.0`, and a second `rand 0.10.x` major
  (parallel to the existing `rand 0.9.x` already in the lockfile).
* `anyhow` 1.0.102 → 1.0.104 — unsound `anyhow::Error::downcast_mut`
  (RUSTSEC-2026-0190). Transitively pulled in via `prost-derive` → `prost`
  → `opentelemetry-otlp`.
* `spin` 0.9.8 → 0.9.9 — yanked. Transitively pulled in via `flume`
  → `sqlx`.

Zero `Cargo.toml` changes; three sequential commits, each pinning a
single advisory to its dep bump.

### Fixed

* **Routed call sites recorded the wrong model, and were priced at its rate
  (018).** Routing itself worked — a call went to the client its call site
  resolved to — but the eleven single-model tools passed
  `config.anthropic_model` to `run_recorded` as the attributed model, so the
  invocation record named the server-wide default and `cost_usd` was computed
  at that model's rate. A `verify` routed to Sonnet 5 ($3/$15) was billed in
  the record at Opus 4.8 rates ($5/$25). Found by running 018's own T050
  validation sweep against the merged binary. `Parallax` now carries the
  resolved `RoutingTable` and every call site attributes through it; the
  now-redundant global `model` field is removed, which is how the compiler
  confirms no site was missed. Research was already correct — its `RunMeter`
  attributes per hop through the routing table.

  This is the same defect class `/speckit-analyze` caught for the checkpoint
  layer before 018 merged: attribution taken from global config rather than the
  resolved route. That instance was fixed; the eleven identical cases in the
  same file were not. The regression test this shipped without —
  `a_routed_call_site_records_the_model_that_served_it` — asserts that a routed
  tool's record names the routed model, and was confirmed to fail against the
  original bug before being kept.

* **Research verification judged from priors, not evidence (004/D3+D4).**
  The per-claim refute-biased verifier received only source titles/hosts as
  context — the fetched page text was dropped after extraction — so
  "default to refuted when support cannot be established" made verdicts a
  function of the judging model's training cutoff: a live run refuted
  "Rust 1.97.0 was released 2026-07-09" while the official announcement sat
  fetched in its own source list (found by the tool-catalog validation
  sweep). `SourceRecord` now retains the capped readable text internally,
  new `research/evidence.rs` deterministically selects a claim-relevant
  excerpt (word-overlap anchored, ≤3 sources × ≤4 000 chars), the verify
  context carries the excerpts, and the template directs the judge to test
  the claim against them rather than its memory. Nothing new reaches the
  wire (FR-012 re-asserted in tests). The scope prompt also gains a
  same-named-referent disambiguation rule (the sweep's second finding: an
  angle drifted to the RUST video game). 004 D3/D4 amended in-change.
* **Batch-screen false positive on distinct-target batches (006/D5).**
  `normalize_input` no longer drops an input-level `id` as volatile: it
  names the action's semantic target (for `forget` it is the entire
  payload), so dropping it made a finite batch of six distinct deletions
  normalize identically and fire a false repetition flag (017 T019 live
  dogfood finding). Retry loops on the *same* target still match and
  still fire — recall is unchanged; the harness's genuinely volatile ids
  (`tool_use_id`, `session_id`, `request_id`) stay dropped. Ground-truth
  table extended with both directions; `006` data-model amended in-change.

### Changed

* **OTel GenAI semconv deprecations cleared (#39, `eeb1608`).** The
  upstream `opentelemetry_semantic_conventions` crate deprecated its
  `attribute::GEN_AI_*` constants in the 0.32 train; CI's `-D warnings`
  turned the deprecations into 13 hard errors at `src/observability.rs`
  and 4 more at `examples/spike_otlp.rs`. The five canonical
  attribute-name strings (`gen_ai.operation.name`, `gen_ai.request.model`,
  `gen_ai.token.type`, `gen_ai.usage.input_tokens`,
  `gen_ai.usage.output_tokens`) are now declared locally — mirroring the
  existing `GEN_AI_PROVIDER_NAME` precedent at `src/observability.rs:37`.
  Stable OTel spec identifiers; the test assertions in
  `src/observability.rs` and `tests/integration.rs` already verify
  against them as raw string literals, so the local consts and the wire
  format share one source of truth.
* **`unstick` tolerates client `blocked`-field arg-drop (#40,
  `0ad8fdc`).** Some MCP clients intermittently drop the `blocked` field
  from the emitted tool-call while `goal` survives. `UnstickParams`
  gains `#[serde(default)]` on `blocked` (advertised optional in the
  contract: `required: ["goal"]`) and a `normalize()` pass at the top of
  `run()` that recovers `blocked` from a `||BLOCKED|| <text>` marker
  appended to `goal`. Unconditional marker strip prevents prompt-leak
  on dual-encoded calls; multi-marker robustness takes only the first
  post-marker segment so a retry-encoded client does not leak the marker
  literal into the recovered `blocked`. Four new `modes::unstick` tests
  cover dropped/missing recovery, dual-encoding, no-marker idempotence,
  and multi-marker first-segment-only.

### Docs

* **`CLAUDE.md` Active-Feature staleness pruned (#41, `a62e47e`).** Two
  stale notes pointed at a "needs the rebuilt binary" precondition and
  an "uncommitted at last check" reminder from before the unstick work
  landed. Both removed now that the rebuilt binary is shipping and the
  unstick work is on its own PR.
* **Dogfood mechanism re-verification stamps (#42, `500b5a6`).**
  Three-line diff in `specs/012-diverge-perspectives/tasks.md` (T013),
  `specs/013-decide-methodology/tasks.md` (T010), and
  `specs/014-preference-elicitation/tasks.md` (T012) — adds a
  "Mechanism re-verified 2026-07-20" sub-bullet below each existing
  inline 2026-06-14 live result. The mode source is unchanged across
  #38–#41, so the 2026-06-14 live `SC-*` results stay structurally held
  (model + `FR-*` contract unchanged); the offline integration suite
  (`cargo test --test integration`, 60/60) re-proves the mechanism.
  Live re-verification against the rebuilt binary is open follow-up
  work for the maintainer.
