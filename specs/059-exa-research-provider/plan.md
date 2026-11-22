# Implementation Plan: Exa replaces Brave for research

**Branch**: `059-exa-research-provider` | **Date**: 2026-11-22 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/059-exa-research-provider/spec.md`

## Summary

`research` finds sources with Exa and takes their text from Exa, so Parallax
no longer downloads or parses pages. One Exa `POST /search` per scoped angle
carries the search type (from depth) and the caller's domain constraints and
returns URLs; after dedup, the domain filter and the source cap, one
`POST /contents` per run fetches the kept sources' text with the freshness
limit and the per-source character cap, so text is paid for only on sources
used. That text feeds the unchanged extraction, grounding, verification and
synthesis phases. Brave, the page fetcher and their three
settings are removed; a still-set removed variable is a startup error. Exa's
reported cost per run is recorded in a new nullable `search_cost_usd` field
and exported as `parallax.search.cost_usd`. The result gains one field,
`sources[].truncated`.

## Technical Context

**Language/Version**: Rust 1.100.0 (MSRV 1.94)

**Primary Dependencies**: `reqwest` (existing) for the Exa client; removes
`rs-trafilatura` and `robotstxt`

**Storage**: SQLite; one additive nullable column
(`invocation_records.search_cost_usd`)

**Testing**: `cargo test`, wiremock doubles for Exa; live
`examples/acceptance_research.rs`, three runs

**Target Platform**: MCP stdio server (Windows, Linux, macOS)

**Project Type**: single Rust crate

**Performance Goals**: one provider request per angle plus one per run (the
contents request); no per-source requests

**Constraints**: off by default (no `EXA_API_KEY`, no research, no egress);
loud configuration errors; no `unwrap`/`expect` in production paths

**Scale/Scope**: `src/client/` (new `exa.rs`, `brave.rs` removed),
`src/traits/search.rs` (request and response types), `src/traits/fetcher.rs`
and `src/research/fetch.rs` removed, `src/research/extract.rs` (readability
removed), `src/research/pipeline.rs`, `src/research/contract.rs`
(`truncated`), `src/research/mod.rs` (meter), `src/config.rs`,
`src/config_facts/`, `src/main.rs` (`--help`), `src/server.rs`,
`src/telemetry.rs`, `src/storage/sqlite.rs`, `src/observability.rs`,
`tests/common/mod.rs`, `tests/integration.rs`,
`examples/acceptance_research.rs`, README, CHANGELOG, design corpus, 007
telemetry contract

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Note |
|---|---|---|
| I. Design-Corpus Fidelity | Pass, with amendment | `SDK_LANDSCAPE.md`, `RESEARCH_PRIMITIVE.md` and `NEW_SERVER_DESIGN.md` §10 name Brave and the hygiene fetcher; dated amendments in this change (research.md D13). |
| II. Constrained-Output Contract | Pass | No mode schema changes. The extraction prompt is unchanged; the page text it receives changes format (Exa's text for trafilatura's), so the live acceptance run gates the PR. |
| III. Compiler-Enforced Discipline | Pass | Removed variables become typed `ConfigError`s; no new lint exceptions. |
| IV. Seams, Composition, Tests | Pass | Exa sits behind `SearchProvider`; the `Fetcher` seam is removed with its only caller; tests first against wiremock. |
| V. Deterministic Over Probabilistic | Pass | The grounding gate is unchanged; truncation is a deterministic length rule. |
| VI. Capabilities Off By Default | Pass | Research and all egress stay off without `EXA_API_KEY`. |
| VII. Simplicity and Scope Discipline | Pass | Net code removal (fetcher, robots, readability); rerank, Jev and Tasks stay named follow-ups; unused Exa features listed with reasons (research.md D12). |

Post-design re-check: unchanged.

## Project Structure

### Documentation (this feature)

```text
specs/059-exa-research-provider/
├── plan.md              # This file
├── research.md          # Decisions D1-D14
├── data-model.md        # Request, response, record and config changes
├── quickstart.md        # Configure, startup errors, live acceptance
├── contracts/
│   └── exa-provider.md  # Operator surface, outbound requests, result, telemetry
├── checklists/
│   └── requirements.md
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
src/client/exa.rs          # ExaClient: POST /search and /contents, retries with Retry-After, cost
src/client/brave.rs        # removed
src/traits/search.rs       # search(SearchRequest) -> hits + cost; contents(ContentsRequest) -> pages with text + cost
src/traits/fetcher.rs      # removed
src/research/fetch.rs      # removed (hygiene fetcher, robots, private-address guard)
src/research/extract.rs    # readability removed; extraction prompt unchanged
src/research/pipeline.rs   # domains sent to search; one contents call for kept sources; cost metered
src/research/contract.rs   # SourceRef.truncated
src/research/mod.rs        # RunMeter: Exa cost sum + unknown flag
src/config.rs              # EXA_API_KEY, EXA_MAX_AGE_HOURS; removed vars are errors
src/telemetry.rs           # InvocationRecord.search_cost_usd
src/storage/sqlite.rs      # additive column
src/observability.rs       # parallax.search.cost_usd attribute
src/server.rs              # wiring; no per-run fetcher
```

**Structure Decision**: Keep the `SearchProvider` seam and widen it to carry
the per-run request options and return text and cost, so the pipeline stays
provider-neutral and tests stay off the network. Remove the `Fetcher` seam
rather than adapt it: with text coming from search, nothing would call it.

## Complexity Tracking

None: no principle is violated.
