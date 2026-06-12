# Feature Specification: Research Layer — Offloaded, Cited, Adversarially-Verified Answers

**Feature Branch**: `004-research-layer`

**Created**: 2026-06-12

**Status**: Draft

**Input**: User description: "Research layer (the Research primitive, RESEARCH_PRIMITIVE.md): offload a research question to a separate budget; get back a short, cited, adversarially-verified answer — not 15 articles. One new MCP tool `research` running the five-phase pipeline: scope → N parallel searches → fetch+extract per source with no barrier → adversarial verification fan-out per deduped claim → compact synthesis with inline citations, surfaced disagreements, and honest gaps. Hard grounding gate: every claim traces to a fetched source; no fabricated citations ever. Depth tiers scale fan-out/rigor; budget/deadline ceilings trigger graceful early synthesis with stopped_early set — no silent truncation. Network egress gated off by default. Fetch hygiene. v1 defers: result/source caches, the Recall write-back hook, MCP progress notifications."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Ask a question, get a verified page back (Priority: P1)

A calling model (or its operator) has a question whose answer lives on the web.
Researching it in-context would mean many searches, many page reads, and
verification anchored on the same context that produced the claims — blowing up
the caller's window and serializing what should be parallel. Instead the caller
hands the question to `research` and gets back one compact answer: an executive
synthesis, key findings each labeled with its verification standing and
citations, places where credible sources disagree (surfaced, not resolved), and
an honest list of what could not be answered. Every citation resolves to a
source that was actually fetched during the run.

**Why this priority**: This is the product. The offload — separate budget,
parallel fan-out, independent verification, compact return — is the entire
value; nothing else in the feature matters without it.

**Independent Test**: With a search credential configured, call `research` with
a factual question. Verify the answer is compact (no raw page content), every
key finding carries at least one citation, every citation id resolves to an
entry in the returned source list, and the stats account for everything found,
fetched, and dropped.

**Acceptance Scenarios**:

1. **Given** a configured search provider, **When** `research` is called with a
   factual question at default depth, **Then** the response contains a synthesis
   answer, key findings each with a support label (confirmed / contested /
   refuted / unverified) and at least one citation, a source list with URL and
   title, and run statistics — and contains no raw page bodies or verifier
   transcripts.
2. **Given** a completed run, **When** the citations are checked, **Then** every
   cited source id resolves to a listed source that was fetched during this run,
   and no listed source is uncited dead weight.
3. **Given** a question that touches a genuinely contested topic, **When** the
   run completes, **Then** conflicting positions appear under disagreements with
   their respective sources, and the answer does not silently pick a winner.
4. **Given** a question the web cannot answer, **When** the run completes,
   **Then** the unanswerable parts appear under gaps, and the answer says so
   rather than fabricating.
5. **Given** a claim refuted by a majority of its independent verification
   passes, **When** the answer is synthesized, **Then** that claim is absent
   from the answer body and counted in the run statistics.

---

### User Story 2 - Depth tiers and hard ceilings (Priority: P2)

The caller scales rigor with one knob: a quick look, a standard pass, or a deep
investigation. Budget and deadline ceilings are enforced, not advisory — when
either is hit mid-run, the tool synthesizes early over whatever has been
verified so far and says so plainly: a stopped-early flag, the reason, and the
unfinished questions listed as gaps. The response never silently narrows.

**Why this priority**: Cost control and honesty about partial work. Without
enforced ceilings the tool is unusable in practice (an open-ended web run can
burn unbounded tokens and minutes); without honest early-stop reporting, a
partial answer masquerades as a complete one.

**Independent Test**: Run the same question at two depths and verify the deeper
run examines more angles and sources. Force a tiny budget or deadline and verify
the response still returns a well-formed answer with stopped-early set and the
stop reason named.

**Acceptance Scenarios**:

1. **Given** the same question at a lower and a higher depth, **When** both
   complete, **Then** the higher depth reports more search angles and more
   sources examined in its statistics.
2. **Given** a deadline (or budget) ceiling set low enough to interrupt the
   run, **When** the ceiling is hit, **Then** the tool returns a well-formed
   response synthesized from claims verified so far, with the stopped-early flag
   set, the reason named, and unfinished sub-questions listed under gaps — not
   an error and not a silently truncated answer.
3. **Given** explicit constraints (source cap, denied domains), **When** the
   run executes, **Then** the source cap is honored, denied domains are never
   fetched, and the statistics reflect the actual counts.

---

### User Story 3 - Gated capability with observability parity (Priority: P3)

