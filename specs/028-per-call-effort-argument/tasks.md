---

description: "Task list for 028 — per-call reasoning effort"
---

# Tasks: Per-Call Reasoning Effort

**Input**: Design documents from `/specs/028-per-call-effort-argument/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/tool-surface.md

**Tests**: REQUIRED (Constitution Principle IV). Every story below includes test
tasks written through the trait seams; the suite must pass without network or
disk.

**Organization**: grouped by user story so each is independently implementable
and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable — different files, no dependency on an incomplete task
- **[Story]**: US1–US5 from spec.md
- Exact file paths in every description

## Path Conventions

Single Rust project: `src/`, `tests/` at repository root.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: the one prerequisite D1 depends on — a shared HTTP client, so the
eager cross product does not multiply connection pools.

- [X] T001 Add `AnthropicClient::with_http_client(config, http, model, effort)` to `src/client/anthropic.rs`, taking an existing `reqwest::Client` instead of calling `reqwest::Client::new()`; keep the current constructors delegating to it so no caller changes
- [X] T002 Add a test in `src/client/anthropic.rs` asserting two clients built from one `reqwest::Client` are independently configured (different model and effort) while sharing the transport

**Checkpoint**: existing suite green; no behaviour change yet.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the plumbing every story needs. **No user story can start until this
phase completes.**

- [X] T003 Extend `ClientPool` in `src/client/pool.rs` with a `by_key: BTreeMap<(String, Option<Effort>), Arc<dyn ModelClient>>` populated eagerly with the full cross product of distinct routed models × six effort states (five levels + `None`), built over one shared `reqwest::Client` from T001
- [X] T004 Add `ClientPool::for_site_with_effort(site, override: Option<Effort>) -> Arc<dyn ModelClient>` in `src/client/pool.rs`, returning the existing `by_site` entry when `override` is `None` and the `by_key` entry otherwise; infallible, because the map is populated from a total function over a finite domain
- [X] T005 [P] Add `effort: Option<String>` to `InvocationRecord` in `src/telemetry.rs`, mirroring the existing `depth` field; document that it holds the **per-call override only**, never the configured level (D4)
- [X] T006 [P] Add the additive `effort TEXT` column in `src/storage/sqlite.rs` using the existing pragma-guarded `ensure_column` pattern (the `depth` precedent at `:275`), and read it back in the row mapper at `:196`
- [X] T007 Test in `src/client/pool.rs` that the pool contains one entry per `(model, effort)` combination and that `for_site_with_effort` returns a *different* client for the same site under a different effort, and the *same* client when the override equals the site's configured effort
- [X] T008 [P] Test in `src/storage/sqlite.rs` that a record written with `effort: None` reads back `None` meaning *no override* (not a default level), and that a row written before the migration also reads back `None`

**Checkpoint**: the pool can produce a client for any effort; the record can carry
one. Nothing caller-facing yet.

---

## Phase 3: User Story 1 — The caller sets effort for one invocation (P1)

**Goal**: a calling model raises or lowers reasoning effort for a single call with
no file edited and no restart.

**Independent Test**: invoke a tool with an explicit effort; confirm the outbound
request carries that level, with no configuration change.

### Tests for User Story 1 (REQUIRED) ⚠️

- [X] T009 [P] [US1] Wire test in `tests/integration.rs`: with no effort configured, a call supplying `effort: "max"` produces a request body carrying `effort: "max"`
- [X] T010 [P] [US1] Wire test in `tests/integration.rs`: with a tier effort configured, a call supplying a different effort sends the caller's value, and the next call without one sends the configured value again (FR-004 — no persistence)
- [X] T011 [P] [US1] Test in `src/server.rs` that an unrecognised effort string is a typed caller input error naming the accepted values, distinct from a provider rejection (FR-006)
- [X] T012 [P] [US1] Test in `tests/integration.rs` that two concurrent invocations at different efforts each carry their own level, with no leakage

### Implementation for User Story 1

- [X] T013 [US1] Add an optional `effort: Option<String>` input property to the seven corrective param structs in `src/modes/` and `src/server.rs` — `verify`, `unstick`, `diverge`, `decide`, `elicit`, `grounded_verify`, `check` — per `contracts/tool-surface.md`; parse to `routing::Effort` at the boundary, rejecting unknown strings
- [X] T013a [P] [US1] Test in `tests/integration.rs` that a valid level the routed model does not accept still reaches the client rather than being refused at the tool boundary, so a future pre-check cannot be added without failing a test (FR-010)
- [X] T014 [US1] Thread the parsed override to client selection in `src/server.rs`, replacing `self.pool.for_site(site)` with `self.pool.for_site_with_effort(site, override.or(routing.effort_for(site)))` at each of the seven corrective handlers
- [X] T015 [US1] Stamp the per-call override onto the invocation record in `src/server.rs`'s `run_recorded` — the override when one was supplied, NULL otherwise; the configured level is never written (FR-007)

**Checkpoint**: US1 delivers alone. 027 becomes verifiable at this point without
touching configuration.

**Resolved, not blocked.** An earlier pass recorded a wire-level assertion as
impossible: `Parallax::new` reused the injected mock only for
`(default model, no effort)`, so any effort-carrying call built a real client
against the production endpoint. The `code-reviewer` pass showed this was worse
than a testing gap — a `cargo test` run was opening TLS connections to
`api.anthropic.com` carrying the fixture key, violating Principle IV.
`Config::anthropic_api_base` fixes the cause; T009 and T012 now exist and T009
is mutation-verified (drop the override at the call site and it fails on
`left: Null, right: "max"`).

---

## Phase 4: User Story 2 — Saying nothing changes nothing (P1)

**Goal**: a caller supplying no effort gets exactly the prior behaviour.

**Independent Test**: with the namespace empty and no argument, confirm the
request body is byte-identical to before this feature.

### Tests for User Story 2 (REQUIRED) ⚠️

- [X] T016 [P] [US2] Extend the existing `without_a_routed_effort_the_request_carries_no_effort_key` test in `src/client/anthropic.rs` to cover the case where the *tool* also supplied nothing, asserting no `effort` key appears anywhere in the body
- [X] T017 [P] [US2] Test in `tests/integration.rs` that with an effort configured for a site and none supplied by the caller, the configured level is sent unchanged
- [X] T018 [P] [US2] Test in `src/client/pool.rs` that the client count for a deployment with no effort configured is unchanged from before this feature on the *default* path — the cross product exists but the site array still binds the same entries

### Implementation for User Story 2

- [X] T019 [US2] Confirm the `None` override path in `src/server.rs` short-circuits to `by_site`, so the default path performs no map lookup and constructs nothing

**Checkpoint**: the silent path is provably untouched.

---

## Phase 5: User Story 3 — Precedence is stated and observable (P2)

**Goal**: the effort in force is attributable to the layer that supplied it.

**Independent Test**: with tier, site, and per-call efforts all set, confirm the
per-call value wins and the record shows what was used.

### Tests for User Story 3 (REQUIRED) ⚠️

- [X] T020 [P] [US3] Test in `src/routing.rs` that per-call beats site beats tier beats absent, for a site whose model comes from a tier and whose effort comes from its own variable (extending the existing most-specific-first test)
- [X] T021 [P] [US3] Test in `src/server.rs` that the record's `effort` equals the caller's override when supplied and is `None` when not — including the case where a configured level was in force, which must still record `None` (FR-007, FR-007a)

### Implementation for User Story 3

- [X] T022 [US3] Document the four-layer precedence in the `src/routing.rs` module docs, which currently describe two layers
- [X] T023 [US3] Extend the startup routing report in `src/routing.rs` to note that a per-call argument can override the printed effort, so the table is not read as the final word

**Checkpoint**: an unexpected level is traceable to its source.

---

## Phase 6: User Story 4 — The caller sets a verification's pass count (P2)

**Goal**: a caller lowers the pass count for one claim and cannot raise it.

**Independent Test**: request one pass against a configured three; confirm one ran
and the result says one. Request five; confirm a caller error naming the ceiling.

### Tests for User Story 4 (REQUIRED) ⚠️

- [X] T024 [P] [US4] Test in `src/modes/verify.rs` that a per-call count of 1 runs exactly one pass and the result reports `passes_used: 1`
- [X] T025 [P] [US4] Test in `src/modes/verify.rs` that a per-call count above the configured value is a caller input error stating the ceiling, and that **no** passes run (FR-012a)
- [X] T026 [P] [US4] Test in `src/modes/verify.rs` that with no per-call count the configured value runs and `passes_used` still reports it — the field is unconditional (FR-013)
- [X] T027 [P] [US4] Equivalent pass-count tests for `src/modes/diverge.rs` and `src/modes/grounded_verify.rs`

### Implementation for User Story 4

- [X] T028 [US4] Change the run path in `src/modes/mod.rs` to resolve `Option<u8>` override against `Mode.ensemble_k` as the default, leaving registration unchanged (D2)
- [X] T029 [US4] Add the optional `passes` input property and the required `passes_used` output field to `verify`, `diverge`, and `grounded_verify` per `contracts/tool-surface.md`; schemas stay flat and closed
- [X] T030 [US4] Enforce the ceiling in the boundary parse, returning a caller input error naming the configured count

**Checkpoint**: pass count is caller-reachable and bounded.

---

## Phase 7: User Story 5 — The caller lowers research concurrency (P3)

**Goal**: a caller reduces a research run's concurrency; raising is clamped.

**Independent Test**: run with concurrency 2 and confirm at most two tasks run
concurrently; request 16 against a ceiling of 8 and confirm the run proceeds at 8.

### Tests for User Story 5 (REQUIRED) ⚠️

- [X] T031 [P] [US5] Test in `src/research/pipeline_tests.rs` that a per-call concurrency of 2 bounds the semaphore to 2
- [X] T032 [P] [US5] Test in `src/research/pipeline_tests.rs` that a per-call concurrency above the configured ceiling is reduced to the ceiling and the run still completes (D3 — clamp, not reject)
- [X] T033 [P] [US5] Test in `src/research/pipeline_tests.rs` that the effective (post-clamp) concurrency is both the value the run used and the value written to the record, so an **operator** can determine what ran — the caller is deliberately not told (D3)

### Implementation for User Story 5

- [X] T034 [US5] Add `concurrency: Option<u32>` to `Constraints` in `src/research/contract.rs` beside `max_sources`, `budget_tokens` and `deadline_ms`
- [X] T035 [US5] Apply `min(requested, configured)` where `Deps.concurrency` is built in `src/server.rs:286`, and pass the effective value into the pipeline

**Checkpoint**: all four settings resolved; the rule in FR-009 is no longer
contradicted by any remaining instance.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [X] T036 [P] Confirming test for FR-016: assert in `tests/integration.rs` that `recall` accepts a per-call `limit` and that `MEMORY_RECALL_LIMIT` is its default — the prior art this feature generalises. If it fails, FR-016 becomes implementation work
- [X] T037 Re-ground the 018 T013 assertion in `tests/integration.rs:670`: keep the model/tier prohibition, narrow the doc comment from "routing is an operator concern" to "**model** selection is an operator concern; effort is not", assert explicitly that an `effort` property is permitted while a `model` property is not, and that exactly the seven correctives expose `effort` while `research`, the memory tools, and the `checkpoint_*` tools do not (FR-008, FR-011)
- [X] T038 [P] Record the operator-owned vs caller-owned test in `docs/design/NEW_SERVER_DESIGN.md`, citing `research.depth`, `recall.limit`, and this feature's effort/passes/concurrency. State explicitly that the **remaining configuration variables were not audited** against the test, so a later reader does not mistake the rule's presence for its having been applied everywhere (FR-009, SC-008)
- [X] T039 [P] Amend `specs/022-per-call-site-effort/spec.md` to record that the env-only surface was the wrong shape and 028 corrects it, without rewriting what 022 shipped
- [X] T040 [P] Document the per-call arguments in `README.md` and `CLAUDE.md` alongside the `PARALLAX_EFFORT_*` namespace documented by 027, including the lowering-only rule
- [X] T041 [P] Add a `CHANGELOG.md` `[Unreleased]` entry covering all four settings, the record column, and the lowering-only rule
- [X] T042 Run the full gate: `cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`
- [X] T043 Constitution-required pre-merge reviews: `design-reviewer` (tool surface, schemas, client pool) and `code-reviewer` (Rust conventions), with D3's clamp called out explicitly as the item to scrutinise against Principle III
- [ ] T044 Live verification: call a corrective with `effort: "low"` on a site routed to `claude-haiku-4-5` and confirm 027's enriched error appears — no config edit, no restart. This is the first live verification this project can run without a session restart, and it retires 027's outstanding unverified status

---

## Dependencies

```text
Phase 1 (Setup)  ──►  Phase 2 (Foundational)  ──┬──►  Phase 3 (US1, P1)  ──►  Phase 4 (US2, P1)
                                                │                              │
                                                │                              └──►  Phase 5 (US3, P2)
                                                │
                                                ├──►  Phase 6 (US4, P2)   [independent of US1–US3]
                                                └──►  Phase 7 (US5, P3)   [independent of everything]

