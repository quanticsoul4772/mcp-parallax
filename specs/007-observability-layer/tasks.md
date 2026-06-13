# Tasks: Observability Layer — OTLP Export

**Input**: Design documents from `specs/007-observability-layer/`
**Prerequisites**: plan.md, research.md (D1–D8, spike S1), data-model.md, contracts/telemetry.md, quickstart.md

**Tests**: REQUIRED (constitution IV) — emission logic against the SDK's in-memory exporters (no network, no bespoke seam); disabled-path inertness asserted; the wire path covered by the spike and acceptance against a wiremock OTLP double.

**Organization**: by user story. US1 (invocation spans + metrics in a backend) is the MVP; US2 reuses the same pipeline for checkpoint evaluations; US3 hardens the failure modes.

## Phase 1: Setup

- [ ] T001 Add the 0.32-train dependencies to `Cargo.toml` per research.md D1: `opentelemetry` 0.32, `opentelemetry_sdk` 0.32.1 (features trace, metrics), `opentelemetry-otlp` 0.32 (default-features off; trace, metrics, http-proto, reqwest-blocking-client, reqwest-rustls-webpki-roots, internal-logs), `opentelemetry-semantic-conventions` 0.32 (semconv_experimental defensively — D5); dev-dependency `opentelemetry_sdk` with the `testing` feature (in-memory exporters); `wiremock` already present. Verify `cargo check --all-features` compiles the tree.

## Phase 2: Foundational (blocking prerequisites for all stories)

- [ ] T002 S1 spike (gates everything — the 0.32 API is web-verified, not yet compiled): `examples/spike_otlp.rs` building the full path: gated init from env (endpoint present + `OTEL_SDK_DISABLED` app-side check) → resource (service.name/version/instance.id) → providers → one retroactive span from a synthetic record (`with_start_time` + `start_with_context(Context::new())` + `end_with_timestamp` — D8 trap 1) → counter/histogram emission → `force_flush` → `shutdown_with_timeout`. Run against a wiremock OTLP/HTTP double asserting requests arrive at `/v1/traces` and `/v1/metrics` when enabled and ZERO requests when the endpoint env is absent. Confirm the two flagged uncertainties (semconv_experimental gating of GEN_AI constants; retroactive timestamps accepted) and record findings in research.md D5/D8.
- [ ] T003 Core module: `src/observability.rs` — constants (data-model §1), `init() -> Result<Option<Guard>, ConfigError>` implementing the D2 gate **as a pure function over an env-lookup closure** (`fn gating(lookup: impl Fn(&str) -> Option<String>) -> ...`; `init()` passes `std::env::var` — the truth-table tests feed maps, never mutate process env, so they stay parallel-safe): endpoint-present detection across the three vars; `OTEL_SDK_DISABLED` honored app-side with the **OTel-spec semantics** (case-insensitive `"true"` disables, any other value does not — a named exception to the loud-malformed-config convention because the variable's contract is OTel's, not ours); malformed endpoint URL = `ConfigError` naming the variable; resource per D6, providers with batch/periodic exporters, instruments created once (data-model §5), static `ENABLED` flag, `Guard::shutdown()` (flush + `shutdown_with_timeout(FLUSH_TIMEOUT_MS)`, warn-logged, never propagated — FR-010), and a test-only constructor injecting `InMemorySpanExporter`/`InMemoryMetricExporter`. Unit tests: gate truth table over injected lookups (no env ⇒ disabled; endpoint ⇒ enabled; `OTEL_SDK_DISABLED` = `true`/`TRUE` ⇒ disabled, `false`/garbage ⇒ not disabled; malformed endpoint ⇒ ConfigError), disabled emit is a no-op, shutdown idempotence. Wire `pub mod observability;` into `src/lib.rs`; init in `src/main.rs` after config load, `Guard::shutdown()` before exit.

**Checkpoint**: gated pipeline exists and is spike-proven — stories can emit.

## Phase 3: User Story 1 — Watch every invocation in an existing telemetry backend (P1) 🎯 MVP

**Goal**: one span + metric set per tool invocation, derived from `InvocationRecord` at the record-write exit point, GenAI names for model/token fields.

**Independent test**: with injected in-memory exporters, complete invocations across outcome classes; finished spans and metrics match the records field-for-field; with telemetry disabled, nothing is emitted.

- [ ] T004 [US1] `emit_invocation(&InvocationRecord)` in `src/observability.rs` per data-model §3: span `parallax.{tool}`, kind CLIENT, retroactive timing (start = created_at − latency), status from outcome, attributes exactly per the table (GenAI constants where they exist + literal `"gen_ai.provider.name"` derived from the model id — D5; `parallax.*` namespace; `error.type` only on non-success), root context; metric emission (`parallax.invocations`, `.invocation.duration`, `.cost`, `gen_ai.client.token.usage` with input/output rows and the GenAI-standard buckets). Unit tests via injected in-memory exporters: field-for-field span equality against sample records for success and each error class (SC-001), token histogram rows, provider derivation (anthropic vs voyageai), no evidence/content attributes (FR-008/SC-005 unit slice).
- [ ] T005 [US1] Call `emit_invocation` at the single record-write exit point in `src/server/record.rs` (`RecordGuard::finish`, after the record struct is final, regardless of storage write success — telemetry is fire-and-forget, FR-006/FR-009). Integration test in `tests/integration.rs`: with a server built via the test constructor (injected exporters), a tool call produces exactly one span whose attributes equal the stored invocation record (success + failure paths); with telemetry disabled (default test config), zero spans and unchanged behavior (SC-003 inertness slice).

