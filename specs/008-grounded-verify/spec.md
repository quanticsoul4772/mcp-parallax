# Feature Specification: Source-Grounded Verification (`grounded-verify`)

**Feature Branch**: `008-grounded-verify`

**Created**: 2026-06-13

**Status**: Draft

**Input**: User description: "Source-grounded verification corrective — a sibling of `verify` that mechanically assembles verbatim evidence from named local source files and runs the stance-blind verification ensemble over it, closing `verify`'s context-trust gap."

## User Scenarios & Testing *(mandatory)*

The `verify` tool is stance-blind and context-only by design: its independence
comes from never seeing the conversation. The cost of that design is that a
`verify` verdict is only as trustworthy as the context the caller hand-writes —
a paraphrased or conclusion-laden context can rubber-stamp an overstated claim.
This was observed live during a spec-conformance review: a claim verified
**supported** against a hand-written paraphrase, then **refuted** against the
verbatim code, then **supported** again once the wording was corrected. The
caller had unintentionally smuggled the conclusion into the evidence.

`grounded-verify` removes that ability. The caller still chooses *which* sources
are relevant, but the **server** — not the model, and not the caller's prose —
reads the verbatim text from those sources and assembles it as the evidence. The
caller cannot paraphrase, summarize, or embed a conclusion in the evidence
itself.

### User Story 1 - Verify a claim against verbatim source (Priority: P1)

The calling model has a claim it wants checked against source it cannot be
trusted to summarize faithfully (its own conclusion may bias the retelling). It
supplies the claim and a set of source locators — file paths, globs, or
file/line ranges — pointing into the configured source root. The server reads
exactly that text, verbatim, assembles it as the verification context, runs the
stance-blind ensemble over it, and returns a verdict (supported or refuted),
specific findings, and an agreement-derived confidence.

**Why this priority**: This is the entire value proposition and the MVP. Without
mechanical verbatim assembly there is no improvement over `verify` — the caller
could still paraphrase. Everything else (audit manifest, completeness) refines a
verdict that this story is what produces.

**Independent Test**: Provide a claim whose plain-language phrasing embeds a
conclusion that the verbatim source contradicts, plus the locators for that
source. The returned verdict reflects what the source says, not what the claim's
phrasing asserts — demonstrating the caller can no longer bias the evidence.

**Acceptance Scenarios**:

1. **Given** a configured source root and a claim with locators to files that support it, **When** `grounded-verify` is called, **Then** it returns `supported` with findings and a confidence derived from cross-pass agreement.
2. **Given** a claim whose phrasing overstates what the named source actually shows, **When** `grounded-verify` is called with locators to that source, **Then** it returns `refuted` with findings naming the specific gap — the verdict tracks the source, not the phrasing.
3. **Given** the same claim and locators called twice, **When** the source on disk is unchanged, **Then** the assembled evidence is identical both times (the assembly step is deterministic; only the model passes vary, exactly as in `verify`).

---

### User Story 2 - Audit the evidence the verdict rests on (Priority: P2)

A reviewer (human or a downstream automated check) needs to know exactly what
evidence produced a verdict, so the verdict is itself auditable rather than a
black box. The result carries an evidence manifest: every file and line range
that was read, with its size, in resolution order.

**Why this priority**: A verdict whose evidence cannot be inspected is only
marginally more trustworthy than `verify` — the reviewer still has to take the
basis on faith. The manifest turns "trust the verdict" into "inspect the basis."
It depends on US1 producing a verdict but adds independent value.

**Independent Test**: Call with a known set of locators (including a glob and a
line range) and confirm the returned manifest names exactly the files and ranges
that were read, with sizes, such that the evidence set can be reconstructed from
the manifest alone.

**Acceptance Scenarios**:

1. **Given** a call with three locators (a full file, a line range, and a glob matching two files), **When** the verdict is returned, **Then** the manifest lists all four resolved sources with their byte sizes and the line ranges actually read.
2. **Given** a glob locator, **When** it resolves to specific files, **Then** the manifest names each resolved file individually, not just the glob pattern.

---

### User Story 3 - Surface evidence the caller omitted (Priority: P3)

The residual weakness after US1 is *incompleteness*: the caller controls which
sources are named, so a verdict can be confidently wrong because a relevant
source was never provided. The verification pass therefore also reports what
additional evidence it would have needed to assess the claim fully but was not
given — turning an omission from invisible into a named gap.

**Why this priority**: This mitigates, but cannot eliminate, the
caller-omits-evidence gap; it is the lowest-priority slice and the one most
reasonable to defer. The tool's hard guarantee is fidelity of *what it read*;
completeness of *what was chosen* remains the caller's judgment, made visible
here rather than enforced.

**Independent Test**: Provide a claim that genuinely depends on a source not
included in the locators. The completeness signal names the missing source class
(e.g., "the definition of the function under test was not provided") rather than
silently rendering a verdict as if the evidence were complete.

**Acceptance Scenarios**:

1. **Given** a claim that references behavior defined in a file not among the locators, **When** `grounded-verify` is called, **Then** the result includes a completeness signal naming the kind of source that was missing.
2. **Given** a claim whose provided locators fully cover it, **When** the call returns, **Then** the completeness signal is empty (nothing material was omitted).

### Edge Cases

