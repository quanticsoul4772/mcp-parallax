# Phase 0 Research: Core Layer

**Date**: 2026-06-11 · **Sources**: `docs/design/SDK_LANDSCAPE.md` (web-grounded
2026-06), `docs/design/SDK_USAGE_CORE.md`, `docs/design/NEW_SERVER_DESIGN.md` §5–6.
No NEEDS CLARIFICATION markers existed in the Technical Context; the decisions
below consolidate the corpus's already-researched choices plus the ones this plan
had to make.

## D1 — MCP framework: rmcp 1.x

- **Decision**: `rmcp` 1.x with `server`, `macros`, `transport-io`, `schemars`
  features; tools return `Json<T>` so results land in `structured_content` with a
  derived `outputSchema`.
- **Rationale**: Official Rust SDK, 1.0 since March 2026, the corpus keeper;
  `Json<T>`/`CallToolResult::structured` resolves the structured-output open item
  (SDK_USAGE_CORE §Part 1).
- **Alternatives considered**: Hand-rolled JSON-RPC loop (mcp-reasoning's origin)
  — more control, far more code, no reason now that rmcp is official and stable.
- **Open sub-item → Spike 3**: confirm the exact 1.x minor that introduced
  `Json<T>` and pin to it.

## D2 — Anthropic client: thin reqwest client (Option A)

- **Decision**: Hand-rolled thin client over `reqwest` targeting
  `output_config.format` (JSON Outputs mode), implementing
  `ModelClient::complete(prompt, schema)`. Retry/backoff lifted from
  mcp-reasoning's `anthropic/client.rs` pattern. `stop_reason` mapped before any
  parse: `end_turn` → parse, `refusal` → Refusal error, `max_tokens` →
  Truncation error, other → protocol error.
- **Rationale**: No official Anthropic Rust SDK exists; a core dependency on an
  unofficial crate is the brittleness this design sheds (SDK_LANDSCAPE §core 2).
  The structured-outputs request/response surface is small.
- **Alternatives considered**: `adk-anthropic` (claims full 2026-03 API parity)
  and `anthropic-sdk-rust` — both community-maintained; revisit only for breadth
  (thinking/effort/citations) after a maintenance-health check.

## D3 — Schema pipeline: schemars → sanitizer → both hops; jsonschema validator

- **Decision**: One `schemars`-derived schema per mode output type feeds both the
  rmcp `outputSchema` and the Anthropic `output_config.format.schema`. A
  **sanitizer** transforms schemars output to the Anthropic grammar subset:
  force `additionalProperties: false` on every object, complete `required`,
  strip `minimum`/`maximum`/`minLength`/`maxLength`/`$schema`/`title` and other
  unsupported keywords. The **`jsonschema`** crate validates returned values
  against the *unsanitized* schema — re-imposing exactly the constraints the
  sanitizer stripped.
- **Rationale**: The API grammar guarantees shape; the local validator guarantees
  value constraints the grammar can't express. Neither is redundant
  (SDK_USAGE_CORE §validator). Anthropic's own Python/TS SDKs do this transform
  silently; hand-rolling the client means owning it.
- **Alternatives considered**: `rsonschema` (2020-12-only, perf-tuned) — fine,
  but `jsonschema` is the mature default and validation is not hot-path-critical
  at this scale. Skipping the sanitizer and hand-writing API-legal schemas —
  rejected: breaks "one type, one schema, two hops" and invites drift.
- **This is Spike 1** — load-bearing; everything depends on it.

## D4 — Verify mechanics: k independent stance-blind passes, agreement-derived confidence

- **Decision**: One Verify invocation fans out **k = 3** (configurable,
  `VERIFY_ENSEMBLE_K`) parallel `ModelClient::complete` calls. Each pass gets
  only the claim + provided context — never requester stance, history, or
  identity (blind by construction). Prompt uses the spike's **calibrated
  profile**: every refutation must name a specific concrete error; a steelman
  lens is included. Per-pass output schema carries `verdict` + `findings`;
  the tool aggregates: majority verdict, union of findings from the majority
  side, and **confidence = agreement ratio** (a value the server computes —
  never the model's self-reported confidence).
- **Rationale**: The verify spike validated exactly this shape: k=3 parallel was
  immune to pushback where a sequential critic caved; the calibrated profile
  moved false positives 1/6 → 0/6 keeping catch at 6/6; the corpus mandates
  ensemble-agreement confidence over self-report (miscalibration row, §7.3).
  Spec SC-003/SC-004 numbers come from this setup.
- **Alternatives considered**: k=1 single pass — cheaper, but the spec's success
  criteria were validated at k=3 and the design's judge-bias contract calls for
  parallel judging; k stays configurable so cost-sensitive operators can lower it
  (accepting the calibration tradeoff).

## D5 — Invocation records: SQLite via the Storage seam (sqlx)

- **Decision**: `Storage` implemented with **sqlx** (sqlite, runtime-tokio,
  rustls). One `invocation_records` table, created by an idempotent startup
  migration at the existing `DATABASE_PATH`. One row per invocation (success or
  failure), written after outcome classification; spans also carry GenAI
  semantic-convention attribute names (`gen_ai.request.model`,
  `gen_ai.usage.input_tokens`, …) so a later OTLP exporter is an output change,
  not an instrumentation change.
- **Rationale**: Append-only SQLite is a design-sanctioned sink (§6.6); the
  Storage seam and `DATABASE_PATH` already exist for exactly this; the prior
  server's "metrics never persisted" failure is the lesson this closes (US3).
  sqlx is async-native under tokio. The known sqlite-vec/sqlx extension-loading
  caveat does **not** apply here (no extensions in this feature) and is the
  memory feature's spike to run.
- **Alternatives considered**: `rusqlite` + `spawn_blocking` — solid, but
  synchronous API under an async server for no benefit at this scale; full OTLP
  export now — deferred: an exporter endpoint is egress, off by default
  (Constitution VI), and records must exist even with no collector running.

## D6 — Error taxonomy: outcome classes are one enum, used twice

- **Decision**: A single outcome classification (success, refusal, truncation,
  timeout, retries_exhausted, invalid_input, validation_failure, config_error,
  cancelled) defined once, used as both the `AppError` variant mapping surfaced
  to the MCP client (distinct, descriptive messages — FR-007) and the
  `outcome` column on invocation records (FR-010).
- **Rationale**: US2's acceptance test is "identify the failure class from the
  error alone"; a shared taxonomy makes the error surface and the observability
  surface incapable of disagreeing.
- **Alternatives considered**: Free-form error strings — rejected; they rot and
  can't be asserted on in the induced-failure test matrix.

## D7 — Model + config additions

- **Decision**: `ANTHROPIC_MODEL` env var, default `claude-opus-4-8` (the
  corpus's stated target, structured-outputs GA). `VERIFY_ENSEMBLE_K` default
  `3`. Existing vars (`ANTHROPIC_API_KEY`, `DATABASE_PATH`, `LOG_LEVEL`,
  `REQUEST_TIMEOUT_MS`, `MAX_RETRIES`) keep their semantics. Present-but-invalid
  values remain hard errors (existing `parse_env` contract).
- **Rationale**: Config-from-env is the scaffold's established contract; the
  model must be operator-switchable without a rebuild (pricing table keyed by
  model id for the cost field).

## The four pre-build spikes (ordered; from SDK_USAGE_CORE §spikes)

| # | Spike | Validates | Exit criterion |
|---|---|---|---|
| 1 | `examples/spike_sanitizer.rs` | schemars → Anthropic-subset transform | Verdict schema sanitizes to a grammar-legal schema (additionalProperties:false everywhere, constraints stripped, required complete); unsanitized schema still validates values locally |
| 2 | `examples/spike_client.rs` (manual; needs key) | one real `complete()` against Opus 4.8 | `content[0].text` parses against the schema; `stop_reason` table behaves as documented |
| 3 | `examples/spike_roundtrip.rs` | rmcp `Json<T>` | in-process client sees `outputSchema` in the catalog and `structured_content` in the result; pin the rmcp minor |
| 4 | `examples/spike_thinking.rs` (manual; needs key) | thinking + `output_config` composability | documented yes/no; if no, core proceeds without thinking (it doesn't depend on it) and the limitation is recorded in the corpus |

Spikes 2 and 4 are manual-run (live API, real spend); they are excluded from the
test suite. Spikes 1 and 3 become permanent unit/integration tests after
validation.

## Risks carried into implementation

- **Grammar subset drift**: Anthropic may change supported keywords; the
  sanitizer is the single choke point, and Spike 2 re-runs cheaply.
- **rmcp minor-version pin** (Spike 3) — `Json<T>` introduction version must be
  confirmed before `Cargo.toml` is locked.
- **Thinking compatibility unknown** (Spike 4) — explicitly does not block core.
- **Cost table staleness**: cost-per-token is a config-time constant per model
  id; exactness to invoice is explicitly not required (spec assumption).
