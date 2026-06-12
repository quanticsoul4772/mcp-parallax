---

description: "Task list for Core Layer — Working Server with First Corrective (Verify)"
---

# Tasks: Core Layer — Working Server with First Corrective (Verify)

**Input**: Design documents from `/specs/001-core-layer/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: REQUIRED (Constitution Principle IV). Every story carries test tasks
written through the trait seams (`ModelClient`/`Storage`/`TimeProvider` mocks,
`wiremock` for the HTTP boundary); the suite passes without network or disk
state. Spikes 2 and 4 are manual-run live calls and are NOT part of the suite.

**Organization**: Tasks grouped by user story (US1 = P1 verify path, US2 = P2
failure surface, US3 = P3 observability) so each story is independently
implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3)

## Phase 1: Setup

**Purpose**: Dependencies and the shared vocabulary every story uses

- [X] T001 Add core dependencies to Cargo.toml: rmcp 1.x (server/macros/transport-io/schemars), reqwest (rustls, no default tls), schemars 1.x, jsonschema, sqlx (sqlite/runtime-tokio-rustls), uuid; dev-deps: wiremock. Run `cargo build` to lock. (Exact rmcp minor pinned later by T006.)
- [X] T002 [P] Extend Config with `ANTHROPIC_MODEL` (default `claude-opus-4-8`), `VERIFY_ENSEMBLE_K` (default 3, validated ≥ 1 — 0 is a config error), and `VERIFY_MAX_CLAIM_CHARS` (default 50000) in src/config.rs, with unit tests for defaults, the ≥1 bound, and the present-but-invalid hard-error contract
- [X] T003 [P] Add the Outcome taxonomy (success, refusal, truncation, timeout, retries_exhausted, invalid_input, validation_failure, config_error, cancelled) and matching AppError variants with distinct, descriptive Display messages in src/error.rs, with unit tests asserting each message names its class (data-model.md §6)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The four spikes (research.md) and the seam implementations every story builds on

**⚠️ CRITICAL**: The spikes validate the load-bearing glue BEFORE the modules are built; T007–T012 must be complete before any user story phase

- [X] T004 [P] Spike 1 — schema sanitizer fidelity in examples/spike_sanitizer.rs: derive the Verdict schema via schemars, transform to the Anthropic grammar subset, assert additionalProperties:false everywhere, constraints stripped, required complete (exit criteria in research.md)
- [X] T005 [P] Spike 3 — rmcp `Json<T>` round-trip in examples/spike_roundtrip.rs: in-process rmcp client sees `outputSchema` in the catalog and `structured_content` in the result
- [X] T006 Pin the exact rmcp minor that provides `Json<T>` in Cargo.toml based on T005's finding (depends on T005)
- [X] T007 [P] Spike 2 — one real structured-outputs call in examples/spike_client.rs (manual-run, needs ANTHROPIC_API_KEY, real spend): confirm `content[0].text` parses against a tiny schema and the documented `stop_reason` behavior
- [X] T008 [P] Spike 4 — thinking + `output_config` compatibility in examples/spike_thinking.rs (manual-run, real spend); record the yes/no finding in docs/design/SDK_USAGE_CORE.md (core does not depend on the answer)
- [X] T009 Schema sanitizer module in src/schema/sanitize.rs + src/schema/mod.rs (promotes T004's validated transform), unit tests covering every stripped keyword and the additionalProperties/required guarantees (depends on T004)
- [X] T010 [P] Defense-in-depth validator in src/schema/validate.rs using the jsonschema crate against the UNSANITIZED schema, unit tests proving it re-imposes exactly what the sanitizer strips (ranges, lengths, non-empty findings)
- [X] T011 Thin Anthropic client implementing ModelClient in src/client/anthropic.rs + src/client/mod.rs: `output_config.format` request body, stop_reason → Outcome mapping (end_turn/refusal/max_tokens), retry with backoff honoring MAX_RETRIES, REQUEST_TIMEOUT_MS; wiremock unit tests for each stop_reason, timeout, and retry exhaustion — no real network (depends on T003, T009)
- [X] T012 Mode registry in src/modes/mod.rs: CorrectiveMode struct (id, description, prompt_template, output_schema, ensemble_k), startup assertion that every registered schema is flat + closed (illegal schema fails boot), unit tests for the assertion both ways (depends on T009)

**Checkpoint**: Sanitizer, validator, client, and registry exist and are fully tested without network — user stories can begin

---

## Phase 3: User Story 1 - Verify a claim and get a structured verdict (Priority: P1) 🎯 MVP

**Goal**: A conforming MCP client connects over stdio, lists `verify` with declared schemas, invokes it, and always receives a structurally valid, stance-blind, ensemble-aggregated verdict

**Independent Test**: In-process MCP client lists tools and invokes verify against a mocked ModelClient; result lands in structured_content and validates against contracts/verify.tool.json

### Tests for User Story 1 (REQUIRED) ⚠️

> Write these first; they fail until T015–T018 land

- [ ] T013 [P] [US1] Integration test skeleton in tests/integration.rs: in-process rmcp client performs handshake, asserts the catalog lists `verify` with the inputSchema/outputSchema from specs/001-core-layer/contracts/verify.tool.json (acceptance scenario 1); plus a concurrency case — two simultaneous verify calls with distinct mocked results complete independently and results are never crossed (spec edge case 3)
- [X] T014 [P] [US1] Aggregation unit tests in src/modes/verify.rs test module (mockall ModelClient): majority verdict, tie → refuted with tie noted, quorum rule (< ⌈k/2⌉ completed passes → dominant failure, never a minority verdict), confidence = agreement ratio, findings deduplicated from majority side (data-model.md §4)

### Implementation for User Story 1

- [X] T015 [P] [US1] Verify types in src/modes/verify.rs: VerifyParams, PassVerdict (per-pass, grammar-minimal), Verdict (aggregated) with schemars derives; unit test asserting the derived schemas match contracts/ and pass the registry's flat+closed assertion
- [X] T016 [US1] Verify execution in src/modes/verify.rs: calibrated prompt template (each refutation names a concrete error + steelman lens; only claim/context placeholders exist — blindness is structural), k parallel ModelClient passes, aggregation per T014's tests (depends on T011, T012, T015)
- [ ] T017 [US1] rmcp server handler in src/server.rs: `#[tool_router]` Parallax struct holding the seams, `verify` tool returning `Result<Json<Verdict>, ErrorData>`, `get_info` with tools capability (depends on T016)
- [ ] T018 [US1] Wire src/main.rs: construct config → client/storage/clock → Parallax, `serve(stdio())`, keep --version/--help and the config-error exit path; plus a spawn-the-binary stdio smoke test in tests/integration.rs (dummy key env, handshake + tools/list — no model call) asserting stdout carries only protocol frames (FR-008) (depends on T017)
- [X] T019 [US1] Stance-blindness guarantee test in src/modes/verify.rs test module: prompt builder output contains claim and context verbatim and nothing else — no stance, history, or identity can flow through (acceptance scenario 5, SC-004's structural half)

**Checkpoint**: MVP — a stock MCP client gets schema-valid verdicts end-to-end (with ModelClient mocked in tests; live via quickstart)

---

## Phase 4: User Story 2 - Failures are distinct, named, and never silent (Priority: P2)

**Goal**: Every failure class surfaces as its own descriptive error; no partial results, no free-text salvage, nothing on stdout

**Independent Test**: Induced-failure matrix (wiremock refusal/truncation/timeout/exhausted retries, bad config, invalid input) — each produces its distinct error and no verdict

### Tests for User Story 2 (REQUIRED) ⚠️

- [ ] T020 [P] [US2] Induced-failure integration matrix in tests/integration.rs: wiremock-backed client returns refusal, max_tokens truncation, timeout, persistent 5xx (retry exhaustion), and a cancellation case (client drops the request mid-invocation; server stays healthy for the next call); assert each invoke yields the matching distinct error text, never a partial Verdict (acceptance scenarios 1–3; spec edge case 4)
- [ ] T021 [P] [US2] Startup failure tests in src/config.rs test module: missing ANTHROPIC_API_KEY and invalid VERIFY_ENSEMBLE_K/REQUEST_TIMEOUT_MS each refuse startup naming the exact variable (acceptance scenario 4)

### Implementation for User Story 2

- [ ] T022 [US2] Input validation in src/modes/verify.rs: empty/whitespace-only claim and oversized claim rejected as invalid_input BEFORE any model call (with the no-model-call assertion in tests via mockall expect-zero) (edge cases 1–2)
- [ ] T023 [US2] Failure surfacing in src/server.rs: map every AppError/Outcome class to a distinct ErrorData (FR-007), ensure validation_failure (validator rejection) is an error not a verdict; unit tests assert the full class → message table (depends on T017, T022)

**Checkpoint**: US1 + US2 — the verify path is safe to rely on; ambiguous failure is impossible

---

## Phase 5: User Story 3 - Every invocation is observable (Priority: P3)

**Goal**: Exactly one structured invocation record per call — success and every failure class — persisted through the Storage seam

**Independent Test**: Invoke verify several times including induced failures; assert one row each in invocation_records with all fields and correct outcome class

### Tests for User Story 3 (REQUIRED) ⚠️

- [ ] T024 [P] [US3] Storage conformance tests in src/storage/sqlite.rs test module against in-memory SQLite: migration idempotency, write + read-back of every Outcome value, one-row-per-id (contracts/invocation-record.schema.json)

### Implementation for User Story 3

- [ ] T025 [P] [US3] sqlx SQLite Storage implementation in src/storage/sqlite.rs + src/storage/mod.rs: invocation_records table per data-model.md §5, idempotent startup migration at DATABASE_PATH
- [ ] T026 [P] [US3] Telemetry module in src/telemetry.rs: InvocationRecord construction (record UUID, session_id = per-process UUID generated at startup, token sums across passes, cost from per-model pricing table, latency via TimeProvider), GenAI semantic-convention span attributes; unit tests with MockTimeProvider
- [ ] T027 [US3] Single-exit recording in src/server.rs: every invocation path (success and each failure class, including `cancelled` via a drop-guard so an abandoned invocation still records) funnels through one record-write point; integration tests assert exactly one record with correct outcome for success + each induced failure from T020, and two records for T013's concurrency case (depends on T023, T025, T026)

**Checkpoint**: All three stories independently functional

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T028 Run the quickstart manual acceptance pass (live key): SC-001 stock-client connect, SC-002 20-claim schema-validity run, SC-003 seeded-error catch set (≥10 seeded-error + ≥6 sound claims), SC-004 stance-flip set, SC-006 latency; record results in specs/001-core-layer/quickstart.md under a Results heading
- [ ] T029 [P] Update README.md and CLAUDE.md: status changes from "scaffold, transport not wired" to "serves verify"; document new env vars (ANTHROPIC_MODEL, VERIFY_ENSEMBLE_K)
- [ ] T030 Full gate (`cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`) plus code-reviewer and design-reviewer agent passes over the branch diff before merge

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: T001 first; T002/T003 parallel after it
- **Foundational (Phase 2)**: T004/T005/T007/T008 parallel after T001; T006 after T005; T009 after T004; T010 parallel; T011 after T003+T009; T012 after T009 — BLOCKS all stories
- **US1 (Phase 3)**: after Foundational; delivers MVP
- **US2 (Phase 4)**: after US1's T017 (server exists to surface errors through); test tasks T020/T021 can be written in parallel with US1
- **US3 (Phase 5)**: T024/T025/T026 independent of US1/US2 internals (seam-level); T027 needs T023 (the final error surface) — last story to close
- **Polish (Phase 6)**: after all stories; T028 needs a live key

### Parallel Opportunities

```text
After T001:  T002 ∥ T003 ∥ T004 ∥ T005 ∥ T007 ∥ T008
After T009:  T010 ∥ T012 (∥ T011 once T003 done)
US1 tests:   T013 ∥ T014 ∥ T015 before the chain T016 → T017 → T018
Cross-story: T024/T025/T026 (US3 seam work) ∥ US2 implementation
Spikes 2/4 (T007/T008) are manual-run and never block CI
```

## Implementation Strategy

**MVP first**: Phases 1–3 only → a working, schema-guaranteed verify over stdio
(US1). Stop, validate against the in-process integration tests, optionally do a
live smoke test via quickstart.

**Incremental delivery**: US2 hardens the failure surface (the trust story),
then US3 closes the observability loop. Each checkpoint is independently
demonstrable. The auto-commit hooks commit after each speckit phase; commit
manually after each task or logical group within phases.