An operator who has not configured a search credential sees no research tool in
the catalog at all — same catalog honesty as the memory layer. Network egress
is off by default. When the capability is on, every invocation (success or any
failure) leaves exactly one invocation record with the same cost/latency/outcome
accounting as every other tool, and failures surface as distinct named classes.

**Why this priority**: Constitution requirements (capabilities off by default,
observability parity) — necessary for operational trust but worthless without
US1.

**Independent Test**: Without the credential, list tools and verify the catalog
is exactly the pre-existing set and the full pre-existing test suite passes
unchanged. With it, verify the research tool appears and each call leaves
exactly one correctly attributed record.

**Acceptance Scenarios**:

1. **Given** no search credential, **When** the server starts and tools are
   listed, **Then** `research` is absent from the catalog and no network egress
   related to research occurs.
2. **Given** the credential is configured, **When** tools are listed, **Then**
   `research` appears with its contracted schema.
3. **Given** any research invocation — success, provider outage, invalid input,
   timeout, or cancellation — **When** it terminates, **Then** exactly one
   invocation record exists with the correct outcome class, and the error (if
   any) names its class distinctly.

---

### Edge Cases

- Searches return zero usable candidates → the response reports an honest gap;
  it never fabricates sources or answers from model memory.
- Every source for a sub-question fails to fetch (unreachable, paywalled,
  oversized, wrong content type) → that sub-question is reported as a gap; the
  failed fetches are counted, and the run continues for other sub-questions.
- A single source fails to fetch → dropped and counted; one bad URL never fails
  the run.
- All verifiable claims for the question are refuted → the answer states that
  the question's premise found no support, with the refutations reflected in
  statistics; nothing refuted is asserted.
- The question is empty, whitespace-only, or exceeds the configured input
  bound → rejected before any provider call with the invalid-input class.
- The search provider is down or rate-limited mid-run → bounded retries; if the
  whole search phase fails, the invocation fails with a distinct provider error
  class; if only some angles fail, the run continues and the loss is counted.
- The synthesis cites a source that was not fetched or makes an unsupported
  claim → the grounding gate rejects it and retries once with the violation fed
  back; on second failure the offending claims are demoted to gaps and the
  stopped-early reason says so. The tool never returns an ungrounded claim.
- Conflicting explicit constraints and tier defaults (e.g. deep depth but a
  10-source cap) → explicit constraints always win over tier defaults.
- A denied domain appears in search results → never fetched, under any
  circumstance.
- The caller cancels mid-run → in-flight work stops, one record with the
  cancelled outcome.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide a `research` capability that accepts a
  required natural-language question plus optional depth, focus angles, and
  constraints (maximum sources, denied domains, allowed domains, token budget,
  deadline), and returns a compact structured answer: synthesis, key findings,
  disagreements, gaps, sources, and run statistics.
- **FR-002**: The system MUST execute research as five phases — scope the
  question into search angles and falsifiable sub-questions; run the angle
  searches concurrently; fetch and extract each source independently (no
  cross-source barrier) into falsifiable claims tied to their sources; verify
  each deduplicated claim with multiple independent refute-biased passes; and
  synthesize a compact cited answer — with concurrency bounded server-side.
- **FR-003**: Every key finding MUST carry at least one citation, every cited
  source id MUST resolve to a source fetched during the run, and no listed
  source may be uncited. A grounding gate MUST validate this before returning;
  on violation it retries once with the violation described, then demotes the
  offending claims to gaps. The system MUST NOT return fabricated citations or
  ungrounded claims under any circumstance.
- **FR-004**: Each verified claim MUST be labeled confirmed, contested,
  refuted, or unverified, derived from its verification votes and source
  agreement. Refuted claims MUST be excluded from the answer body (and
  counted); contested claims MUST surface under disagreements with positions
  and sources, never silently resolved; unverified claims MUST NOT be stated
  as fact.
- **FR-005**: The response MUST report a confidence value derived from
  verification agreement and coverage of the scoped sub-questions — an answer
  that settled few of its sub-questions cannot report high confidence
  regardless of how well-supported those few are.
- **FR-006**: Depth tiers MUST scale the number of search angles, the source
  cap, and the verification votes per claim, with documented defaults per
  tier. Explicit caller constraints MUST override tier defaults.
- **FR-007**: Token-budget and deadline ceilings MUST be enforced, not
  advisory: hitting either stops new work and synthesizes early over claims
  verified so far, returning a well-formed response with a stopped-early flag,
  a named stop reason, and unfinished sub-questions as gaps. The system MUST
  NOT silently truncate: run statistics MUST honestly account for sources
  found vs fetched, claims extracted vs verified vs dropped, tokens, and
  elapsed time.
