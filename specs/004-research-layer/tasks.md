---

description: "Task list for Research Layer — Offloaded, Cited, Adversarially-Verified Answers"
---

# Tasks: Research Layer — Offloaded, Cited, Adversarially-Verified Answers

**Input**: Design documents from `/specs/004-research-layer/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: REQUIRED (Constitution Principle IV) — mockall seams
(`SearchProvider`/`Fetcher`/`ModelClient`), wiremock for Brave + fetch
endpoints, in-process rmcp for integration; no network/disk state. The
acceptance example is manual-run live spend.

## Format: `[ID] [P?] [Story?] Description`

## Phase 1: Setup

- [X] T001 [P] Config in src/config.rs: optional `BRAVE_API_KEY` (presence = capability on, filtered non-empty like `VOYAGE_API_KEY`), `FETCH_TIMEOUT_MS` (default 10000), `RESEARCH_CONCURRENCY` (default 8, 1..=32 with a named MAX const); parse-or-error, never silent fallback; unit tests incl. range violations (research.md D10)
- [X] T002 [P] Error taxonomy in src/error.rs: `AppError::SearchProvider(String)` ↔ `Outcome::SearchProvider` ("search_provider"), mirroring 003's embedding_provider exactly; distinct-message + round-trip tests; add the value to specs/001-core-layer/contracts/invocation-record.schema.json enum (addition)

## Phase 2: Foundational

- [X] T003 [P] Spike S1 in examples/spike_extract.rs (offline): add `rs-trafilatura` dep; extract readable text from 3 bundled HTML fixtures under tests/fixtures/ (article, docs page, boilerplate-heavy) — assert non-empty main text and boilerplate exclusion; validates research.md D2 before the pipeline depends on it (fallback crates named in D2 if quality fails)
- [ ] T004 [P] Spike S2 in examples/spike_brave.rs (live, one request, manual-run): assert Brave `web.results[].{url,title,description}` response shape with the dev key; pins the deserializer the wiremock tests mirror (research.md D1) — **written; blocked on a valid BRAVE_API_KEY (the configured key returns SUBSCRIPTION_TOKEN_INVALID; issue a new one at the Brave Search API dashboard, then run)**
- [X] T005 [P] Seams: src/traits/search.rs (`SearchProvider::search(query, count) -> Vec<SearchHit>`, `SearchHit {url, title, snippet}`) and src/traits/fetcher.rs (`Fetcher::fetch(url) -> FetchedPage`) with mockall automock + mock-contract tests; register in src/traits/mod.rs
- [X] T006 Thin Brave client in src/client/brave.rs implementing SearchProvider: GET /res/v1/web/search, X-Subscription-Token auth, retry/backoff/timeout mirroring the Voyage client (429/5xx retry, timeout terminal, other 4xx → SearchProvider); with_base_url + with_backoff_base_ms; wiremock tests for happy path, auth header, retries-exhausted, timeout, terminal 4xx, out-of-contract empty body (depends on T002, T005)
- [X] T007 HygieneFetcher in src/research/fetch.rs implementing Fetcher: per-fetch timeout, redirect cap 5, streaming 2 MB size cap (never trusts Content-Length), content-type allowlist, per-domain politeness (one in-flight + spacing), robots.txt via `robotstxt` crate (fail-open on robots fetch error, fail-closed on explicit disallow), domains_deny absolute before connection, domains_allow restricting fetches to listed registrable domains when present; wiremock tests for each guard incl. oversized-body cutoff, robots disallow, and a URL outside domains_allow never fetched (research.md D5; depends on T005)
- [X] T008 [P] Internal types + pure verdict logic in src/research/mod.rs (ScopePlan, SearchHit dedup key, Claim with normalized-text dedup merge, VerifiedClaim, Support enum, tier table constants from research.md D8) and src/research/verdict.rs (`support(passes, agreement, verdict, n_sources)` with the **order-sensitive** D7 mapping — contested band (winning share < 2/3) checked BEFORE the aggregate verdict, because the verify ensemble resolves ties to refuted; `claim_confidence()`, `overall_confidence()` per data-model.md §4); property tests pinning the mapping order incl. K=2 1–1 and K=3 2–1 landing contested (never refuted), K=1 never contested, and the coverage penalty
- [X] T009 [P] Grounding gate in src/research/grounding.rs (pure): parse `[sN]` tokens from answer prose, validate finding sources resolve, prune uncited sources, return structured Violations for the retry prompt; tests for fabricated token, unresolved finding source, uncited-source pruning, clean pass (data-model.md §4)
- [X] T010 Wire types in src/research/contract.rs matching contracts/research.tool.json (ResearchParams/Constraints/ResearchResult/KeyFinding/Disagreement/SourceRef/Stats; schemars derives; nested legal — MCP-side only); contract-sync test both directions (003 pattern; depends on T008)
- [X] T011 Extraction in src/research/extract.rs: readable-text via rs-trafilatura (pure fn over FetchedPage) + per-source claim-extraction model call with flat schema `{claims: [string]}` (≤ 12, span dropped — research.md D4); tests with MockModelClient incl. empty-text short-circuit and claim cap (depends on T003, T008)

## Phase 3: US1 — ask a question, get a verified page back (P1) 🎯 MVP

- [X] T012 [US1] Pipeline in src/research/pipeline.rs: input validation at entry (question via check_text naming INPUT_MAX_CHARS, constraint ranges — FR-010), then five phases — scope call (flat `{angles[], sub_questions[]}`, weaving caller `focus` entries into the prompt — FR-001), concurrent angle searches with URL dedup barrier, per-source fetch→extract via JoinSet + RESEARCH_CONCURRENCY semaphore (no cross-source barrier), normalized-text claim dedup (D6), per-claim verification via the verify ensemble machinery with the refute-biased prompt variant and tier K (D3), verdict mapping + server-assembled findings/disagreements/sources/stats (D7), synthesis call (flat `{answer, gaps[]}`, local bounds: answer ≤ 8000 chars), grounding gate with one retry then demotion to gaps + stop_reason "grounding"; unit tests through mock seams: happy path with citations resolving, a focus string reaching the scope prompt, single-source fetch failure degrades and counts (FR-013), refuted claim absent from body, contested claim in disagreements, grounding retry then demotion, token usage summed (split into research/pipeline/ submodules if >500 lines; depends on T006–T011)
- [X] T013 [US1] Server wiring in src/server.rs: `research` #[tool] via run_recorded (model = anthropic model, research.md D10), ResearchDeps composed when BRAVE_API_KEY present (Brave client + HygieneFetcher + ModelClient + clock + config), catalog gating via remove_route("research") when absent, get_info instructions mention research only when enabled; unit tests: catalog with/without key, one record per call with attribution (depends on T012)
- [X] T014 [US1] Integration round trip in tests/integration.rs: wiremock serving /res/v1/web/search + fetchable HTML pages + /v1/messages (scope/extract/verify/synthesis routed by body matchers); full research call through the real rmcp client — structured result validates against contracts/research.tool.json, every cited id resolves in sources, no raw page body crosses the wire (FR-012), exactly one success record (depends on T013)

## Phase 4: US2 — depth tiers and hard ceilings (P2)

- [X] T015 [P] [US2] Tier scaling + constraint override unit tests in src/research/pipeline.rs tests: quick vs deep angle/source/K counts from the tier table; explicit constraints (max_sources, domains_allow, domains_deny, budget, deadline) override tier defaults (FR-006); domains_deny never reaches the Fetcher (mock asserts times(0) for denied domain) and a hit outside domains_allow is never fetched
- [X] T016 [US2] Ceiling enforcement in src/research/pipeline.rs: budget/deadline checked before each new unit of work; on trip stop spawning, drain in-flight, synthesize over verified claims with stopped_early + stop_reason; unit tests with MockTimeProvider (deadline) and tiny budget (token sum); integration test: induced tiny deadline returns well-formed early-synthesized result, not an error (SC-004; depends on T012)

## Phase 5: US3 — gated capability with observability parity (P3)

- [X] T017 [P] [US3] Gating integration tests in tests/integration.rs: without BRAVE_API_KEY the catalog is exactly the prior tools and construction performs zero research egress; with the key `research` appears with the contracted description/schemas; update the stdio smoke test to env_remove BRAVE_API_KEY (the dev machine carries a real key); SC-005 gate: full pre-existing suite passes unchanged
- [X] T018 [P] [US3] Failure parity in tests/integration.rs: Brave outage (all angles fail) surfaces `[search_provider]` with one record; partial angle failure degrades and counts (FR-013); empty/oversized question rejected pre-provider as `[invalid_input]` naming INPUT_MAX_CHARS (FR-010); cancellation mid-run records cancelled (FR-011)

## Phase 6: Polish

- [ ] T019 [P] Acceptance example examples/acceptance_research.rs (live: BRAVE_API_KEY + ANTHROPIC_API_KEY): ≥6 live questions asserting zero fabricated citations + schema conformance (SC-001/SC-002), latency per tier (SC-003), a tiny-ceiling run (SC-004), a false-premise question (SC-007); record results in specs/004-research-layer/quickstart.md
- [ ] T020 [P] Docs: README.md + CLAUDE.md status and env tables (research capability off by default; BRAVE_API_KEY/FETCH_TIMEOUT_MS/RESEARCH_CONCURRENCY); repo layout gains research/ + new seams; note the named cost inexactness — research records carry summed LLM tokens only, Brave's per-request fee is not in cost_usd (plan.md telemetry note)
- [ ] T021 Full gate (`cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`) + code-reviewer and design-reviewer agent passes over the branch diff + apply findings

## Dependencies

T001/T002 → T006; T003 → T011; T005 → T006/T007; T008 → T010/T011;
T006+T007+T009+T010+T011 → T012 → T013 → T014; T012 → T015/T016;
T013 → T017/T018; T019/T020 after stories; T021 last. T004 is independent
(manual live spike, informs T006's deserializer).

## Parallel opportunities

- Phase 1: T001 ∥ T002. Phase 2 start: T003 ∥ T004 ∥ T005 ∥ T008 ∥ T009;
  then T006 ∥ T007 ∥ T010 ∥ T011.
- After T012: T015 ∥ T016 prep; after T013: T014 ∥ T017 ∥ T018.
- Polish: T019 ∥ T020.

## Implementation strategy

MVP = Phase 1 + 2 + US1 (T001–T014): a working, gated, grounded research tool
at standard depth with full record parity at the unit level. US2 adds the
tier/ceiling guarantees, US3 the gating/parity integration proof. Each story
is independently testable through the mock seams; only T004/T019 spend live
money and are manual-run.
