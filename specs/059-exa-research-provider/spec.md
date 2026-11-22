# Feature Specification: Exa replaces Brave for research

**Feature Branch**: `059-exa-research-provider`

**Created**: 2026-11-22

**Status**: Draft

**Input**: User description: "Exa replaces Brave as the research tool's search and page-text provider. (1) Search: Exa POST /search behind the SearchProvider seam, gated on EXA_API_KEY; BRAVE_API_KEY and the Brave client are removed (breaking configuration change; how a still-set BRAVE_API_KEY is treated is a decision). (2) Page text: each source's text comes from Exa's contents (returned with search results or from POST /contents) instead of Parallax fetching and parsing HTML itself; the grounding gate and claim extraction are unchanged; whether the SSRF-guarded fetcher is deleted or kept is a decision. (3) Exa features used, mirroring the user's exa-mcp-server: search type mapped from research's depth, freshness (maxAgeHours), highlights or text with a character cap, a truncation marker; not used, each with a recorded reason: category and domain filters, outputSchema (Parallax's model writes the answer), summaries. (4) Read-only Exa requests retry on network errors, 429 and 5xx, honoring Retry-After, under MAX_RETRIES. (5) Exa's per-request cost is recorded in the invocation record and telemetry. (6) Out of scope, named follow-ups: Voyage rerank of sources and passages, a Jev prompt-injection screen (shadow/warn), research as an MCP Task. (7) Tests use HTTP doubles for Exa; a live research acceptance run passes before the PR."

## Clarifications

### Session 2026-11-22

- Q: A still-set `BRAVE_API_KEY` at startup? → A: A startup error naming it,
  saying Brave is removed and `EXA_API_KEY` enables research.
- Q: Parallax's own page fetcher once Exa supplies the text? → A: Deleted,
  with `FETCH_ALLOW_PRIVATE` and `FETCH_TIMEOUT_MS`; a still-set one is a
  startup error, the same rule as `BRAVE_API_KEY`.
- Q: The caller's domain constraints? → A: Sent to Exa as domain filters and
  kept as the local filter, which stays the authority.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Research runs on Exa (Priority: P1)

An operator sets an Exa API key and no Brave key. The calling model asks
`research` a question and gets back a cited, verified answer, as before. The
sources were found by Exa, and each source's text came from Exa rather than
from Parallax downloading and parsing the page.

**Why this priority**: It is the feature. Brave is removed, so without it the
research tool has no search provider at all.

**Independent Test**: With an Exa key configured and Exa replaced by a test
double, run `research` and confirm the answer's sources are the double's
results, the claims were extracted from the double's page text, and no page was
downloaded by Parallax itself.

**Acceptance Scenarios**:

1. **Given** an Exa key is configured, **When** the server starts, **Then**
   `research` is in the tool catalog.
2. **Given** no Exa key is configured, **When** the server starts, **Then**
   `research` is not in the catalog, as when no Brave key was set before.
3. **Given** a research run, **When** a source is used, **Then** its text is
   the text Exa returned for that URL, and the existing grounding gate decides
   whether each claim is supported by that text.
4. **Given** each research depth (quick, standard, deep), **When** a run
   searches, **Then** it asks Exa for the search type mapped to that depth.

---

### User Story 2 - An operator moving off Brave is told what changed (Priority: P1)

An operator who had `BRAVE_API_KEY` set upgrades. They learn, at startup and
in the CHANGELOG, that Brave is gone and which variable now enables research.

**Why this priority**: A silent loss of the research tool after an upgrade is
the failure this project's configuration rules exist to prevent.

**Independent Test**: Start the server with only `BRAVE_API_KEY` set and read
what it reports.

**Acceptance Scenarios**:

1. **Given** only `BRAVE_API_KEY` is set, **When** the server starts, **Then**
   the operator is told that Brave is no longer supported and `EXA_API_KEY`
   enables research, and the server does not start until `BRAVE_API_KEY` is
   removed.

---

### User Story 3 - Cost and retries are visible (Priority: P2)

The operator sees what each research run spent on Exa, next to its model
cost, and a brief Exa outage or rate limit does not fail a run that a retry
would have saved.

**Why this priority**: Research is the one tool that spends money outside the
model provider; the operator needs it in the same record. Retries are already
part of how every other external call behaves.

**Independent Test**: Run `research` against a double that reports a cost per
request and fails the first request with a rate limit and a retry delay; check
the run succeeds after waiting that delay and the record carries the summed
Exa cost.

**Acceptance Scenarios**:

1. **Given** a run that made several Exa requests, **When** it finishes,
   **Then** its invocation record and its exported telemetry carry the total
   Exa cost reported for those requests.
