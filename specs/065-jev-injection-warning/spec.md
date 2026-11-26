# Feature Specification: Jev injection warning on research sources

**Feature Branch**: `065-jev-injection-warning`

**Created**: 2026-11-25

**Status**: Draft

**Input**: plan `~/.claude/plans/warm-kindling-shell.md` §6.1 U5, as amended 2026-11-25 (the U4 claim cascade moved to a later feature), and the 064 injection back-test in `claudedocs/jev-064-measurements-2026-11-24.md`.

## Clarifications

### Session 2026-11-25

- Q: Should the user's setup be `on` at 0.35 at the end of 065, or `shadow` until plan §7.3 passes on real traffic? → A: `on` at 0.35, as plan §6.1 and §4.1 say; the tool docs state the measured false-positive rate.
- Q: What text does Jev screen? → A: The same capped text research reads for extraction and 063 screens, one call per page; no splitting.
- Q: When does each page's Jev call happen? → A: Concurrently with that page's extraction call, joined before the page's result is assembled.
- Q: What deadline does each Jev call have? → A: 2000 ms per call including retries, capped by the run's remaining time (064's research-claim deadline).
- Q: What does the result show for the Jev fields with the site off or in shadow? → A: Nothing: the fields are left out, so those results match today's; they appear only with the site on.
- Q: What does each injection record store from 063, for the agreement report? → A: The names of the 063 patterns that matched that page, in the record's label; names only, no page text.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Research marks sources that address the AI (Priority: P1)

With the injection site on, every source page research keeps for extraction is also asked Jev's `directs_ai` question. A page that scores at or above the pinned threshold carries a Jev injection warning in the research result, so the calling model treats instructions in it as untrusted data. The page is still read and used exactly as before.

**Why this priority**: it is the first Jev answer that changes what Parallax returns, and its threshold is already measured (064 back-test on `jev-1.14.0`: recall 0.86, false-positive rate 0.024 at 0.35).

**Independent Test**: with a stand-in Jev that scores one page high and one low, the result marks only the first, every other result field is identical to a run with the site off, and both calls leave records.

**Acceptance Scenarios**:

1. **Given** the site is on and Jev scores a kept page 0.9, **When** research finishes, **Then** that source carries the Jev warning with its probability, and the stats count it.
2. **Given** the site is on and Jev scores a page 0.1, **When** research finishes, **Then** the source carries no warning and shows that it was screened.
3. **Given** the site is on and Jev errors, times out, gives an invalid answer or answers with another model, **When** research finishes, **Then** the source carries no warning, shows that it was not screened and why, and the stats count the failure.

---

### User Story 2 - Shadow first, separate from the pattern screen (Priority: P1)

With the site in shadow, the same calls are made and recorded, and the result is unchanged. In every mode, 063's pattern screen (`injection_patterns`, `flagged_pages`) runs and reports exactly as today.

**Why this priority**: the operator can collect real-traffic answers before the warning shows, and the two screens stay comparable.

**Independent Test**: in shadow, the result equals the site-off result except latency fields, and one record exists per kept page; 063's fields are identical in all three modes.

**Acceptance Scenarios**:

1. **Given** the site is in shadow, **When** research runs, **Then** the result has no Jev warning fields set and one record per kept page is written.
2. **Given** a page that 063 flags and Jev does not (or the reverse), **When** research finishes with the site on, **Then** each screen reports its own answer and neither changes the other.

---

### User Story 3 - Safe configuration and records (Priority: P2)

The site is off unless the operator sets it; a misconfiguration stops startup with a message naming the variable; every call leaves a record the back-test harness can read.

**Independent Test**: configuration tests for each error, and a records test joining every injection row to its research invocation.

**Acceptance Scenarios**:

1. **Given** `on` is set without a TypeSafe key, or for a model with no pinned threshold, **When** the server starts, **Then** it refuses to start and names both variables.
2. **Given** the site is off, **When** research runs, **Then** no Jev call is made and no injection record is written.

### Edge Cases

