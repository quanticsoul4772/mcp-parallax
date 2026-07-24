---

description: "Task list for 018 per-hop model routing"
---

# Tasks: Per-Hop Model Routing

**Input**: Design documents from `/specs/018-model-routing/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/config.md

**Tests**: REQUIRED (Constitution Principle IV). Every user story includes test tasks
written through the trait seams (`ModelClient`/`Storage`/`TimeProvider` mocks); the
suite must pass without network or disk. A story without tests is incomplete.

**Organization**: Tasks are grouped by user story so each can be implemented and
tested independently.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3)
- Exact file paths in every description

## Path Conventions

Single Rust project. Production code under `src/`; unit tests live in the
`#[cfg(test)] mod tests` block of the file they cover (project convention);
cross-module tests in `tests/integration.rs`.

---

## Phase 1: Setup

**Purpose**: establish the baseline the byte-identical claim will be measured against.

- [ ] T001 Run the full gate (`cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`) and record the passing test count as the pre-feature baseline
- [ ] T002 Identify and list, in `specs/018-model-routing/tasks.md` under Notes, the existing tests in `src/telemetry.rs` and `src/observability.rs` whose expected values must remain **unmodified** through this feature — these are the SC-004 evidence

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the pure routing layer. No wiring, no client, no I/O — resolution and
validation only.

**⚠️ CRITICAL**: no user story work begins until this phase is complete.

- [ ] T003 Create `src/routing.rs` with `Tier` (`Bulk`/`Judgment`), `CallSite` (the twelve ids from data-model.md §1), and their stable string suffixes, plus `RouteSource` (`Site`/`Tier`/`Default`)
- [ ] T004 Implement `CallSite::tier()` in `src/routing.rs` mapping `research_extract` to `Bulk` and the other eleven to `Judgment`
- [ ] T005 Implement `resolve(site, env, default) -> ResolvedRoute` in `src/routing.rs` applying most-specific-first order (site setting → tier setting → `ANTHROPIC_MODEL`)
- [ ] T006 Implement `RoutingTable::from_env` in `src/routing.rs` returning all twelve resolved routes, and namespace validation rejecting any `PARALLAX_MODEL_*` variable with an unrecognised suffix or an empty/whitespace value (FR-006, FR-006a)
- [ ] T007 [P] Unit tests in `src/routing.rs` covering: precedence (site beats tier beats default), tier fan-out, unknown suffix rejected by name, empty value rejected by name and value, unset namespace resolving every site to the default
- [ ] T008 Wire `RoutingTable` into `Config::from_env` in `src/config.rs` so a routing error is a startup `ConfigError`, with unit tests asserting the server refuses to start (SC-005)

**Checkpoint**: routing resolves and validates as pure computation, fully tested,
wired to nothing.

---

## Phase 3: User Story 1 - Route mechanical work to a cheaper model (Priority: P1) 🎯 MVP

**Goal**: a route an operator sets actually takes effect, and they can see that it did.

**Independent Test**: set `PARALLAX_MODEL_BULK` to a different model, start the
server, and confirm from the startup routing table that `research_extract` resolves to
it while the other eleven call sites resolve to `ANTHROPIC_MODEL`; confirm two call
sites routed alike share one client.

### Tests for User Story 1 (REQUIRED) ⚠️

> Write these first and confirm they fail before implementing.

- [ ] T009 [P] [US1] Test in `src/server.rs` that the client pool holds one entry per **distinct** resolved model, so two call sites routed alike share one client (FR-004)
- [ ] T010 [P] [US1] Test in `src/server.rs` that each call site's dependency struct receives the client for its own resolved model (FR-001)
- [ ] T011 [P] [US1] Test in `src/server.rs` that with no `PARALLAX_MODEL_*` set, every call site resolves to `ANTHROPIC_MODEL` and exactly one client is built (FR-002)
- [ ] T012 [P] [US1] Test in `tests/integration.rs` that the startup routing table names all twelve call sites with resolved model and supplying setting, and is emitted before serving (FR-005, SC-007)
- [ ] T013 [P] [US1] Test in `tests/integration.rs` that no tool's input schema mentions a model and the catalog gains no entry (FR-003, FR-005a, FR-016)

### Implementation for User Story 1