Phases 3–7  ──►  Phase 8 (Polish)
```

- **US2 depends on US1** only in that its assertions are about the path US1 adds;
  the implementation is a confirmation, not new code.
- **US3 depends on US1** — precedence is only observable once the top layer exists.
- **US4 and US5 are fully independent** of US1–US3 and of each other. They touch
  different files and could be built first if effort were deprioritised.
- **T037 depends on US1** (the effort property must exist for the assertion to
  mean anything). Every other Phase 8 task is independent.

## Parallel Opportunities

- **Phase 2**: T005, T006, T008 in parallel with each other; T003→T004 sequential
- **Phase 3**: T009–T012 all parallel (distinct test fns); then T013→T014→T015 sequential (same files)
- **Phase 4**: T016, T017, T018 all parallel
- **Phase 5**: T020, T021 parallel
- **Phase 6**: T024–T027 parallel; T028→T029→T030 sequential
- **Phase 7**: T031–T033 parallel; T034→T035 sequential
- **Phase 8**: T036, T038, T039, T040, T041 all parallel; T042→T043→T044 sequential and last
- **Across phases**: Phase 6 and Phase 7 can run concurrently with Phase 3–5 once Phase 2 lands, since they share no files

## Implementation Strategy

**MVP = Phase 1 + Phase 2 + Phase 3 (US1).** That alone delivers the thing the
feature exists for — effort reachable by the caller with no restart — and makes
027 verifiable. Everything after it is the same correction applied to the
remaining settings, plus the evidence that the silent path is untouched.

**Recommended increment order**: US1 → US2 (cheap, and it protects US1 from
regression) → US4 → US3 → US5 → Polish. US3 is deferred below US4 because
attribution matters more once more than one setting is caller-controlled.

**Stop-and-review point**: after Phase 2. If the eager cross product turns out
larger or costlier than D1 predicts, that is where it shows, and the fallback
(lazy memoisation) is a local change to two functions rather than a redesign.