- **FR-008**: The research capability MUST be absent from the tool catalog when
  no search-provider credential is configured (presence of the credential
  enables it), and constructing the server without the credential MUST NOT
  perform any network activity — the same catalog-honesty pattern as the
  memory layer.
- **FR-009**: Fetching MUST enforce hygiene: per-fetch timeout, response size
  cap, content-type guard, redirect cap, and a per-domain politeness limit.
  Denied domains MUST never be fetched. A failed or rejected fetch is dropped
  and counted, never fatal to the run.
- **FR-010**: The question MUST be validated before any provider call:
  non-empty after trimming and within the configured input bound, with
  violations rejected under the invalid-input class naming the configured
  limit.
- **FR-011**: Every invocation MUST leave exactly one invocation record on
  every exit path (success, each failure class, cancellation) with cost,
  latency, token, and outcome accounting consistent with the existing tools.
  Search-provider failures MUST surface as a distinct named outcome class,
  parallel to the existing embedding-provider class.
- **FR-012**: The response MUST stay compact: no raw fetched page content, no
  verifier transcripts, and no per-search result listings cross the wire —
  sources carry identity (URL, title, fetch time) only.
- **FR-013**: Failures of individual items — one search angle, one source
  fetch, one extraction, one verification pass — MUST degrade the run, not
  fail it: the item is dropped after bounded retries and counted in run
  statistics.

### Key Entities

- **Research request**: the question; optional depth tier, focus angles, and
  constraints (max sources, allowed/denied domains, token budget, deadline).
- **Research answer**: the synthesis text; overall confidence; key findings;
  disagreements; gaps; sources; run statistics.
- **Key finding**: one claim with its support label (confirmed / contested /
  refuted / unverified), post-verification confidence, and citation ids.
- **Disagreement**: one contested claim with the conflicting positions and the
  sources backing each.
- **Source**: identity of one fetched document — id, URL, title, fetch time —
  never its body.
- **Run statistics**: honest accounting — angles, searches, sources found /
  fetched, claims extracted / deduplicated / verified / dropped, tokens,
  elapsed time, stopped-early flag and stop reason.
- **Claim (internal)**: a falsifiable statement extracted from a source span,
  carrying its source id; deduplicated across sources before verification;
  never returned raw.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Across an acceptance set of at least 6 live research questions,
  100% of cited source ids resolve to sources fetched in that run, and 0
  fabricated citations occur (every URL in the source list was actually
  retrieved).
- **SC-002**: 100% of research responses — including early-stopped and
  failure-degraded runs — conform to the declared output structure.
- **SC-003**: On the acceptance set, a default-depth research run completes in
  under 4 minutes wall clock, and the quick tier in under 150 seconds.
  *(Amended 2026-06-12 from live measurement: the original 90-second quick
  target was calibrated to the budget-starved 40k-token tier; completed quick
  runs measure 84–93 s.)*
- **SC-004**: With a deliberately tiny budget or deadline, 100% of runs return
  a well-formed early-synthesized response with the stopped-early flag and a
  named stop reason — 0 errors, 0 silently truncated answers.
- **SC-005**: With no search credential configured, the complete pre-existing
  test suite passes unchanged and the tool catalog contains exactly the
  pre-existing tools.
- **SC-006**: 100% of research invocations (successes and all failure classes)
  leave exactly one correctly attributed invocation record.
- **SC-007**: Given an acceptance question seeded with a known-false premise,
  the answer does not confirm the false premise: the relevant claim ends
  refuted or contested, or the premise is challenged in the synthesis.

## Assumptions

- One search provider in v1, chosen at planning time, behind a swappable
  boundary; additional providers are out of scope.
- Depth tiers in v1 are quick / standard / deep; an exhaustive tier is
  deferred until the deep tier's cost/quality knee is measured.
- Content extraction is local (no second managed extraction service) and
  targets static page content; pages requiring script execution to render are
  dropped and counted like any failed fetch.
- English-language questions and sources are the acceptance target; other
  languages may work but are not measured.
- v1 defers, by explicit scope cut: result/source/claim caches, the write-back
  of completed research into the memory layer, MCP progress notifications
  during the run, and the exhaustive depth tier. Each is named in the design
  doc and remains on the roadmap.
- Credibility scoring starts heuristic and conservative (domain class,
  corroboration count); a learned signal is out of scope.
- Verification reuses the existing constrained-output verification machinery
  where it fits; the refute-biased prompt stance follows the design doc.
- True determinism is not promised — the web moves between runs; the
  statistics record what was actually seen.
