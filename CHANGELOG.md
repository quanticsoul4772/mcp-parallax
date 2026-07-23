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