- [ ] T014 [US1] Create `src/client/pool.rs` (D10) with a function from the resolved model ids plus `&Config` to a `BTreeMap<String, Arc<dyn ModelClient>>` — one `AnthropicClient` per distinct id — and call it from `Parallax::new` in `src/server.rs` (FR-004). The pool goes here rather than in `routing.rs`, which stays free of any client dependency, or in `server.rs`, already at 1397 lines
- [ ] T015 [US1] Replace the single shared `Arc<dyn ModelClient>` with the per-call-site `Arc` in each dependency struct in `src/server.rs` (`CheckpointDeps`, `MemoryDeps`, `GroundedDeps`, `CheckDeps`)
- [ ] T016 [US1] Split `ResearchDeps.model_client` in `src/research/pipeline.rs` into four per-call-site fields (scope, extract, verify, synthesize) and update their use sites in `src/research/pipeline.rs`
- [ ] T017 [US1] Emit the startup routing table in `src/server.rs` as one `tracing::info!` event after config resolution and before serving — stderr only, never stdout (FR-005, Principle III)
- [ ] T018 [US1] Update `src/research/pipeline_tests.rs` construction of `ResearchDeps` for the new fields, keeping every existing assertion's expected value unchanged
- [ ] T046 [US1] Set `CheckpointDeps.model` in `src/server.rs` from the resolved `checkpoint_review` route rather than `config.anthropic_model` — the field feeds checkpoint-record cost attribution (`src/checkpoint/mod.rs:40`), so leaving it global silently misprices a routed review

**Checkpoint**: routing takes effect end to end and is observable. Shippable alone —
savings are real from here, only the accounting is still single-model.

---

## Phase 4: User Story 2 - Keep cost attribution correct across models (Priority: P2)

**Goal**: an invocation spanning two models records what was actually spent, and names
every model that spent it.

**Independent Test**: construct a two-model `ModelUsage`, build a record from it, and
compare its cost against a hand computation from published rates and per-model tokens;
confirm the record names both models and that the exported telemetry agrees.

### Tests for User Story 2 (REQUIRED) ⚠️

- [ ] T019 [P] [US2] Unit tests for `ModelUsage` in `src/telemetry.rs`: `totals()` sums, `dominant()` picks greatest input+output, ties break lexicographically, empty usage yields `None` (D5)
- [ ] T020 [P] [US2] Unit test in `src/telemetry.rs` that a two-model record's `cost_usd` equals the sum over models of that model's own tokens at that model's own rate (FR-008, SC-003)
- [ ] T021 [P] [US2] Unit test in `src/telemetry.rs` that a single-model record's `model`, `input_tokens`, `output_tokens`, and `cost_usd` are **identical** to the pre-feature values (FR-009a, SC-004)
- [ ] T022 [P] [US2] Unit test in `src/telemetry.rs` that pricing lookup returns `pricing_known = false` for an unrecognised model and still prices at the conservative Opus-tier fallback (FR-012)
- [ ] T023 [P] [US2] Test in `src/storage/sqlite.rs` that the migration is idempotent and that a row written before the migration reads back as a single-model record with `models`/`usage_by_model` absent (D4)
- [ ] T024 [P] [US2] Test in `src/observability.rs` that a multi-model invocation emits one span carrying `parallax.models` and `parallax.cost_estimated`, and records `parallax.cost` and `gen_ai.client.token.usage` once per participating model while `parallax.invocations` increments once (D6, FR-010)
- [ ] T025 [P] [US2] Test in `src/observability.rs` that every instrument is byte-identical to its pre-feature emission for a single-model invocation (SC-004)
- [ ] T026 [P] [US2] Test in `src/telemetry.rs` that a model which failed or never ran contributes nothing to `models`, usage, or cost (FR-015b)

### Implementation for User Story 2