- **Named source does not exist**: a locator pointing to a missing file is a loud error naming the offending locator — never a silent verification over the sources that *did* resolve.
- **Named source is empty**: a zero-byte file or a glob matching nothing is a loud error — verifying against no evidence is meaningless and must not look like a clean result.
- **Locator escapes the source root**: a path-traversal (`../`) or symlink that resolves outside the configured root is rejected before any read.
- **Line range out of bounds**: a range whose start exceeds the file's length is a loud error.
- **Total evidence exceeds the size bound**: assembled bytes over the configured ceiling is a loud error identifying which locators overflowed — never silently truncated, which would hide evidence from the pass.
- **Source root not configured**: the tool is absent from the catalog entirely and performs no file reads.
- **Non-text/binary source**: a locator resolving to non-text content is rejected with a naming error rather than feeding garbage to the pass.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The tool MUST be gated and off by default — present in the catalog only when an operator has configured an allowed source root, following the existing off-by-default convention for capabilities that read or reach outside the process.
- **FR-002**: The tool MUST accept a claim plus a set of source locators, where a locator is a file path, a glob, or a file-with-line-range, all interpreted relative to the configured source root.
- **FR-003**: The server MUST assemble the verification evidence by reading the verbatim text of the resolved locators with no model involvement in the assembly step — the calling model never authors, paraphrases, or edits the evidence.
- **FR-004**: Every resolved locator MUST be confined to the configured source root; any locator resolving outside it (via traversal or symlink) MUST be rejected before any content is read.
- **FR-005**: The total bytes read per call MUST be bounded; exceeding the bound is a loud error that names the overflowing locators, never a silent truncation.
- **FR-006**: The tool MUST run the same stance-blind verification ensemble as `verify` over the assembled evidence and return a verdict (supported or refuted), specific findings, and an agreement-derived confidence.
- **FR-007**: The verification passes MUST see only the claim and the assembled evidence — never the conversation, the caller's stance, or any caller-supplied prose beyond the claim itself (independence preserved, identical to `verify`).
- **FR-008**: The result MUST include an evidence manifest naming every file and line range actually read, in resolution order, with byte sizes, sufficient to reconstruct the evidence set.
- **FR-009**: A named source that is missing, empty, matches nothing, is out of range, or is non-text MUST produce a loud error identifying the offending locator — never a verification rendered over absent or partial evidence as though it were complete.
- **FR-010**: The result MUST include a completeness signal naming additional evidence the pass would have needed but was not given; when the provided evidence is sufficient, the signal is empty. *(US3 — the deferrable slice.)*
- **FR-011**: Each call MUST produce exactly one invocation record like every other tool (tool name, model, tokens, cost, latency, outcome) and, when telemetry is configured, the same record exported via OTLP.
- **FR-012**: The verdict, findings, manifest, and completeness signal MUST be server-assembled into the structured result; the model authors only the judgment content (findings and verdict reasoning), never the manifest or the labels.

### Key Entities

- **Source locator**: a caller-supplied reference to evidence — a file path, a glob, or a file-with-line-range — interpreted within the configured source root.
- **Assembled evidence**: the verbatim text the server reads from the resolved locators, in resolution order, with provenance tying each span back to the locator that produced it. The only context the verification passes receive besides the claim.
- **Evidence manifest**: the inspectable record of exactly which files and ranges were read and their sizes — the audit surface for the verdict.
- **Grounded verdict**: the structured result — verdict (supported/refuted), findings, agreement-derived confidence, evidence manifest, and completeness signal.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For a paired test set where each claim's phrasing embeds a conclusion the verbatim source contradicts, 100% of verdicts track the source rather than the phrasing — the caller cannot bias the evidence through wording.
- **SC-002**: 100% of returned manifests exactly match the resolved locators (files, line ranges, sizes); a reviewer can reconstruct the exact evidence set from the manifest alone with no access to the call inputs.
- **SC-003**: For every named source that is missing, empty, zero-match, out of range, or non-text, the call returns a loud error naming the offending locator — 0% of such cases produce a verdict.
- **SC-004**: 100% of locators that resolve outside the configured source root (traversal or symlink escape) are rejected before any byte is read.
- **SC-005**: With no source root configured, the tool is absent from the catalog and performs zero file reads over a full test session.
- **SC-006**: Over a seeded set of claims each missing one required source, the completeness signal names the missing source in a measurable majority of cases (target reported as a recall figure), and is empty for claims whose evidence is complete.

## Assumptions

- **The sources live on the server's filesystem.** In the MCP model the server reads its own filesystem, not the client's; "source root" scopes that filesystem. The calling model supplies locators (relative paths/globs/ranges); the operator configures which root they resolve within.
- **The existing `verify` ensemble and stance-blindness are reused** — this feature changes *where the evidence comes from*, not how verification reasons. It is a sibling of `verify`, distinct from `check` (which executes a formal target rather than judging).
- **Text sources only for v1.** Binary/non-text content is rejected, not parsed; rendering or decoding non-text formats is out of scope.
- **The tool guarantees fidelity of what it reads, not completeness of what was chosen.** Selecting the right sources remains the caller's judgment; FR-010's completeness signal makes omissions visible but does not enforce them. This boundary is intentional.
- **Enablement and bounds follow the existing env-configuration convention**: a present-but-malformed configuration value is a startup error, never a silent fallback; the source root and byte/locator ceilings are environment-configured and off (or at safe defaults) by default.
- **No write capability.** The tool only reads within the source root; it never modifies files.
