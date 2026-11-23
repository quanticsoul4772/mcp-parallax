# Feature Specification: Screen research pages for prompt injection

**Feature Branch**: `063-research-injection-screen`

**Created**: 2026-11-23

**Status**: Draft

**Input**: Research screens fetched page text for prompt injection (warn
mode). Before claim extraction, an in-process deterministic pattern screen
checks each kept page for known injection phrasings, chat role markers and
special tokens, and hidden-text tricks. No new dependency, network call or
model call. A flagged page is still extracted and verified as today; each
source in the result names the patterns that matched, and the stats count
flagged pages. Always on.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The caller sees which sources carry injection attempts (Priority: P1)

A model calls `research`. One of the pages it reads contains text written to
steer a language model ("ignore previous instructions…", a fake system turn,
hidden characters). The result marks that source with what was found, so the
calling model can weigh or distrust it, and the run's stats show how many
pages were flagged.

**Why this priority**: research feeds web text into model calls and then into
the caller's context. Today nothing tells the caller that a source tried to
give instructions.

**Independent Test**: a run whose fetched pages include one page with an
injection phrase marks exactly that source with the matching pattern name and
counts one flagged page; the run's findings are otherwise unchanged.

**Acceptance Scenarios**:

1. **Given** a kept page containing an instruction-override phrase, **When**
   research runs, **Then** that source lists the matching pattern and the
   stats count 1 flagged page.
2. **Given** a kept page containing zero-width or bidirectional control
   characters, **When** research runs, **Then** that source lists the
   hidden-text pattern.
3. **Given** no kept page matches any pattern, **When** research runs,
   **Then** every source lists no patterns and the count is 0.

---

### User Story 2 - Warn, never block (Priority: P1)

A flagged page is still read, its claims extracted and verified exactly as
any other page, so the screen can never reduce what research finds and an
ordinary page that happens to match costs nothing but a label.

**Why this priority**: a pattern screen has false positives (a security blog
quoting an injection string); blocking would silently remove good sources.

**Independent Test**: the same run with and without the flagged page's
pattern produces the same extraction calls and findings.

**Acceptance Scenarios**:

1. **Given** a flagged page, **When** research runs, **Then** its text goes to
   extraction unchanged and its claims can be cited.

### Edge Cases

- Matching is case-insensitive and tolerant of extra whitespace between the
  words of a phrase.
- Each pattern name appears at most once per source, however many times it
  matched.
- Pages without text are not screened and carry no patterns.
- Pages dropped before extraction (denied domain, not kept by the rerank) are
  not screened and are not counted.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Every page that reaches claim extraction MUST be screened
  before extraction, with no network or model call.
- **FR-002**: The screen MUST detect at least three families: instruction
  override or role reassignment phrasings, chat role markers or special
  tokens, and invisible or bidirectional control characters; each match
  reports a stable pattern name.
- **FR-003**: Each source in the result MUST list the names of the patterns
  that matched its page (empty when none).
- **FR-004**: The result stats MUST count the pages with at least one match.
- **FR-005**: A flagged page's text MUST reach extraction and verification
  unchanged.
- **FR-006**: The published research result contract MUST describe the new
  fields.

### Key Entities

- **Pattern match**: a stable pattern name (for example
  `instruction_override`, `role_marker`, `hidden_text`) attached to a source.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a test corpus of known injection strings, one per family,
  every string is flagged with its family's name.
- **SC-002**: On the 7 live acceptance questions, no run's findings or
  extraction-call count change because of the screen, and each flag reported
  is checked by hand and recorded as a true or false positive.
- **SC-003**: Screening adds no network or model calls (shown by tests).

## Assumptions

- The pattern list is short and versioned in code; it catches common, known
  phrasings, not novel or paraphrased attacks.
- Always on: it reads text already in memory and sends nothing anywhere.
- Out of scope: blocking or stripping text, model-based classification,
  external screening services, new invocation-record columns, screening the
  caller's own question.
