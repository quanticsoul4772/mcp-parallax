# Feature Specification: Glob Locators for grounded-verify

**Feature Branch**: `009-glob-locators`

**Created**: 2026-06-13

**Status**: Draft

**Input**: User description: "Glob locators for grounded-verify (the deferred slice of 008): a glob pattern as a new source-locator shape, resolved server-side to the matching files within the configured root — deterministic, root-confined, all-or-nothing."

## User Scenarios & Testing *(mandatory)*

`grounded_verify` (008) lets the caller name source as exact file paths and
file/line ranges. Often the relevant evidence is "every file matching a
pattern" — all the modules of a layer, every test file, every config. Naming
each path by hand is tedious and error-prone, and the caller can silently *miss*
files, weakening the verdict. A glob locator lets the caller say "verify against
`src/**/*.rs`" and have the **server** expand it to the exact matching set —
deterministically, confined to the root, and refusing to silently drop matches.

This is the deferred slice of 008: the exact-path and line-range locators are
unchanged; this adds one new locator shape.

### User Story 1 - Verify against a pattern-matched file set (Priority: P1)

The caller provides a glob pattern as a locator instead of an exact path. The
server expands it, within the configured root, to the set of matching files;
each match is read verbatim and judged as evidence, and each appears in the
manifest — exactly as if the caller had named every path individually.

**Why this priority**: This is the whole feature. Without expansion there is no
glob support; everything else (determinism, confinement, limits) is a property
of how this expansion behaves.

**Independent Test**: Provide a glob that matches several files; the verdict is
rendered over all of them and the manifest lists each expanded file
individually (not the pattern).

**Acceptance Scenarios**:

1. **Given** a configured root with several files matching a pattern, **When** `grounded_verify` is called with that glob as a locator, **Then** the verdict is rendered over all matching files and the manifest names each expanded file with its bytes.
2. **Given** the same glob over an unchanged file set, **When** the call is repeated, **Then** the assembled evidence is byte-for-byte identical (expansion order is stable), preserving 008's determinism.
3. **Given** a mix of a glob locator and an exact-path locator in one call, **When** the call runs, **Then** both contribute evidence and the existing exact-path behaviour is unchanged.

---

### User Story 2 - A glob that matches nothing fails loudly (Priority: P2)

A glob that expands to zero files is a caller error worth surfacing — verifying
against no evidence is meaningless and must never look like a clean result.

**Why this priority**: It is the most likely glob mistake (a typo'd pattern, a
wrong directory) and, left silent, produces a confident verdict over nothing —
the exact "looks clean but isn't" failure the grounded layer exists to prevent.

**Independent Test**: A glob matching no files returns a loud named error
identifying the pattern; no verdict is produced.

**Acceptance Scenarios**:

1. **Given** a glob that matches no file under the root, **When** `grounded_verify` is called, **Then** it returns a loud error naming the pattern and renders no verdict.

---

### User Story 3 - Expansion respects the existing ceilings (Priority: P3)

A glob can expand to many files. The expansion is subject to the same per-call
ceilings as hand-named locators: the locator count and the total evidence bytes.
Exceeding either is a loud named error, never a silent truncation that would
hide files from the verdict.

**Why this priority**: It prevents a single broad glob (`**/*`) from quietly
blowing the budget or being silently trimmed; correctness of the all-or-nothing
guarantee depends on it, but it is a bound on an already-working expansion.

**Independent Test**: A glob whose match count exceeds the locator ceiling, and
one whose total bytes exceed the evidence ceiling, each return a loud named
error and no verdict.

**Acceptance Scenarios**:

1. **Given** a glob that expands to more files than the per-call locator ceiling, **When** `grounded_verify` is called, **Then** it returns a loud error naming the overflow and renders no verdict.
2. **Given** a glob whose matched files together exceed the evidence-byte ceiling, **When** the call runs, **Then** it returns a loud error and renders no verdict.

### Edge Cases

