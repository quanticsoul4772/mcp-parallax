---

description: "Task list for Memory Layer — Recall Corrective with Verified-Before-Stored Memory"
---

# Tasks: Memory Layer — Recall Corrective with Verified-Before-Stored Memory

**Input**: Design documents from `/specs/003-memory-layer/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: REQUIRED (Constitution Principle IV) — MockEmbedder/mockall,
wiremock for the Voyage client, in-process rmcp for integration; no
network/disk state. The acceptance example is manual-run live spend.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [X] T001 [P] Config in src/config.rs: optional `VOYAGE_API_KEY` (presence = capability on), `VOYAGE_MODEL` (default voyage-4), `MEMORY_RECALL_LIMIT` (default 5, 1..=20), generic `INPUT_MAX_CHARS` (default 50000) with `VERIFY_MAX_CLAIM_CHARS` honored as fallback alias and used by verify/unstick error text going forward; unit tests incl. alias precedence (research.md D7)
- [X] T002 [P] Error taxonomy in src/error.rs: `AppError::EmbeddingProvider` ↔ `Outcome::EmbeddingProvider` ("embedding_provider"); distinct-message + round-trip tests; add the value to specs/001-core-layer/contracts/invocation-record.schema.json enum (addition)

## Phase 2: Foundational

- [X] T003 [P] Spike S1 in examples/spike_bruteforce.rs (no key): f32 BLOB round-trip through the sqlx pool + brute-force cosine timing at 5k×1024 — assert < 50 ms; this validates the named sqlite-vec deviation (research.md S1)
- [X] T004 [P] `Embedder` seam in src/traits/embedder.rs: embed_document/embed_query returning the f32 vector + input_tokens with mockall automock
- [X] T005 Thin Voyage client in src/client/voyage.rs implementing Embedder: input_type document/query, retry/backoff/timeout mirroring the Anthropic client, failures → EmbeddingProvider/Timeout/RetriesExhausted; wiremock tests for happy path, input_type correctness, 5xx exhaustion, timeout (depends on T002, T004)
- [X] T006 [P] Memory types + ranking in src/memory/mod.rs and src/memory/ranking.rs: Memory/Kind/Trust types; pure scoring (cosine + 0.02×2^(−age_days/30)) and ε=0.05 trust-tier partition; property tests pinning FR-004's three clauses (data-model.md §3)
- [X] T007 Memory store in src/memory/store.rs: `memories` table idempotent migration on SqliteStorage, insert/fetch_all/delete_by_id, f32↔BLOB encoding, embedding_model column; in-memory SQLite tests incl. persistence of every Trust value and forget-by-id (depends on T006)
- [X] T008 Amend docs/design/SDK_LANDSCAPE.md §memory: brute-force v1 decision recorded, sqlite-vec moved to the scale path with the unsafe/workspace finding (Constitution I, same-change amendment)

## Phase 3: US1 — save now, recall later (P1) 🎯 MVP

- [X] T009 [P] [US1] Unit tests in src/memory/tools.rs test module (MockEmbedder + in-memory store): save embeds with document type and persists; recall embeds with query type, ranks, respects kind filter and limit; empty store → empty result; empty/oversized inputs rejected before any embed call
- [X] T010 [US1] Tool logic in src/memory/tools.rs: save (first-hand path), recall, forget; schemas matching contracts/ (schemars types; nested recall output per D6); contract-sync tests (depends on T009 red, T005, T007)
- [X] T011 [US1] Server wiring in src/server.rs: build Embedder + memory tools only when the key is present (D5 — router filtering, mechanism per rmcp API), three `#[tool]` methods via run_recorded; integration round trip in tests/integration.rs with a wiremock Voyage endpoint: save→recall returns the saved memory top-1, one record per call with correct tool attribution (depends on T010)

## Phase 4: US2 — verified-before-stored (P2)

- [X] T012 [P] [US2] Trust unit tests in src/memory/tools.rs: first-hand → first_hand without any verify call (mock times(0)); external without verify → untrusted; external+verify with supporting ensemble → verified; refuting ensemble → save rejected carrying findings (ValidationFailure class)
- [X] T013 [US2] Verify-at-save in src/memory/tools.rs via the existing verify mode/registry; integration test: untrusted vs first-hand ranking at equal relevance; recall labels trust (depends on T012 red, T011)

## Phase 5: US3 — gating + parity (P3)

- [X] T014 [P] [US3] Gating integration tests in tests/integration.rs: without key the catalog equals exactly ["unstick","verify"] and zero Voyage connections occur (no wiremock server needed — construction must not touch the network); with key the catalog adds the three tools; SC-005 gate: full pre-existing suite passes unchanged
- [X] T015 [P] [US3] Failure parity + forget semantics in tests/integration.rs: Voyage outage surfaces `[embedding_provider]` with one record; forget removes → recall never returns it (incl. across a store reopen); unknown id → distinct not-found

## Phase 6: Polish

- [X] T016 [P] Acceptance example examples/acceptance_memory.rs (live: VOYAGE_API_KEY + ANTHROPIC_API_KEY): 12 saves + 10 paraphrased queries (SC-001 ≥9/10 top-3, ≥7/10 top-1), trust scenarios (SC-003), latency (SC-004); record results in quickstart.md
- [X] T017 [P] Docs: README/CLAUDE.md status + env tables (memory capability, off by default)
- [X] T018 Full gate + code-reviewer and design-reviewer agent passes over the branch diff

## Dependencies

T001/T002 → T005; T003/T004/T006/T008 parallel after setup; T006 → T007;
T005+T007 → T010 → T011 → T013/T014/T15; T009/T012 (tests-first) precede their
implementations; T016/T017 after stories; T018 last.