2. **Given** Exa answers a request with a rate limit or a server error,
   **When** retries remain, **Then** the request is retried, waiting the
   delay Exa asked for when it gave one.
3. **Given** a request fails with any other client error, **When** it is
   reported, **Then** it is not retried.

### Edge Cases

- Exa returns a result with no text for a URL: the source is dropped and
  counted, as a page without readable text is today.
- Exa's text for a page stops at the character cap: the source is marked as
  truncated, so a gap is not mistaken for the page's whole content.
- A search returns URLs on a denied domain or outside an allowed list: the
  caller's existing domain constraints still exclude them, as today.
- Exa is unreachable for the whole run: the run fails with a clear outcome,
  not an empty answer that reports success.
- Exa reports no cost for a request: the cost is recorded as unknown for that
  request, not as zero.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Research MUST find sources with Exa search, and MUST be offered
  only when an Exa API key is configured.
- **FR-002**: The Brave provider and its configuration MUST be removed. A
  still-set `BRAVE_API_KEY` MUST be a startup error that names it, says Brave
  is removed, and names `EXA_API_KEY`.
- **FR-003**: Each source's text MUST come from Exa. Parallax MUST NOT
  download or parse the source page itself. Parallax's own page fetcher, its
  private-address guard, page-hygiene rules and readability extraction, and
  the `FETCH_ALLOW_PRIVATE` and `FETCH_TIMEOUT_MS` settings MUST be removed;
  a still-set `FETCH_ALLOW_PRIVATE` or `FETCH_TIMEOUT_MS` MUST be a startup
  error naming it, as for `BRAVE_API_KEY`.
- **FR-004**: The grounding gate, claim extraction, verification and
  synthesis MUST be unchanged apart from where the text comes from.
- **FR-005**: Each research depth MUST map to one Exa search type, stated in
  the plan and the tool description.
- **FR-006**: Research MUST ask Exa for pages no older than a configured
  freshness limit, with a default stated in the plan.
- **FR-007**: Per-source text MUST be capped at a stated character count, and
  a source whose text reached the cap MUST be marked truncated.
- **FR-008**: The caller's `domains_allow` and `domains_deny` constraints
  MUST be sent to Exa as search domain filters, so excluded domains do not use
  up result slots, and MUST also keep excluding sources locally as they do
  today; the local filter is the authority.
- **FR-009**: Exa requests that only read MUST be retried on network errors,
  rate limits and server errors, honoring a requested retry delay, up to the
  existing retry limit; other client errors MUST NOT be retried.
- **FR-010**: The total Exa cost of a run MUST be recorded in its invocation
  record and exported telemetry; an unreported cost MUST be recorded as
  unknown, not zero.
- **FR-011**: Exa features not used MUST be listed in the plan with the
  reason: category filters, synthesized output (`outputSchema`), and
  summaries.
- **FR-012**: The CHANGELOG MUST name the breaking configuration change
  (`BRAVE_API_KEY`, `FETCH_ALLOW_PRIVATE` and `FETCH_TIMEOUT_MS` removed), and
  the README and `--help` MUST name `EXA_API_KEY` in place of
  `BRAVE_API_KEY`.

### Key Entities

- **Search result**: a URL, a title, and the text Exa returned for it, with
  a truncated flag.
- **Exa cost**: the cost Exa reports per request, summed per run; unknown
  when not reported.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With only an Exa key configured, a live research run on a
  factual question returns a cited answer with at least one supported
  finding, over three runs.
- **SC-002**: No research run makes a page request from Parallax itself;
  every source's text is Exa's.
- **SC-003**: Every run's invocation record carries its Exa cost, and the
  exported telemetry carries the same value.
- **SC-004**: A run whose first Exa request is rate-limited with a retry
  delay succeeds after waiting that delay.
- **SC-005**: An operator who upgrades with `BRAVE_API_KEY`,
  `FETCH_ALLOW_PRIVATE` or `FETCH_TIMEOUT_MS` still set gets a startup error
  naming the variable, and for `BRAVE_API_KEY`, the variable that enables
  research now.

## Assumptions

- The user's own exa-mcp-server (`D:\Projects\exa-mcp-server`) is the
  reference for which Exa features are used and how: authentication header,
  search types, contents options, retry rules.
- Voyage reranking of sources and passages, a Jev prompt-injection screen
  (shadow or warn mode, per its back-test), and running research as an MCP
  Task are named follow-ups, not part of this feature.
- Highlights alone are too short for claim extraction, so research asks Exa
  for page text; highlights may be requested alongside it.
- The tool's input and output shapes do not change; only where sources and
  their text come from changes, plus the cost now recorded.
- The model-facing prompts may need wording changes where they describe
  fetched pages; any such change passes the live research acceptance run
  before the PR.
