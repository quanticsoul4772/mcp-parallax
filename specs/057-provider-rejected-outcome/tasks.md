# Tasks: Provider rejections recorded as their own outcome

**Input**: Design documents from `specs/057-provider-rejected-outcome/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/outcome.md, quickstart.md

**Tests**: required. The owner's rule is test-first for every behaviour change: each test is written and run to fail before the code that makes it pass. Where a test cannot fail first, the task says so.

**The gate** is `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests` (run with `-j 2` on this machine, piped steps under `set -o pipefail`), plus `pre-commit run markdownlint` on changed markdown.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependency on an unfinished task)
- **[Story]**: US1, US2, US3 from spec.md

## Phase 1: Setup

- [X] T001 Run the full gate on branch `057-provider-rejected-outcome` before any code change and record the passing test counts as the baseline in `specs/057-provider-rejected-outcome/quickstart.md`

## Phase 2: Foundational (blocks every story)

- [X] T002 In `src/error.rs` tests, replace `outcome_strings_match_the_record_contract` (literals only; never opens the contract) with a test that reads the `outcome.enum` array from `specs/001-core-layer/contracts/invocation-record.schema.json` and the backticked values in the `parallax.outcome` row of `specs/007-observability-layer/contracts/telemetry.md`, and asserts each equals the recorded outcomes (`Outcome::ALL` minus `config_error`); plus a test that `Outcome::parse(o.as_str()) == Some(o)` for every `o` in `ALL`. Add `Outcome::ALL` (today's values). This passes on today's code by design: it pins what exists, and T004 makes it fail
- [X] T003 Write a failing test in `src/error.rs` tests: `AppError::ProviderRejected { status: 400, detail }` has outcome `Outcome::ProviderRejected`, whose string is `provider_rejected`, and its Display contains `HTTP 400` and the detail (fails to compile until T004); and a test in `src/observability.rs` tests that a record with outcome `provider_rejected` exports `parallax.outcome` and `error.type` equal to `provider_rejected`
- [X] T004 Add `AppError::ProviderRejected { status: u16, detail: String }` (Display `provider rejected the request: HTTP {status}: {detail}`), `Outcome::ProviderRejected`, its `as_str`/`parse` string `provider_rejected`, and the `ALL` entry, in `src/error.rs`; T003 passes and T002 fails on both contracts
- [X] T005 Add `provider_rejected` to the outcome enum in `specs/001-core-layer/contracts/invocation-record.schema.json` and to the `parallax.outcome` list in `specs/007-observability-layer/contracts/telemetry.md` (exact text asked); T002 and T003 pass

## Phase 3: User Story 1 — provider rejections separated from malformed output (P1, MVP)

**Goal**: the Anthropic client records provider 4xx (other than 429) as `provider_rejected`.

**Independent test**: against a `wiremock` double, each of the five rejections gives `ProviderRejected` with its status; an out-of-contract 200 still gives `Client` / `validation_failure`.

- [X] T006 [US1] Write failing tests in `src/client/anthropic.rs` tests: a credit-balance 400, a 401, a 403, a 404 and a rejected-request 400 each give `AppError::ProviderRejected` with that status, outcome `provider_rejected`, and the provider's message in the text; `err.billed()` is (0, 0); the effort-rejection hint still appears for an effort 400. Existing tests that assert `AppError::Client` for a 4xx change to `ProviderRejected` (each named when the task runs; that change is what lets them pass)
- [X] T006a [US1] Write a failing integration test in `tests/integration.rs`: with the Anthropic double returning HTTP 401, a tool call's surfaced error starts with `[provider_rejected]` and contains `HTTP 401`, and its one invocation record has outcome `provider_rejected`
- [X] T007 [US1] In `src/client/anthropic.rs`, build `ProviderRejected { status, detail + effort hint }` in the non-success branch after the 429/5xx retry check; T006 passes
- [X] T008 [US1] Run the client and error tests; out-of-contract 200, 429 and 5xx tests unchanged and passing

## Phase 4: User Story 2 — both backends agree (P2)

**Goal**: the OpenAI-compatible client records terminal 4xx the same way; its strategy ladder is unchanged.

**Independent test**: the same five rejections against its double give `ProviderRejected`; a strategy-rejecting 400 still moves to the next rung.

- [X] T009 [US2] Write failing tests in `src/client/openai_compat.rs` tests: the same five rejections each give `AppError::ProviderRejected` with that status and outcome `provider_rejected` and `err.billed()` is (0, 0); existing tests asserting `AppError::Client` for a terminal 4xx change (each named when the task runs); the ladder and out-of-contract 200 tests stay as they are; and `a_rejected_pinned_rung_fails_loud_naming_model_and_strategy` gets `ProviderRejected` with the last rejection's status
- [X] T010 [US2] In `src/client/openai_compat.rs`, build `ProviderRejected` in the terminal branch after the strategy-ladder check; T009 passes
- [X] T011 [US2] Run the client tests; ladder, 429 and 5xx tests unchanged and passing

## Phase 5: User Story 3 — the published surfaces state it (P3)

- [X] T012 [P] [US3] Add a dated 2026-11-20 amendment to `specs/001-core-layer/data-model.md` §6 adding `provider_rejected` to the taxonomy, with the reason (exact text asked)
- [X] T013 [P] [US3] Add a CHANGELOG entry under `[Unreleased]` → Changed in `CHANGELOG.md` (exact text asked)

## Phase 6: Polish & cross-cutting

- [ ] T014 Run the full gate and markdownlint on changed markdown
- [ ] T015 Independent review: a fresh general-purpose subagent given only the diff, told to trace every caller of `AppError`, `Outcome` and the two changed sites; each finding asked one question per finding
- [ ] T016 Update the CLAUDE.md Spec Kit block when the feature closes (exact text asked)

## Dependencies

- T001 → T002 → T003 → T004 → T005 → US1 (T006–T008) → US2 (T009–T011) → US3 (T012–T013) → Polish (T014–T016)
- T012 and T013 edit different files and can run beside US1/US2 once T005 is done.

## Parallel opportunities

- T012 and T013.

## Implementation strategy

MVP is US1: once T008 passes, the default backend records provider rejections as their own outcome. US2 brings the second backend in line; US3 and Polish complete the record and review. No live check: no prompt, schema or request changes.
