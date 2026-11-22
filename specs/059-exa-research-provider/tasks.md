# Tasks: Exa replaces Brave for research

**Input**: Design documents from `specs/059-exa-research-provider/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/exa-provider.md, quickstart.md

**Tests**: required. Every behavior change gets a test written to fail first
(project rule).

**Organization**: by user story. US1 and US2 are P1, US3 is P2. US2's config
errors are independent of US1; US3 builds on US1's client.

## Phase 1: Setup

None: no new dependency. Two are removed in US1 (`rs-trafilatura`,
`robotstxt`).

## Phase 2: Foundational (blocks US1 and US3)

- [X] T001 Write failing unit tests in `src/traits/search.rs` for the widened seam: `search(&SearchRequest)` (query, count, search type, include and exclude domains) returning hits (url, title) and an optional request cost, and `contents(&ContentsRequest)` (urls, max age hours, max characters) returning pages (url, title, text, truncated) and an optional request cost, using the mock
- [X] T002 Widen `SearchProvider` in `src/traits/search.rs` with both methods; add `SearchType` (`auto`, `deep-lite`) and the depth mapping quick→auto, standard→auto, deep→deep-lite in `src/research/mod.rs` (research.md D2) with a unit test per depth; adapt `BraveClient` and the mocks to the new shape (Brave: hits as before, no cost; `contents` unsupported, never called while the pipeline still fetches) so the crate builds after this task (analyze A1)

**Checkpoint**: the seam compiles with the new shape; no provider implements it yet.

## Phase 3: User Story 1 — Research runs on Exa (P1)

**Goal**: sources and their text come from one Exa search per angle; Parallax fetches no page.

**Independent test**: run `research` against an Exa double and confirm the sources, the extracted claims and the grounding all come from the double's text, with no page request.

- [X] T003 [US1] Write failing wiremock tests for `ExaClient` in `src/client/exa.rs`: `POST /search` with `x-api-key`, `type`, `numResults`, `includeDomains`/`excludeDomains` only when non-empty and no contents request, mapping to hits (title falls back to the URL); `POST /contents` with `urls`, `text.maxCharacters` and `maxAgeHours`, mapping to pages (text ≥ 99% of the cap is truncated; missing text is empty; a URL with an `error` status is absent)
- [X] T004 [US1] Implement `ExaClient` in `src/client/exa.rs` (`new`, `with_base_url`, `with_backoff_base_ms`) and export it from `src/client/mod.rs`
- [X] T005 [US1] Add `EXA_API_KEY` and `EXA_MAX_AGE_HOURS` (-1..=720, default 168; out of range or unparseable is a startup error naming it) to `src/config.rs`, with tests first; update the config facts in `src/config_facts/` and `--help` in `src/main.rs` and README's configuration rows (exact text approved first) so their binding tests pass (analyze A2)
- [X] T006 [US1] Write failing pipeline tests in `src/research/pipeline_tests.rs`: text comes from one contents call for the kept URLs only (after dedup, domain filter and the source cap; no fetcher); a page with empty text, or a URL absent from the contents response, is dropped and counted; domain constraints reach the search request and the local filter still excludes; each depth sends its search type; a truncated page yields `sources[].truncated = true`; every Exa search failing returns the provider error, not an empty answer (analyze A5); the contents request failing returns its error, metered (analyze B1)
- [X] T007 [US1] Rework `src/research/pipeline.rs`: build the `SearchRequest` per angle from settings and constraints, make one `ContentsRequest` for the kept URLs, take each source's title and text from its page (trimmed, the existing 16,000 cap), drop the `Fetcher` parameter from `run_at`/`run_metered_at` and update the call in `src/server.rs` (analyze A1); add `truncated` to `SourceRef` in `src/research/contract.rs`; reword the `sources_fetched`, `fetched_at` and per-angle count-cap comments (analyze A6)
- [X] T008 [US1] Remove `readable_text` and `ReadablePage`'s HTML path from `src/research/extract.rs` (the extraction prompt is unchanged), delete `src/research/fetch.rs` and `src/traits/fetcher.rs`, remove `rs-trafilatura` and `robotstxt` from `Cargo.toml` (and `reqwest`'s `stream` feature if nothing else uses it)
- [X] T009 [US1] Wire `ExaClient` in `src/server.rs` (catalog gating on `EXA_API_KEY`, no per-run fetcher, the disabled-capability message names `EXA_API_KEY`); delete `src/client/brave.rs`
- [X] T010 [US1] Move `tests/common/mod.rs` (`serve_with_research`, `mount_search_and_page`) and the research tests in `tests/integration.rs` to an Exa double; switch `examples/acceptance_research.rs` to `ExaClient` and `EXA_API_KEY`

**Checkpoint**: US1 complete; research runs on Exa end to end against doubles.

## Phase 4: User Story 2 — Removed variables are startup errors (P1)

**Goal**: an operator upgrading with a removed variable is told, and the server does not start.

**Independent test**: load config with each removed variable set and read the error.

- [ ] T011 [US2] Write failing tests in `src/config.rs`: `BRAVE_API_KEY` set without `EXA_API_KEY` is a `ConfigError` naming both; with both set, config loads and ignores `BRAVE_API_KEY`; `FETCH_ALLOW_PRIVATE` and `FETCH_TIMEOUT_MS` set are errors naming each and saying Parallax no longer fetches pages
- [ ] T012 [US2] Implement the three removed-variable errors in `src/config.rs`; remove the `brave_api_key`, `fetch_timeout_ms` and `fetch_allow_private` fields and their uses; update `src/config_facts/` (sources and exclusions), `--help`, and README's configuration rows (exact text approved first; analyze A2)

**Checkpoint**: US2 complete.

## Phase 5: User Story 3 — Cost and retries are visible (P2)

**Goal**: Exa's cost is in the record and telemetry; a transient Exa failure is retried.

**Independent test**: a double reports cost and rate-limits the first request with `Retry-After`; the run succeeds after the delay and the record carries the summed cost.

- [ ] T013 [US3] Write failing tests in `src/client/exa.rs`, for both `/search` and `/contents`: 429 with `Retry-After` (seconds, and HTTP date) waits that long and retries; 5xx and network errors retry with backoff up to `MAX_RETRIES`; 400 is not retried and maps to `provider_rejected`; `costDollars.total` is returned, absent cost is `None`
- [ ] T014 [US3] Implement the retry rules and cost parsing in `src/client/exa.rs`
- [ ] T015 [US3] Write failing tests: the run meter in `src/research/mod.rs` sums Exa costs and flags a missing one; `InvocationRecord.search_cost_usd` in `src/telemetry.rs` is the sum, null when any cost was missing, null for other tools, and set on a failed run; the column round-trips in `src/storage/sqlite.rs` (older rows read null)
- [ ] T016 [US3] Implement the meter, record field, additive `ALTER TABLE invocation_records ADD COLUMN search_cost_usd REAL` migration, and the server plumbing from `run_at` to the record in `src/server.rs`
- [ ] T017 [US3] Write a failing test in `src/observability.rs` that the research span carries `parallax.search.cost_usd` equal to the record's value and other tools' spans do not; implement it
- [ ] T018 [US3] Add an integration test in `tests/integration.rs`: a research run whose first Exa request is rate-limited with `Retry-After` succeeds, and its stored record has the summed `search_cost_usd` (SC-003, SC-004)

**Checkpoint**: US3 complete.

## Phase 6: Polish & Cross-Cutting

- [ ] T019 Update `specs/007-observability-layer/contracts/telemetry.md` with `parallax.search.cost_usd` and the record contract for `search_cost_usd`
- [ ] T020 Dated amendments (2026-11-22) to `docs/design/SDK_LANDSCAPE.md`, `docs/design/RESEARCH_PRIMITIVE.md` and `docs/design/NEW_SERVER_DESIGN.md` §10 (research.md D13), exact text approved first
- [ ] T021 README's tools table and security paragraph, the `research` tool description in `src/server.rs`, and CLAUDE.md's runtime-configuration paragraph name `EXA_API_KEY` and `EXA_MAX_AGE_HOURS` and drop the removed variables (exact text approved first; README's configuration rows are done in T005 and T012)
- [ ] T022 CHANGELOG `Unreleased`: Changed (research on Exa, `sources[].truncated`, `search_cost_usd`) and Removed (Brave, the page fetcher, three variables, now startup errors)
- [ ] T023 Run the full gate (`cargo fmt --all -- --check`, `cargo clippy --all-features --all-targets -- -D warnings`, `cargo test --all-features --tests`)
- [ ] T024 Independent review of the branch diff by a fresh subagent; act on findings
- [ ] T025 Live acceptance (quickstart.md): three runs of `acceptance_research` with `EXA_API_KEY`, one at `deep`; record findings, sources, truncated sources, Exa cost and elapsed time in quickstart.md (SC-001, research.md D2)
- [ ] T026 Update the CLAUDE.md active-feature block for 059 (exact text approved first)

## Dependencies & Execution Order

- Phase 2 blocks US1 and US3: T001 → T002.
- US1: T003 → T004; T005 independent of T003/T004; T006 → T007 → T008 → T009 → T010.
- US2 is independent of US1 except that T012 and T009 both edit `src/server.rs`-adjacent config uses; do T012 after T009.
- US3 after US1: T013 → T014 (same file as T004); T015 → T016 → T017; T018 last.
- Polish after all stories; T023 before T024; T025 after T024; T026 last.

## Parallel Opportunities

- T005 (config) alongside T003/T004 (client): different files.
- T019, T020, T021, T022 are documentation in different files.

## Implementation Strategy

MVP is Phase 2 + US1 + US2: research works on Exa and nobody upgrades into a
silently missing tool. US3 then adds cost and retries. One commit per phase at
least, each after the full gate.