- **No kept pages**: no calls.
- **Page text**: Jev screens the same capped text research reads for extraction and 063 screens, one call per page; text past the cap is never screened, and never read.
- **Redaction fails**: nothing is sent; the page shows "not screened" and the failure is recorded.
- **Rate limit or overload**: shadow never retries; `on` retries through the shared helpers within the run's remaining time.
- **401**: Jev is disabled for the process, with one log line, as in 064.
- **Run deadline**: Jev screening never extends a research run past its deadline; pages not answered in time show "not screened".
- **Cancelled run**: records for calls already made are still written.
- **Claim shadow (064) excludes 063-flagged sources**: unchanged; a Jev warning does not exclude claims.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001 Question**: the `directs_ai` yes/no question, with exa-mcp-server's instructions and both criteria verbatim (as recorded in `claudedocs/jev-064-measurements-2026-11-24.md`), at a pinned question version.
- **FR-002 What is sent**: each kept page as `{page_url, page_text}`, through the mandatory redactor (URL included), to the pinned model `jev-1.14.0`. The page text is the capped text research reads for extraction, the same text 063 screens. Stored memories and the caller's question are never sent.
- **FR-003 Warning**: a page whose probability is at or above the threshold pinned for the answering model carries a warning; the threshold for `jev-1.14.0` is 0.35. A missing, invalid or other-model answer never warns.
- **FR-004 Warn only**: the warning changes no page selection, extraction, verification, grounding, label, confidence or other field. It appears in its own per-source field and a stats count, separate from 063's fields; the source also shows whether it was screened and, if not, why. These fields and the count appear only with the site on; with it off or in shadow they are left out.
- **FR-005 Wording**: the research tool description and docs state that the warning is advisory, that "no warning" does not mean the page is clean, and the measured rates at the pinned threshold (064 back-test: recall 0.86; 10 of 413 clean pages warned, 9 of them pages about prompt injection).
- **FR-006 Modes**: `PARALLAX_JEV_MODE_RESEARCH_INJECTION`, values `off|shadow|on`, default `off`. Unknown values, a mode set without `TYPESAFE_API_KEY`, or `on` for a model with no pinned threshold are startup errors naming the variable. A startup line names the destination host and the data class sent. `on` for the other Jev sites remains a startup error.
- **FR-007 Calls**: one request per kept page, made concurrently with that page's extraction call and joined before the page's result is assembled, within 064's limit of 4 in flight per process, under a deadline of 2000 ms per call, retries included, capped by the run's remaining time; `on` retries 429, 529 and 5xx through the shared retry helpers within that time; shadow makes exactly one request.
- **FR-008 Records**: one `jev_records` row per page request, with the new site `research_injection`, the mode, the answer and decision band, the answering model, outcome, cost, links to the research invocation and source, and as its label the names of the 063 patterns that matched the page (names only, no page text); A row's `applied` flag is 1 only for an `on` row whose decision warned, and 0 otherwise. Jev cost goes into the invocation's `jev_cost_usd`. Jev tokens stay out of the research token ceiling.
- **FR-009 Measurement**: the back-test harness reports the injection site from stored records: share warned, failures by outcome, and agreement with 063's pattern screen on the same pages.
- **FR-010 Setup**: the last task sets the site in the user's Parallax settings, after merge and rebuild, with the exact key shown first and a backup taken; only that key changes. The value is `on`, at the pinned 0.35 threshold, as plan §6.1 and §4.1 say (Clarifications).

### Key Entities

- **Injection record**: one Jev request about one kept research page, with its probability, decision, answering model, outcome, cost and links to the research invocation and source.
- **Source screening result**: per source, whether it was screened, the probability, whether it was warned, and the reason when not screened.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With the site off or in shadow, the research result is identical to the site-off result except elapsed time, for a confident, erroring, invalid and never-returning Jev - shown by tests.
- **SC-002**: With the site on, only the Jev warning fields and their stats count differ from the site-off result - shown by tests.
- **SC-003**: A Jev that never returns still lets every research run finish within its deadline - shown by tests.
- **SC-004**: No secret fixture appears in any captured request body - shown by tests on the whole serialized body.
- **SC-005**: A live research run with the site on writes one injection record per kept page, all joined to the invocation, each answered row carrying the pinned model, and the result shows every kept page as screened or not screened with a reason - asserted by a live example that exits non-zero on failure.

## Assumptions

- The 064 back-test (612 items: 413 Exa pages, 75 BIPIA, 124 InjecAgent attacks inserted into pages) stands as the threshold evidence for `jev-1.14.0`; a different model needs its own back-test before `on`.
- 064's client, redactor, records table, validator and 401 handling are reused unchanged except where named here.
- The kept pages are the pages research reads for extraction after reranking (061), the same set 063 screens.
- Out of scope: Jev settling claims (moved to a later feature), blocking or removing page text, any change to 063's screen, screening the caller's question, hand-labeled correctness.
