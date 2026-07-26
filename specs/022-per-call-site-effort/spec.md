# Feature Specification: Per-Call-Site Reasoning Effort

**Feature Branch**: `022-per-call-site-effort`

**Created**: 2026-07-25

**Status**: Retrofitted — see *Process deviation* below

**Input**: "Set `effort` per call site. Every parallax call currently runs at the
default `high`, meaning deep reasoning on `check` translation and research
extraction that nobody asked for."

## Process deviation (named, per Principle I)

**This spec was written after the implementation, not before it.** The
constitution mandates the Spec Kit sequence for behaviour changes; this feature
was built directly on request and the artifacts were retrofitted afterwards.

Recorded rather than quietly skipped because Principle I requires deviations be
named in the spec or plan. A `decide` pass scored retrofitting at 80 against 40
for naming the deviation only in a PR description and 55 for discarding the work
and rebuilding — the deciding factor being that the analyze pass is what catches
corpus drift, which is precisely how feature 021 shipped four stale corpus
statements to review.

What this costs: the spec did not shape the implementation, so it describes what
exists rather than constraining what would be built. The `/speckit-analyze` pass
runs against artifacts written by the same author who wrote the code, which is
weaker than analysing artifacts written before it.

## Context

Every model call the server makes runs at the provider's default reasoning
effort, which is `high`. That default is right for `verify`, `decide` and
`elicit` — modes whose value *is* the reasoning. It is not obviously right for
`check`'s claim-to-formal-target translation or research's per-source claim
extraction, which are transcription-shaped work whose volume scales with the
size of a run.

Feature 018 established that a call site's *model* is the operator's to choose,
over a reserved `PARALLAX_MODEL_*` namespace with per-site and per-tier
settings resolved most-specific-first. Reasoning effort is the same kind of
decision about the same twelve call sites, and the provider exposes it as
`output_config.effort` with levels `low`, `medium`, `high`, `max`, `xhigh`.

This was a named deferral in 018's research (D7): "per-family `thinking`
suppression, to be decided on measured cost." The mechanism has since changed —
`thinking: {type: "disabled"}` returns a 400 on newer model families, and the
supported control is `output_config.effort`, which governs all output tokens
including thinking and needs no beta header.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - An operator spends deep reasoning only where it earns its keep (Priority: P1)

An operator sets a low effort for the transcription-shaped call sites and leaves
the judgment ones alone, without changing which model any of them uses.

**Why this priority**: This is the feature. Nothing else in it matters if the
setting does not reach the call sites the operator names.

**Independent Test**: Set a tier-level effort, resolve the table, and confirm
only that tier's call sites carry it.

**Acceptance Scenarios**:

1. **Given** an effort set on a tier, **When** the table resolves, **Then**
   every call site in that tier requests it and no site outside it does.
2. **Given** an effort set on a single call site, **When** the table resolves,
   **Then** that site requests it regardless of its tier's setting.
3. **Given** a call site with an effort but no model setting, **When** the table
   resolves, **Then** it keeps the default model and still requests the effort —
   the two settings are independent.

---

### User Story 2 - An upgrade changes nothing until the operator asks (Priority: P2)

An operator who sets nothing sees behaviour identical to before the feature
existed.

**Why this priority**: Constitution Principle VI. A capability that alters
requests on upgrade is a defect even when the capability is useful.

**Independent Test**: With the namespace unset, inspect the request body and
confirm it carries no effort field at all.

**Acceptance Scenarios**:

1. **Given** an empty effort namespace, **When** any call is made, **Then** the
   request body is byte-identical to before this feature — no effort key.
2. **Given** an empty effort namespace, **When** the table resolves, **Then** no
   call site carries an effort and the client count is unchanged.

---

### User Story 3 - A misspelled setting is refused, not ignored (Priority: P3)

An operator who mistypes a variable name or a level is told at startup.

**Why this priority**: Smallest, and the same rule 018 established for the model
namespace. A setting that silently does nothing leaves the operator believing a
call site changed when it did not.

**Independent Test**: Resolve with a misspelled suffix and with an invalid
level; both must be startup errors naming the variable.

**Acceptance Scenarios**:

1. **Given** an unrecognised suffix in the reserved namespace, **When** the
   server starts, **Then** it errors naming the variable.