- **Zero matches**: a loud named error (US2), never an empty-but-clean result.
- **Escape via the pattern**: a glob that would reach outside the root (a `../` segment, or matches reached through a symlinked directory) has every expanded path re-checked against the canonicalized root; any escaping match is rejected before it is read.
- **Glob with a line range**: rejected — a line range is meaningless across multiple files, so a locator that is both a glob and a range is a loud named error.
- **Over the locator ceiling**: a loud named error identifying the overflow (US3), never a silent truncation.
- **Over the byte ceiling**: a loud named error (US3); the existing per-file and total-bytes guards still apply to every expanded file.
- **A pattern that matches directories or non-text files**: directories are not read as evidence; a matched non-text file is rejected by the existing text-only guard (the whole call fails, all-or-nothing).
- **Determinism across runs**: the expanded set is ordered by a stable rule so repeated calls over unchanged files produce identical evidence.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: A source locator MAY be a glob pattern, in addition to the exact path and file/line range shapes from 008. The existing shapes are unchanged.
- **FR-002**: The server MUST expand a glob, within the single configured source root, to the set of matching files, and treat each match as it treats an exact-path locator — read verbatim, judged as evidence, and recorded as its own manifest entry naming the concrete file (not the pattern).
- **FR-003**: Glob expansion MUST be deterministic: the same pattern over an unchanged file set MUST yield the same ordered set, so assembled evidence is identical across repeated calls (preserving 008's determinism guarantee).
- **FR-004**: Every expanded path MUST be confined to the configured root — re-checked against the canonicalized root so no match can escape via traversal or a symlinked directory — before it is read.
- **FR-005**: A glob that matches zero files MUST be a loud error naming the pattern; no verdict is rendered.
- **FR-006**: Glob expansion MUST be subject to the existing per-call ceilings — the maximum locator count and the maximum total evidence bytes. Exceeding either MUST be a loud named error, never a silent truncation.
- **FR-007**: A locator that is both a glob and a line range MUST be rejected with a loud named error (a range across multiple files is meaningless).
- **FR-008**: The whole call remains all-or-nothing: if any expanded file fails to resolve (non-text, unreadable, or any ceiling breach), the entire call fails with a named error and renders no verdict.
- **FR-009**: The feature MUST add only the new locator shape; the exact-path and line-range code paths, their behaviour, and their tests MUST be unchanged.

### Key Entities

- **Glob locator**: a caller-supplied pattern interpreted within the configured root, which the server expands to a concrete, ordered set of file locators before assembly.
- **Expanded match set**: the deterministic, root-confined set of files a glob resolves to; each member behaves as an exact-path locator and produces its own manifest entry.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A glob matching N files produces a verdict over all N and a manifest with exactly N entries, each naming a concrete file with its bytes — 0% of matched files silently dropped.
- **SC-002**: The same glob over an unchanged file set yields byte-identical assembled evidence on 100% of repeated calls.
- **SC-003**: 100% of expanded paths that resolve outside the configured root (traversal or symlinked directory) are rejected before any read.
- **SC-004**: A zero-match glob, an over-locator-ceiling glob, and an over-byte-ceiling glob each return a loud named error and produce a verdict in 0% of cases.
- **SC-005**: The 008 exact-path and line-range behaviour is unchanged — 100% of the existing grounded-verify tests pass without modification.

## Assumptions

- **Globs match files within the single configured root only** (008's single-root model is unchanged); a pattern is interpreted relative to that root.
- **The all-or-nothing and confinement guarantees come from 008** — this feature reuses the existing root-confined reader and assembly; the only new machinery is deterministic expansion of a pattern into the concrete file set before assembly.
- **Text-only and per-file/total byte guards from 008 apply unchanged** to every expanded file.
- **A glob expands to files, not directories** — directories among the matches are not read as evidence.
- **Pattern syntax follows a standard, widely-understood glob convention** (e.g. `*`, `**`, `?`, character classes); the exact supported syntax is a planning detail, but `**` recursive matching is in scope since "every file under a subtree" is the motivating case.