- [ ] T027 [US2] Add `ModelUsage` (ordered `BTreeMap<String, Usage>`) with `single`, `add`, `totals`, `dominant`, and `cost_usd` in `src/telemetry.rs` (data-model.md §3)
- [ ] T028 [US2] Add the current pricing rows (`claude-opus-5` 5/25, `claude-sonnet-5` 3/15, `claude-fable-5` 10/50) and return `pricing_known` from the rate lookup in `src/telemetry.rs` (FR-011, D8)
- [ ] T029 [US2] Extend `InvocationRecord` in `src/telemetry.rs` with `models`, `usage_by_model`, and derived `cost_estimated`; change `create` to take `ModelUsage` and compute the attributed model, summed tokens, and summed cost (FR-007..FR-009)
- [ ] T030 [US2] Add the two nullable columns and the pragma-guarded `ALTER TABLE` migration in `src/storage/sqlite.rs`, following the 017 pattern, and persist/read the new fields
- [ ] T031 [US2] Change `run_recorded` and `RecordGuard` in `src/server.rs` to carry `ModelUsage`, and update all twelve call sites — eleven via `ModelUsage::single`, keeping the existing attributed-model fallback for cancelled and failed invocations
- [ ] T032 [US2] Make `RunMeter` in `src/research/mod.rs` accumulate per model behind its existing lock, leaving `total()` summing across models so token budgets and deadlines behave exactly as before
- [ ] T033 [US2] Update `src/observability.rs` for the new span attributes and per-model metric recording, and amend `specs/007-observability-layer/contracts/telemetry.md` in the same change (Principle I)
- [ ] T047 [US2] Update the `telemetry::cost_usd` call at `src/checkpoint/run.rs:304` for the `pricing_known` return added in T028, and add a test in `src/checkpoint/run.rs` that a routed `checkpoint_review` prices `checkpoint_records.cost_usd` at the **routed** model's rate (depends on T028, T046)

**Checkpoint**: cost is correct and attributable across models; US1's savings are now
measurable rather than merely real.

---

## Phase 5: User Story 3 - Route safely across model families (Priority: P3)

**Goal**: any model the provider offers can serve any call site without a truncated
verdict or a silently mispriced run.

**Independent Test**: route a call site to a model from each supported family in turn
and run its tool; every family returns a complete result and a correct cost, or fails
at startup naming the setting at fault.

### Tests for User Story 3 (REQUIRED) ⚠️

- [ ] T034 [P] [US3] Test in `src/client/anthropic.rs` that the request body sends no `thinking` field, so the one shape is accepted by families that reject `thinking: disabled` (FR-014, D7)
- [ ] T035 [P] [US3] Test in `src/client/anthropic.rs` that a response consuming the raised budget still yields a parsed verdict, and that a genuine overrun still classifies as `AppError::Truncation` (FR-013)
- [ ] T036 [P] [US3] Test in `tests/integration.rs` that an invocation on a model with no price row completes, is costed conservatively, and is marked estimated end to end (FR-012, US3 scenario 2 — depends on T028)
- [ ] T048 [P] [US3] Test in `tests/integration.rs` that a call site whose client always errors surfaces the outcome class and **never invokes a second client** — no cross-model retry, and the record's `models` names only the model that ran (FR-015a, FR-015b)
- [ ] T049 [P] [US3] Test in `tests/integration.rs` that `checkpoint_turn` with an unreachable routed review model returns silence with `fail_open` set and records the failure, leaving the turn unblocked (FR-015, US3 scenario 4)

### Implementation for User Story 3

- [ ] T037 [US3] Raise `MAX_TOKENS` in `src/client/anthropic.rs` to **at least 4×** the schema-derived answer floor (research D7 step 1: research synthesis bounds its own output at 8 000 answer chars + 10×500 gap chars ≈ ~3.5k tokens; every other mode schema is smaller). Compute the floor from the schemas — no network needed — and record the arithmetic in the commit message
- [ ] T038 [US3] Raise the `REQUEST_TIMEOUT_MS` default in `src/config.rs` to **at least 3×** the slowest single call observed while setting T037's budget, since a larger ceiling on a thinking-by-default family can outrun 30 s (D7 step 3)
- [ ] T039 [US3] Document in `specs/018-model-routing/quickstart.md` the measured values chosen in T037/T038, replacing the placeholder guidance
- [ ] T050 [US3] Family sweep: route one call site to a model from each **completion** family in the shipped price list in turn and run its tool, recording in this file that each returned a complete result and a correct cost (SC-006; embedding models are excluded — they answer no call site). This sweep is also the **acceptance test for T037/T038** (D7 step 4): zero `AppError::Truncation` and zero `AppError::Timeout` outcomes. If either appears, raise the offending value and re-run the sweep