2. **Given** a level outside the supported set, **When** the server starts,
   **Then** it errors naming both the variable and the accepted levels.

---

### Edge Cases

- Two call sites on the same model at different efforts: they must not share a
  client, or one site's effort rides on the other's calls.
- Two call sites on the same model at the same effort: they should share one
  client, as they did before this feature.
- An effort set on a tier and overridden on one of its call sites: the more
  specific wins, matching the model namespace.
- A value that is whitespace or empty: rejected as unparseable rather than
  silently treated as unset.
- A level accepted by the provider for some model families but not others: out
  of scope here — the server passes the level through and surfaces the
  provider's error. *(027: this happened, on the model this project's own
  quickstart recommends for bulk. The pass-through decision stands; 027 makes
  the surfaced error name which setting caused it.)*

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST let an operator set a reasoning effort per call
  site and per tier, over a reserved namespace distinct from the model one.
- **FR-002**: The system MUST resolve effort most-specific-first — call site
  over tier — and MUST resolve it independently of the model, so a call site may
  take one from a tier and the other from its own setting.
- **FR-003**: The system MUST send no effort field when none is set, leaving the
  request byte-identical to before this feature.
- **FR-004**: The system MUST reject an unrecognised name in the reserved
  namespace at startup, naming the variable.
- **FR-005**: The system MUST reject a value outside the supported levels at
  startup, naming both the variable and the accepted levels.
- **FR-006**: The system MUST build one client per distinct model-and-effort
  pair, so call sites differing in either do not share a client.
- **FR-007**: The system MUST keep the constrained-output contract intact — the
  effort field accompanies the output format rather than replacing it.
- **FR-008**: The design corpus and the configuration contract MUST be amended
  in this same change.

### Key Entities

- **Effort**: A reasoning level the operator may request for a call site. One of
  five ordered values; absent is distinct from any of them.
- **Resolved route**: A call site's model and effort together, each with the
  setting that supplied it.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With the namespace unset, the request body contains no effort key,
  and the number of clients built is unchanged from before this feature.
- **SC-002**: A tier-level effort reaches every call site in that tier and no
  other.
- **SC-003**: A per-site effort overrides its tier's for that site alone.
- **SC-004**: A call site can carry an effort while keeping the default model,
  and vice versa.
- **SC-005**: Two call sites on one model at different efforts result in two
  clients; at the same effort, one.
- **SC-006**: A misspelled variable and an invalid level each fail startup with
  a message naming the variable.

## Assumptions

- The provider's default when no effort is sent is `high`, so unset and an
  explicit `high` behave the same. Only unset is provably unchanged on the wire,
  which is why unset rather than `High` is the default state.
- Effort is a behavioural signal, not a token budget. `MAX_TOKENS` remains the
  ceiling and is unaffected.
- Which level suits which call site is an operator decision informed by
  measurement, not something the server should decide. This feature ships the
  control, not a recommended setting.
- Level support varies by model family. The server passes the level through; a
  family that rejects one surfaces the provider's error rather than the server
  second-guessing it.

  *(028, 2026-07-26: **the environment namespace was the wrong surface for this
  control, and 028 corrects it** — not by removing it, but by adding a per-call
  argument above it. The reasoning 022 never examined: which model runs a call
  site sets the rate the operator is billed at, so it is theirs; how much
  reasoning one invocation deserves is a per-task judgment the caller makes. 022
  mirrored 018's `PARALLAX_MODEL_*` shape because the machinery existed, while
  `research`'s `depth` and `recall`'s `limit` were already caller-facing in the
  same codebase. The consumer here is a model, so a setting reachable only by
  editing a file and restarting the session is unreachable in practice —
  changing it destroys the context that motivated the change. What 022 shipped
  stays correct as the **default layer**; see `specs/028-per-call-effort-argument/`
  and the operator-owned vs caller-owned test now in
  `docs/design/NEW_SERVER_DESIGN.md` §10.)*

  *(027, 2026-07-25: this statement was right and two other 022 artifacts
  contradicted it — `SDK_LANDSCAPE.md:285` and `018/research.md:169` both
  claimed every routed family accepts the parameter. They were the error and
  have been corrected. What "surfaces the provider's error" was always supposed
  to mean is now implemented: the client appends the model it sent, the level it
  sent, and both remedies. No capability table, no startup refusal — the fact
  comes from the provider at the moment it is true.)*