**Checkpoint**: US1 fully functional — invocations visible in any backend.

## Phase 4: User Story 2 — Measure the checkpoint layer's precision in a dashboard (P2)

**Goal**: one span + counter per checkpoint evaluation with boundary/verdict/suppression/fail-open; rates from telemetry equal rates from records.

**Independent test**: drive evaluations across all verdicts with injected exporters; spans match `CheckpointRecord`s; computed per-boundary rates equal SQL-computed rates.

- [ ] T006 [US2] `emit_checkpoint(&CheckpointRecord)` in `src/observability.rs` per data-model §4: span `parallax.checkpoint.{boundary}`, kind INTERNAL, status always Ok, attributes per the table (signal KINDS as a string array — never evidence strings, FR-008), `parallax.checkpoint.evaluations` counter + `.duration` histogram. Unit tests: field equality across all verdicts incl. suppressed and fail-open rows; evidence-absence assertion.
- [ ] T007 [US2] Call `emit_checkpoint` at the checkpoint record-write exit point in `src/checkpoint/run.rs` (`record()`, same fire-and-forget placement). Integration test: checkpoint evaluations (flag, suppressed, fail-open, silence) each produce exactly one span; per-boundary verdict counts derived from exported metrics equal `checkpoint_records` SQL counts (SC-002 slice).

**Checkpoint**: US2 done — the alarm-fatigue dashboard is feedable.

## Phase 5: User Story 3 — Telemetry can never hurt the server (P3)

**Goal**: unreachable collector, buffered shutdown, and stdout discipline proven harmless.

**Independent test**: enabled-with-unreachable-endpoint run behaves byte-identically to disabled; shutdown bounded; stdout clean.

- [ ] T008 [US3] Failure-mode hardening + tests: (a) integration test with telemetry enabled against an unreachable endpoint (`http://127.0.0.1:1` style) — every invocation completes with identical results/records/error surfaces to a disabled run, no error reaches the caller (FR-006, SC-004); (b) shutdown-bound test — `Guard::shutdown()` with the unreachable endpoint returns within `FLUSH_TIMEOUT_MS` + margin, never hangs (FR-010); (c) extend the existing stdio smoke test to run with telemetry enabled-but-unreachable and assert stdout carries only protocol frames (FR-007); (d) default the OTel internal-log noise via the existing `EnvFilter` setup in `src/main.rs` (`opentelemetry*=warn` unless overridden — D8 trap 5).

**Checkpoint**: all three stories complete.

## Phase 6: Polish & Cross-Cutting

- [ ] T009 [P] Acceptance: `examples/acceptance_otlp.rs` against an in-process wiremock OTLP double — drive real invocations (mocked Anthropic via wiremock, the 005/006 pattern) + checkpoint evaluations; decode the protobuf payloads (`opentelemetry-proto` dev-dependency) and assert SC-001 (one span per record, attribute values equal stored records), SC-002 (telemetry-computed rates == record-computed rates), SC-003 (endpoint unset ⇒ zero telemetry requests, plus the overhead half: time a large batch of disabled `emit_invocation` calls and assert a per-call microseconds bound demonstrating the atomic fast path), SC-004 (unreachable collector ⇒ identical behavior + bounded shutdown), SC-005 (attribute audit: no content/credentials anywhere in the payloads), SC-006 (GenAI attribute names present). Record results honestly in `specs/007-observability-layer/quickstart.md`.
- [ ] T010 [P] Docs + corpus sync: README.md (Environment table gains the OTel enablement row + the inheritance note + the `http://` scheme gotcha; status paragraph gains observability), CLAUDE.md status + runtime-config note, `docs/design/SDK_LANDSCAPE.md` §observability amended per D1 (bridge crates dropped — spans derive from records; verified versions noted) — Constitution I same-change amendment.
- [ ] T011 Full gate (`cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`) + code-reviewer and design-reviewer agent passes over the branch diff + apply findings.

## Dependencies & Execution Order

- T001 → T002 (spike needs the deps) → T003 (module shape follows spike findings) → stories.
- US1: T004 → T005. US2: T006 → T007 (T006 parallel-eligible with T004 in principle but both edit `src/observability.rs` — sequential by file). US3: T008 after T005/T007 (it exercises both emission points).
- Polish: T009 ∥ T010 after all stories; T011 last.
- Shared files: `src/observability.rs` (T003→T004→T006), `tests/integration.rs` (T005→T007→T008) — sequential chains.

## Implementation Strategy

The spike is deliberately the second task: the whole feature stands on a
web-verified-but-uncompiled 0.32 API, so S1 compiles the riskiest path
(retroactive spans, env gating, flush/shutdown, the two flagged
uncertainties) before the module hardens. US1 alone is shippable (invocation
visibility); US2 is the same pipeline pointed at the second record type; US3
converts the failure-mode promises into tests. Every tunable is a constant;
the only operator surface is the standard OTel env contract.
