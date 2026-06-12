# Research: Research Layer (004)

**Date**: 2026-06-12. All Technical Context unknowns resolved below. Sources:
`docs/design/RESEARCH_PRIMITIVE.md`, `docs/design/SDK_LANDSCAPE.md` §research
(web-grounded 2026-06), crates.io searches run 2026-06-12, and the 003
implementation experience.

## D1 — Search provider: Brave behind a `SearchProvider` trait

**Decision**: Brave Search API (`GET https://api.search.brave.com/res/v1/web/search`,
`X-Subscription-Token` header, `q`/`count` params) as the only v1 provider,
implemented as `client/brave.rs` behind a new `SearchProvider` trait.
`BRAVE_API_KEY` presence gates the whole capability.

**Rationale**: SDK_LANDSCAPE's benchmarked pick — top agent score, lowest
latency (~669 ms), which matters because the pipeline fans out N searches; a
31-char `BRAVE_API_KEY` is already present in the dev environment, so live
acceptance is feasible. The trait seam (the design doc's own call) keeps
Tavily/Exa drop-in.

**Alternatives considered**: Tavily (answers-with-citations, but 5 s+ on
research tiers — latency × N angles hurts); Exa (best deep retrieval, second
provider cost). Multiple providers in v1 — rejected, YAGNI.

**Retry/error policy**: mirror the Anthropic/Voyage clients — 429/5xx +
transport retry with backoff, per-request timeout terminal, other 4xx terminal
→ new `AppError::SearchProvider` / outcome `search_provider` (the
`embedding_provider` pattern; contract enum gains the value).

## D2 — Extraction: `rs-trafilatura`, local, pure-Rust

**Decision**: `rs-trafilatura` 0.2.x for readable-text extraction (HTML →
main text), with the size/content-type guards applied *before* extraction.

**Rationale**: the landscape's pick — local (no second paid API),
page-type-aware. crates.io search (2026-06-12) confirms `rs-trafilatura`
0.2.2 published; siblings exist (`trafilatura` 0.3.0, `kawat`, `readex`,
`article_scraper` 2.3.1) if quality disappoints. Spike S1 (below) validates
compile + extraction quality on fixture HTML before the pipeline is built.

**Alternatives considered**: Firecrawl (managed, top quality — second paid
dependency, against the landscape's own preference); `article_scraper`
(fivefilters configs + Readability — heavier, pulls libxml).

## D3 — Verification: reuse the existing verify ensemble unchanged

**Decision**: per-claim verification calls the existing `verify` mode logic
(`modes::verify::run`) with the claim text and a context line naming the
source domain/title, K passes from the depth tier. No new mode schema.

**Rationale**: the verify mode *is* K stance-blind refute-capable passes with
agreement-derived confidence — exactly the design doc's §4 shape. Reuse keeps
one verification implementation (Principle VII) and one schema (Principle II).
The refute-bias lives in the existing stance-blind prompt; "default to refuted
if uncertain" is already its failure direction (a refutation must name the
error).

**Alternatives considered**: a bespoke per-lens verifier mode
(deep/exhaustive's diverse lenses) — deferred with the exhaustive tier; K
identical-prompt passes are the v1 design point.

## D4 — Extraction output: flat `{claims: []}`, spans dropped (named deviation)

**Decision**: the per-source claim-extraction call returns the flat+closed
schema `{claims: [string]}` (bounded count). The design doc's per-claim
`span` field is **dropped in v1**.

**Rationale**: Principle II requires flat+closed schemas on model hops; an
array of `{claim, span}` objects is not flat. The span's purpose (grounding)
is served structurally instead: claim→source binding comes from the call
itself (one extraction call per source), and the grounding gate validates at
claim-source granularity. Named deviation from RESEARCH_PRIMITIVE.md §2(3).

**Alternatives considered**: parallel arrays `{claims[], spans[]}` — fragile
index coupling for a field nothing consumes in v1; relaxing the flat
invariant — rejected, constitutional.

## D5 — Fetch hygiene: reqwest + `robotstxt` + hard guards

**Decision**: `research/fetch.rs` implements the `Fetcher` seam with: per-fetch
timeout (`FETCH_TIMEOUT_MS`, default 10 000), redirect cap (5), response size
cap (2 MB, enforced while streaming — not trusting Content-Length),
content-type allowlist (`text/html`, `text/plain`, `application/xhtml+xml`),
per-domain politeness (one in-flight request per domain + minimum spacing),
robots.txt respect via the `robotstxt` crate (0.3.x, Google-parser port;
fetched once per domain per run, fail-open on robots fetch error but
fail-closed on explicit disallow), and absolute `domains_deny` enforcement
before any connection.

**Rationale**: every guard is from RESEARCH_PRIMITIVE.md §6; `robotstxt` is a
small pure dependency (crates.io confirmed) so corpus fidelity costs little.
A rejected/failed fetch drops the source and counts it (FR-009/FR-013).

**Alternatives considered**: skipping robots.txt for v1 — rejected, it is in
the corpus and cheap; headless rendering for JS pages — out of scope by spec
assumption (dropped + counted).

## D6 — Claim dedup: normalized-text, not embedding-cosine (named deviation)

**Decision**: dedup claims by normalized text (lowercase, whitespace/punctuation
collapse) merging source lists on collision. No embedding dedup in v1.

**Rationale**: the design doc suggests embedding-cosine + merge, but the
research capability must not couple to `VOYAGE_API_KEY` (independent gates),
and an LLM merge pass would need a non-flat schema. Normalized-text dedup is
deterministic (Principle V), free, and conservative — near-duplicates that
survive merely cost extra verification votes, they cannot corrupt results.
Named deviation from RESEARCH_PRIMITIVE.md §2(4)1; revisit when the memory and
research layers are both on and an embedding dedup can be conditional.

## D7 — Synthesis: server-assembled response; the model writes only prose

**Decision**: `key_findings`, `disagreements`, `sources`, support labels,
confidences, and `stats` are assembled **deterministically** from pipeline
state by pure functions (`verdict.rs`). The synthesis model call receives the
verified claims (with ids and support labels) and returns the flat schema
`{answer: string, gaps: [string]}`, citing sources inline as `[s3]` tokens.
The **grounding gate** (`grounding.rs`, pure) then validates: every `[sN]`
token in `answer` resolves to a fetched source; every key finding's sources
resolve; no listed source is uncited (uncited sources are pruned from the
list, not errors). Violation → one retry with the violation named; second
violation → offending content demoted to `gaps`, `stop_reason: "grounding"`.

**Rationale**: this narrows the design doc's synthesis step in the
deterministic direction (Principle V): the model cannot fabricate a finding,
a label, or a confidence because it never emits them — it can only fabricate
a citation token, which a string check catches. It also keeps the synthesis
schema flat (Principle II) where the doc's illustrative response would not be.

**Verdict mapping** (pure, `verdict.rs`): from the verify run (majority
verdict, agreement confidence) + independent source count `n`:
refuted-majority → `refuted` (dropped from body, counted); supported-majority
with `n ≥ 2` → `confirmed`; supported-majority with `n = 1` → `unverified`
(never stated as fact); near-tie agreement (< quorum margin) → `contested`
(surfaced in `disagreements`). Overall confidence = coverage-weighted mean of
finding confidences, penalized by unanswered sub-questions (design §4.2).

## D8 — Depth tiers and ceilings (constants, constraints override)

| tier | angles | max_sources | verify K | default deadline | default budget |
|---|---|---|---|---|---|
| quick | 3 | 8 | 1 | 90 s | 40k tok |
| standard (default) | 5 | 25 | 2 | 240 s | 120k tok |
| deep | 8 | 60 | 3 | 480 s | 350k tok |

Exhaustive deferred (spec assumption). Explicit `constraints` always override
tier defaults (FR-006). Ceiling enforcement: the pipeline checks
budget/deadline before spawning each new unit of work; on trip, it stops
spawning, drains in-flight work (bounded by the per-call timeout), and
synthesizes over verified claims with `stopped_early: true` and
`stop_reason: "budget" | "deadline"`. Token accounting sums usage from every
model call in the run (the verify runs already return usage; scope/extract/
synthesis calls do too).

## D9 — Concurrency

Bounded by a semaphore: `RESEARCH_CONCURRENCY` (default 8, 1..=32) caps
concurrent fetches and concurrent extraction/verification calls. Search phase:
all angles concurrent (N ≤ 8). Fetch+extract: per-source pipeline with no
cross-source barrier (`tokio::JoinSet`). Verify: per-claim fan-out, K votes
concurrent within the existing verify run. Barriers exactly where the design
doc puts them: after search (URL dedup) and before synthesis.

## D10 — Config & gating

New env vars (all parse-or-error like 003, never silent fallback):

- `BRAVE_API_KEY` (optional; presence enables the `research` tool — absent ⇒
  not in catalog, zero research egress at construction)
- `FETCH_TIMEOUT_MS` (default 10 000)
- `RESEARCH_CONCURRENCY` (default 8, 1..=32)

`INPUT_MAX_CHARS` bounds the question (existing). Record attribution: research
invocations record the Anthropic model (the LLM calls dominate cost; Brave is
per-request, not per-token — recorded tokens are the summed LLM usage).

## Spikes (run before/with early tasks)

- **S1 — extraction quality** (offline): `rs-trafilatura` against 3 bundled
  HTML fixtures (article, docs page, boilerplate-heavy page) — asserts
  non-empty main text and boilerplate exclusion. Validates D2 before the
  pipeline depends on it; fallback crates named in D2.
- **S2 — Brave response shape** (live, one request): assert the
  `web.results[].{url,title,description}` shape against the real endpoint
  with the dev key; pins the deserializer the wiremock tests then mirror.