**Checkpoint**: every completion model in the shipped price list can serve every call site (SC-006), and the budget and timeout are validated rather than assumed.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T040 Amend the design corpus in `docs/design/` for per-call-site routing — **required by Constitution Principle I, not optional follow-up**: `SDK_LANDSCAPE.md` §core currently describes a single `ModelClient` behind one configured model
- [ ] T041 [P] Record the named deferral (per-family `thinking` suppression, research D7) in the corpus amendment so it is visible outside this feature's spec directory
- [ ] T042 [P] Append the `[Unreleased]` entry to `CHANGELOG.md` in Keep a Changelog 1.1.0 format, covering the routing surface, the record/telemetry change, and the pricing rows
- [ ] T043 Run the full gate and confirm the SC-004 evidence from T002 — the listed test expectations must still be unmodified
- [ ] T044 Walk `specs/018-model-routing/quickstart.md` end to end against the built binary and correct anything that does not match observed behavior
- [ ] T045 Live dogfood: run one research question unrouted, then with `PARALLAX_MODEL_BULK` set to a cheaper model; record in this file both costs, the per-model split, the measured saving against SC-001, and a diff of the two runs' verified findings against SC-002

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies
- **Foundational (Phase 2)**: depends on Setup — BLOCKS all user stories
- **US1 (Phase 3)**: depends on Foundational
- **US2 (Phase 4)**: depends on Foundational. Independent of US1 — `ModelUsage` and
  per-model cost are testable against a constructed two-model usage with no routing
  configured — but only *valuable* once US1 makes multi-model invocations reachable
- **US3 (Phase 5)**: depends on Foundational; T036 additionally depends on T028;
  T050 depends on T028 (the price list it sweeps) and T037/T038 (the budget it
  exercises)
- **Polish (Phase 6)**: depends on all desired stories

### Checkpoint-layer cost attribution (added after `/speckit-analyze`)

`checkpoint_review` is routable, and the checkpoint layer keeps its **own** cost
record (`checkpoint_records.cost_usd`, exported as `parallax.checkpoint.cost_usd`)
computed from `CheckpointDeps.model` at `src/checkpoint/run.rs:304`. Two tasks close
that hole and must land together or the field lies: T046 threads the resolved model,
T047 updates the cost call and proves the rate is the routed one.

### Within Each User Story

- Tests written and failing before implementation
- Types before the code that carries them (T027 before T029 before T031)
- T031 is the wide change — land it with the suite green **before** T029's cost
  semantics change, so a plumbing bug and an accounting bug cannot arrive together
  and be mistaken for each other (plan.md Risks)

### Parallel Opportunities

- T007 runs alongside T003–T006 as they land
- T009–T013 (US1 tests) are five different assertions and can be written in parallel
- T019–T026 (US2 tests) touch four different files and can be written in parallel
- T034–T036 (US3 tests) can be written in parallel
- T041 and T042 are different files from T040
- US2 and US3 can proceed in parallel with US1 once Phase 2 is done, if staffed

## Parallel Example: User Story 2

```bash
# Launch the US2 test tasks together — four distinct files:
Task: "T019 ModelUsage unit tests in src/telemetry.rs"
Task: "T023 migration idempotence + NULL read-back in src/storage/sqlite.rs"
Task: "T024 multi-model span and per-model metrics in src/observability.rs"
Task: "T036 unknown-price end to end in tests/integration.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1 Setup — capture the baseline
2. Phase 2 Foundational — routing resolves and validates
3. Phase 3 US1 — routing takes effect and is visible
4. **STOP and VALIDATE**: set `PARALLAX_MODEL_BULK`, confirm the startup table and a
   cheaper research run
5. At this point savings are real; cost records still report them single-model

### Incremental Delivery

1. Setup + Foundational → routing layer ready
2. US1 → routes take effect → **MVP**
3. US2 → cost becomes correct and attributable → savings measurable
4. US3 → the safe model set widens beyond families that do not reason by default
5. Polish → corpus amendment (constitutional), CHANGELOG, dogfood

---

## Notes

- `[P]` = different files, no dependencies
- Task **IDs are stable identifiers, not execution order**. T046–T050 were added
  after `/speckit-analyze` found coverage gaps; they are placed physically in the
  phase they belong to, and existing IDs were left alone so the cross-references in
  this file and in review history keep pointing at the same work. Execute in file
  order within each phase.
- The `ModelClient` trait signature does not change in any task. If a task appears to
  require changing it, the design has drifted — re-read research D2 before proceeding
- SC-004 evidence: T002 lists the test expectations that must survive untouched. If a
  task requires editing one of them, the byte-identical-when-unrouted invariant broke
  and that is a defect, not a test that needs updating
- Commit after each task or logical group; the full gate runs before every commit
