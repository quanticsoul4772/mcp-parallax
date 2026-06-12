# Implementation Plan: Research Layer — Offloaded, Cited, Adversarially-Verified Answers

**Branch**: `004-research-layer` | **Date**: 2026-06-12 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/004-research-layer/spec.md`

## Summary

One new MCP tool, `research`, gated on `BRAVE_API_KEY` presence (catalog-honesty
pattern proven in 003). Five-phase pipeline: one scope call decomposes the
question into angles and falsifiable sub-questions; angle searches run
concurrently against Brave behind a new `SearchProvider` seam; each source
flows fetch→extract independently (hygiene-enforced `Fetcher` seam +
local readable-text extraction + one claim-extraction call per source);
deduplicated claims are verified by the **verify ensemble machinery with a
refute-biased prompt variant** (same schema, K passes from the depth tier —
research.md D3); synthesis is **server-assembled** —
the model writes only the prose answer and gap phrasing, while key findings,
disagreements, sources, and stats are built deterministically from pipeline
state, and a **pure grounding gate** validates every citation token against the
fetched-source map before anything returns. Budget/deadline ceilings stop new
work and synthesize early with `stopped_early`/`stop_reason` set.

## Technical Context

**Language/Version**: Rust (pinned stable via `rust-toolchain.toml`, MSRV 1.94)

**Primary Dependencies**: rmcp 1.7 (existing), reqwest (existing), tokio
(existing) + new: `rs-trafilatura` (local readable-text extraction),
`robotstxt` (Google-parser port) — see research.md D2/D5

**Storage**: existing SQLite via the `Storage` seam (invocation records only —
v1 has no research-specific persistence; caches deferred by spec)

**Testing**: cargo test — mockall seams (`SearchProvider`, `Fetcher`,
`ModelClient`), wiremock for the Brave client, in-process rmcp for
integration; live acceptance example (BRAVE_API_KEY present in dev env)

**Target Platform**: cross-platform stdio binary (Windows dev, Linux CI)

**Project Type**: single Rust crate — MCP server

**Performance Goals**: SC-003 — standard depth < 4 min wall clock, quick
< 90 s; bounded concurrency (RESEARCH_CONCURRENCY, default 8)

**Constraints**: budget/deadline are hard ceilings with graceful early
synthesis (FR-007); zero fabricated citations (FR-003, deterministic gate);
no raw page bodies cross the wire (FR-012); capability off without
BRAVE_API_KEY (FR-008)

**Scale/Scope**: depth tiers quick/standard/deep — up to 8 angles, 60
sources, K=3 verification votes per claim (exhaustive tier deferred)

## Constitution Check

*GATE: evaluated against constitution v1.0.0 before Phase 0; re-checked after
Phase 1 design.*

| Principle | Status | Notes |
|---|---|---|
| I. Design-corpus fidelity | PASS | Maps to `RESEARCH_PRIMITIVE.md` + `SDK_LANDSCAPE.md` §research (Brave + local extraction). Named deviations: v1 defers caches/Recall-hook/progress-notifications/exhaustive tier (spec Assumptions); claim spans dropped from extraction (research.md D4); dedup is normalized-text not embedding-cosine (D6). Each is recorded here and in research.md; no corpus amendment needed — the design doc marks these as open/tiered choices. |
| II. Constrained-output contract | PASS | All model hops use flat+closed schemas: scope `{angles[], sub_questions[]}`, extract `{claims[]}`, verify (existing mode, unchanged), synthesis `{answer, gaps[]}`. The nested response (findings/disagreements/sources) is MCP-side only — server-assembled, no model hop (003 D6 precedent). |
| III. Compiler discipline | PASS | No new unsafe, no stdout; extraction crates are pure-Rust (D2 verifies). |
| IV. Seams + tests | PASS | Two new seams: `SearchProvider`, `Fetcher` (both automocked). Extraction is a pure function. Verify reuses `ModelClient`. Whole pipeline testable offline. |
| V. Deterministic over probabilistic | PASS (strengthened) | Key findings, disagreements, support labels, confidence, and the grounding gate are all deterministic functions of pipeline state — the model writes only prose. This is a deliberate narrowing of the design doc's synthesis step, recorded in research.md D7. |
| VI. Capabilities off by default | PASS | Network egress (search + fetch) exists only when BRAVE_API_KEY is set; absent → tool absent from catalog, zero egress at construction. |
| VII. Simplicity / ≤500-line modules | PASS | Module split below keeps each file under target; `pipeline.rs` is the watch item — if it crosses 500, the phase functions split into `pipeline/` submodules. |

**Post-Phase-1 re-check**: PASS — contracts and data model introduce no new
violations; the wire response is nested but MCP-side only.

## Project Structure

### Documentation (this feature)

```text
specs/004-research-layer/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── research.tool.json
└── tasks.md             # Phase 2 output (/speckit-tasks — not this command)
```

### Source Code (repository root)

```text
src/
├── traits/
│   ├── search.rs         # NEW seam: SearchProvider — search(query, opts) → Vec<SearchHit>
│   └── fetcher.rs        # NEW seam: Fetcher — fetch(url) → FetchedPage
├── client/
│   └── brave.rs          # NEW thin Brave Search client (reqwest, wiremock-tested)
├── research/
│   ├── mod.rs            # internal types: ScopePlan, Claim, VerifiedClaim, Support
│   ├── contract.rs       # wire types (request/response; MCP-side, nested legal)
│   ├── fetch.rs          # HygieneFetcher: timeout/size/content-type/redirect/robots/per-domain politeness
│   ├── extract.rs        # readable text (rs-trafilatura) + claim-extraction model call
│   ├── verdict.rs        # PURE: support labels + per-claim/overall confidence
│   ├── grounding.rs      # PURE: deterministic grounding gate over citation tokens
│   ├── prompts.rs        # model-hop bundles: templates + flat schemas + registration
│   ├── settings.rs       # tier defaults + caller overrides + validation
│   ├── synthesis.rs      # phase 5: server-assembled findings + grounded synthesis
│   └── pipeline.rs       # five-phase orchestration, ceilings, early synthesis
├── config.rs             # + BRAVE_API_KEY (gate), FETCH_TIMEOUT_MS, RESEARCH_CONCURRENCY
├── server.rs             # + research #[tool] via run_recorded; catalog gating
├── error.rs              # + SearchProvider failure class ("search_provider")
└── telemetry.rs          # (no change — Brave bills per-request, not per-token; recorded at 0 token cost, latency/outcome as usual)

tests/integration.rs      # + gating, wiremock Brave+fetch round trip, ceiling tests
examples/acceptance_research.rs  # live acceptance (BRAVE_API_KEY + ANTHROPIC_API_KEY)
```

**Structure Decision**: single crate, new `research/` module family mirroring
`memory/` (contract.rs split learned in 003); two new trait seams next to the
existing four. Tier table and hygiene limits are constants in `research/mod.rs`;
only operator-relevant knobs become env vars.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| `pipeline.rs` crossed 500 lines during implementation | the five-phase spine plus ceiling/accounting logic | mitigated per the plan: prompts/schemas split to `prompts.rs`, settings to `settings.rs`, phase-5 assembly to `synthesis.rs`, tests to `pipeline_tests.rs`; the remaining spine is the orchestration that reads best unbroken |
