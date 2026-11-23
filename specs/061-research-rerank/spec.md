# Feature Specification: Rerank fetched research pages

**Feature Branch**: `061-research-rerank`

**Created**: 2026-11-23

**Status**: Draft

**Input**: Rerank research pages by relevance after their text is fetched, with
Voyage rerank-3 (the provider's current reranker; rerank-2.5 is listed as an older model, 2026-11-23), and extract claims only from the most relevant ones, so a
research run makes fewer model calls. On when VOYAGE_API_KEY is set; a failed
rerank falls back to today's order and is recorded (operator-approved exception
to the no-fallback rule).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Research reads only the most relevant pages (Priority: P1)

An operator on a usage-limited subscription runs `research`. After the pages'
text is fetched, each page is scored against the question, and claims are
extracted only from the highest-scoring pages, so the run makes fewer model
calls with an answer that is still grounded and cited.

**Why this priority**: research is the heaviest user of the subscription's
usage limit, and extraction (one model call per page) is its largest step.

**Independent Test**: with a rerank double that scores pages, research extracts
from only the top-scoring pages and the others never reach extraction.

**Acceptance Scenarios**:

1. **Given** a Voyage key and more fetched pages than the keep count, **When**
   research runs, **Then** claims are extracted from exactly the keep count of
   pages, the highest-scoring ones.
2. **Given** fewer fetched pages than the keep count, **When** research runs,
   **Then** every page is kept and no page is dropped.
3. **Given** no Voyage key, **When** research runs, **Then** it behaves exactly
   as before this feature.

---

### User Story 2 - A failed rerank never fails the run (Priority: P1)

If the rerank call fails, research continues in today's order and the result
says the rerank was skipped and why.

**Why this priority**: rerank only trims work; it must never cost the operator
an answer.

**Independent Test**: with a rerank double that fails, research completes on
today's order and records the skip.

**Acceptance Scenarios**:

1. **Given** the rerank provider returns an error or times out, **When**
   research runs, **Then** it completes on today's order and the run records
   `rerank_skipped` with the reason.

---

### User Story 3 - The cut is visible (Priority: P2)

Each run records how many pages were fetched and kept, the rerank's token use,
and each kept source's relevance score.

**Why this priority**: the operator can see what the rerank saved and why a
source was kept.

**Independent Test**: a research run's record and result carry the counts,
the token use and the per-source scores.

**Acceptance Scenarios**:

1. **Given** a run with rerank, **When** it finishes, **Then** the record holds
   pages fetched, pages kept and rerank tokens, and each kept source carries
   its relevance score.

---

### Edge Cases

- A page with empty text is not sent to the reranker; it stays dropped as
  empty, as before this feature (plan D7).
- Two pages with equal scores keep today's relative order.
- A rerank reply missing a page's score is treated as a failed rerank (FR-005).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: With VOYAGE_API_KEY set, research MUST score each fetched page's
  text against the question with Voyage rerank-3 before extraction.
- **FR-002**: Only the top K pages by score MUST reach extraction; K is a
  per-depth keep count below that depth's source limit (defaults set in the
  plan).
- **FR-003**: With fewer pages than K, every page MUST be kept.
- **FR-004**: Without VOYAGE_API_KEY, research MUST be unchanged.
- **FR-005**: A rerank failure (error, timeout, malformed reply) MUST NOT fail
  the run: research continues on today's order and records that rerank was
  skipped and why. This is the one named exception to the no-fallback rule:
  rerank only trims work, so skipping it costs usage, never correctness.
- **FR-006**: The record MUST hold pages fetched, pages kept, and the rerank's
  token use; each kept source MUST carry its relevance score in the result.
- **FR-007**: The rerank call MUST be bounded by the run's deadline and retried
  like the other Voyage calls.
- **FR-008**: Tests MUST use a rerank double; no test calls Voyage.

### Key Entities

- **Rerank result**: a relevance score per fetched page, in the order the
  pages were fetched.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On the same live questions, a run with rerank makes at least 30%
  fewer extraction calls than without, at standard depth.
- **SC-002**: Every live answer still cites only kept sources and passes the
  existing grounding gate.
- **SC-003**: A run whose rerank fails still completes and records the skip.

## Assumptions

- Voyage rerank-3 accepts the question plus up to the deep-depth page count
  in one request; the plan confirms the request limits.
- Page text is trimmed to the existing per-source cap before reranking.
- Reranking by title before fetching is out of scope (scored 42).
