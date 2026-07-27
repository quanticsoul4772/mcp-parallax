# Feature Specification: An Unresolvable Default Fails Instead of Being Skipped

**Feature Branch**: `029-unresolvable-default-fails` (the branch hook numbered from
`specs/`, which lags because features 029–039 shipped without spec directories;
the spec directory is `040-` to follow the changelog's feature numbering)

**Created**: 2026-07-27

**Status**: Draft

**Input**: "The derived-defaults checks are incomplete in a way that hides itself, and the incompleteness is the defect rather than the missing coverage."

## Context

### What is broken

Three features built checks that compare an operator-facing document against the
configuration source, so a default cannot drift without something failing:

- **034** made `--help` list every variable the configuration reads, and pinned
  five defaults by hand.
- **036** replaced those five with a scan that reads default values out of the
  source, because a hand-written list only ever covers what its author thought
  of.
- **039** applied the same scan to the README's configuration table.

The scan reads a `name, default` pair, reduces the default to its digits, and —
when no digits remain — moves to the next variable **without recording that it
skipped one**.

Configuration expresses defaults in four shapes. The scan handles one:

| Shape | Example variable | Checked today |
| --- | --- | --- |
| Numeric literal | `RESEARCH_CONCURRENCY` | yes |
| Named numeric constant | `GROUNDED_VERIFY_MAX_BYTES`, `GROUNDED_VERIFY_MAX_LOCATORS` | **no — skipped silently** |
| Named string constant | `ANTHROPIC_MODEL`, `VOYAGE_MODEL`, `ANTHROPIC_API_BASE` | no — a hand-written list of three |
| Inline string literal | `LOG_LEVEL`, `DATABASE_PATH` | **no — checked nowhere** |

**Demonstrated, not inferred.** Setting both `--help` and the README table to a
wrong byte ceiling of `999999` leaves every test passing. Two operator-facing
documents contradict the code and nothing reports it.

### Why the incompleteness is the defect

Each feature reported the loop closed and closed part of it:

- 034 pinned five defaults and left the rest to drift.
- 036 derived one shape of four and left three, guarded by a floor that requires
  at least eight variables to be checked — a number the literal-valued ones
  clear on their own, so the floor measures effort rather than coverage.
- 039 inherited 036's scan and its blind spot along with it.

Three instances of one shape: **partial derivation reported as derivation.** The
individual missing variables matter less than the fact that a check can decline
to examine something and still report success. That is what this feature
removes.

## Clarifications

### Session 2026-07-27

- Q: Does the check also verify the reverse direction — that every default a
  document states belongs to a variable that has one? → A: Yes, both
  directions, but the reverse reads only the **structured default marker**
  (the README table's Default column, `--help`'s `(default: X)`), never the
  surrounding prose. Settled by `decide` at 85 against 62 for equal strictness.
  Prose carries numbers that are not defaults — `1..=20`, ceilings quoted in
  explanations, versions — and a reverse scan over it would produce false
  positives, which get silenced rather than fixed. That is this feature's own
  failure mode aimed the other way. It also follows the corpus rule directly:
  the Default column is a fact, the Purpose column is a reason; check the facts
  and leave the prose alone.

- Q: Where may a named constant be resolved from — a declared set of source
  files, or anywhere in the crate? → A: A **declared set**. Searching the whole
  crate scored 15: Rust permits one constant name in several modules, so a
  collision resolves to whichever declaration is found first and the check then
  compares a document against a value from the wrong one — a wrong answer that
  looks like success, which is this feature's own failure mode. Enumerating the
  search space makes that unrepresentable rather than merely detectable
  (whole-crate-with-ambiguity-failure scored 55 for catching it after the fact).

  `decide` preferred a variant that also named "the file that would need
  adding" (90 v 82), and the confirmation `verify` refuted it 3/3: a check that
  searches only the declared set has no idea where the missing constant lives,
  so naming the file requires exactly the unrestricted search the design
  rejects. The message states what the check can know — the constant was not in
  the declared set, and here is that set.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A default the check cannot resolve stops the build (Priority: P1)

Someone adds a configuration variable, or changes how an existing one expresses
its default, in a form the check does not recognise. The build fails naming the
variable, rather than passing while quietly examining one variable fewer.

**Why this priority**: This is the feature. Every other story is coverage that
follows from it, and coverage added without this regrows the same blind spot the
next time a new shape appears.

**Independent Test**: Introduce a default in an unrecognised form and confirm the
check fails naming that variable.

**Acceptance Scenarios**:

1. **Given** a variable whose default the check cannot resolve, **When** the
   suite runs, **Then** it fails and names the variable.
2. **Given** that same variable added to the exclusion list with a reason,
   **When** the suite runs, **Then** it passes.
3. **Given** an exclusion entry for a variable that no longer exists, **When**
   the suite runs, **Then** it fails, so the list cannot accumulate stale
   entries.

---

### User Story 2 - Every documented default matches the code (Priority: P1)

An operator reading `--help` or the README sees the values the server will
actually apply, for every variable that has one.

**Why this priority**: Equal-first. This is the outcome the checks were built for
and have not delivered; US1 is what keeps it delivered.

**Independent Test**: Change any default in the configuration source without
touching the documents, and confirm the suite fails naming the variable and both
values.

**Acceptance Scenarios**:

1. **Given** a default expressed as a named constant, **When** it changes and
   the documents do not, **Then** the suite fails.
2. **Given** a default expressed as a string, **When** it changes and the
   documents do not, **Then** the suite fails.
3. **Given** a document stating a default the code does not apply, **When** the
   suite runs, **Then** it fails naming both values.

---

### User Story 3 - Coverage is stated, not implied (Priority: P2)

A reader can tell how many variables the check examined and which it did not,
without reading its implementation.

**Why this priority**: Lower than the two above because it changes no outcome —
but a floor that passes on partial coverage is what let 036 look complete, so
reporting the real number is what makes a future regression legible.

**Independent Test**: Confirm the check reports the count it examined and that
the count equals the number of variables carrying a default, less the exclusions.

**Acceptance Scenarios**:

1. **Given** the suite runs, **When** it reports coverage, **Then** the number
   is the variables examined rather than a minimum threshold.

---

### Edge Cases

- A default that is a constant defined in a different file from the
  configuration: resolves if that file is in the declared set, and fails
  nameably if not. One of the named string constants is declared elsewhere
  today, so the set has more than one member from the start.
- Two modules declaring the same constant name: cannot mis-resolve, because the
  check reads only the enumerated set rather than searching for the name.
- A default that is an expression rather than a literal or a constant: the check
  cannot resolve it, so it fails and forces either handling or exclusion.
- A variable with no default at all — absence gates a capability rather than
  selecting a value. Out of scope; these are already required to be listed.
- A document stating a default for a variable that has none, or for one since
  removed: caught by the reverse direction (FR-009), which reads the structured
  default marker only.
- A number in a document's prose that is not a default — a range, a ceiling
  quoted in an explanation, a version: **not** a finding. The reverse direction
  never reads prose, which is what keeps it from producing the false positives
  that would get it silenced.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: A default the check cannot resolve MUST fail the suite, naming the
  variable. Skipping MUST NOT be possible without a record.
- **FR-002**: A variable MAY be excluded from resolution only by an explicit
  entry carrying a stated reason.
- **FR-003**: An exclusion for a variable that no longer carries a default MUST
  fail, so the list cannot outlive what it excuses.
- **FR-004**: Defaults expressed as a named constant MUST be resolved to the
  constant's value, including when the constant is declared outside the
  configuration source.
- **FR-004a**: Constants MUST be resolved from an **enumerated set of source
  files**, never by searching the crate. One constant name may be declared in
  several modules, and an unrestricted search resolves to whichever declaration
  it reaches first — comparing a document against the wrong value while
  reporting success. Enumerating the search space makes that outcome
  unrepresentable instead of merely detectable.
- **FR-004b**: A constant absent from the declared set MUST fail under FR-001,
  and the message MUST state the constant, the variable, and the set that was
  searched. It MUST NOT claim to know which file would need adding: the check
  has not looked outside its set and cannot know.
- **FR-005**: Defaults expressed as a string — literal or named constant — MUST
  be compared, replacing the hand-written list of three.
- **FR-006**: Every operator-facing document that states defaults MUST be
  checked by the same resolution, so one document cannot be guarded more
  thoroughly than another.
- **FR-007**: The check MUST report how many variables it examined, as a count
  rather than a minimum.
- **FR-008**: Variables read with no default MUST remain out of scope for value
  comparison, and remain in scope for the existing presence requirement.
- **FR-009**: Every default a document states MUST belong to a variable that
  carries one, checked with the same strictness as the forward direction.
- **FR-010**: The reverse direction MUST read only a document's **structured
  default marker** — the README table's Default column, `--help`'s
  `(default: X)` — and MUST NOT read surrounding prose. Prose is hand-written
  reasoning and contains numbers that are not defaults; reading it would
  generate false positives, and a check that cries wolf gets silenced rather
  than corrected.

### Key Entities

- **Default**: A value the configuration applies when a variable is unset.
  Expressed as a numeric literal, a named numeric constant, a string literal, or
  a named string constant.
- **Exclusion**: A named variable the check does not resolve, with a stated
  reason. Must correspond to a variable that exists and carries a default.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Changing any configuration default without updating the documents
  fails the suite, for every variable that carries one — currently true for the
  numeric-literal shape only.
- **SC-002**: The mutation that motivated this feature — a wrong byte ceiling in
  both documents — fails, where today it passes.
- **SC-003**: Adding a variable whose default is expressed in an unrecognised
  form fails until someone either handles the form or records an exclusion.
- **SC-004**: The number of variables checked is reported and equals the number
  carrying a default, less recorded exclusions.
- **SC-005**: No document stating configuration defaults is left unchecked by the
  resolution the others use.
- **SC-006**: A default stated in a document for a variable that does not carry
  one fails the suite.
- **SC-007**: A number appearing in a document's explanatory prose produces no
  finding, so the reverse direction stays quiet on everything except structured
  default markers.
- **SC-008**: The resolver reads an enumerated file set rather than searching
  for a name, so a constant declared in more than one module has no path by
  which it could be compared against the wrong declaration. **Stated as a design
  property, not a measurable outcome**: the failure was made unrepresentable, so
  there is no behaviour left to measure. The observable consequence — a constant
  outside the set failing rather than resolving wrongly — belongs to SC-003.

  A success criterion phrased as a measurement here would describe a lower rung
  of §10's ladder than what was built.

## Assumptions

- The set of shapes is closed at the four observed today. A fifth is expected to
  appear eventually; FR-001 is what makes that appearance visible rather than
  silent, and is the reason the feature is worth building beyond the coverage it
  adds.
- Reasons for exclusion are prose written by a person. There is no source to
  derive them from, and the corpus rule is to derive facts and hand-write
  reasons.
- Variables read with no default stay out of scope. Absence gates a capability
  rather than selecting a value, so there is nothing to compare.
