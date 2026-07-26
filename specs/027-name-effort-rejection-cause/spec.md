# Feature Specification: Name the Cause When the Provider Rejects an Effort Level

**Feature Branch**: `027-name-effort-rejection-cause`

**Created**: 2026-07-25

**Status**: Draft

**Input**: "Setting an effort on a call site routed to a model that rejects the parameter makes every call in that phase fail. The provider's message says *this model*; the operator has to work out which of their settings caused it."

## Context

Feature 022 added `PARALLAX_EFFORT_*`, setting a provider reasoning-effort level
per model call site. Setting `PARALLAX_EFFORT_BULK=low` while the bulk tier was
routed to `claude-haiku-4-5` made every extraction call return
`HTTP 400: This model does not support the effort parameter`. The run wasted its
search and page-fetch spend, and — until feature 023 — reported success with an
empty answer.

023 fixed the silence. What remains is that the surfaced message describes the
provider's view (*this model*) and not the operator's (*this setting*). The
client holds both facts the message omits: which model it is, and what effort it
sent.

### 022's own decision, implemented rather than reversed

`spec.md:186` of feature 022 records: *"Level support varies by model family.
The server passes the level through; a family that rejects one surfaces the
provider's error rather than the server second-guessing it."*

That decision stands. This feature makes the surfaced error actually useful,
which is what "surfaces the provider's error" was always supposed to mean.

### A rejected alternative, and why

An earlier plan proposed a compiled model-capability table refusing startup on a
known-rejecting pairing. Two adversarial reviews rejected it, and the reasons
belong here so it is not re-proposed:

- Its `decide` justification compared closing this gap against *adding per-call
  effort*, which a `verify` pass had already found to be a downstream dependency
  rather than a rival. A false dilemma — no alternative design was scored.
- Its `verify` refutation addressed fail-closed on **unknown** values. It then
  applied fail-closed to **known** values with no override, a case the
  refutation never covered. The most probable change to such a table is effort
  support extending to cheaper models — exactly the entry that would then refuse
  a valid configuration, fixable only by rebuilding.
- It matched model ids exactly, so it prevented one *string*.
  `claude-haiku-4-5-20251001` would have reproduced the failure with the check
  in place.
- `018/quickstart.md:9-17` recommends `PARALLAX_MODEL_BULK=claude-haiku-4-5`;
  022's motivation is that bulk deserves low effort. Under that plan the
  corpus's two standing recommendations were jointly a dead server.

### A false statement already released

022 shipped `docs/design/SDK_LANDSCAPE.md:283-285` and
`specs/018-model-routing/research.md:168-170` both stating that
`output_config.effort` is *"accepted by every routed family"*. That was false
when written — the support list excluding Haiku 4.5 had already been read the
same day. `CHANGELOG.md:61` repeats it inside the released `[0.2.0]` block. 022
also contradicts itself: `spec.md:186` says support varies by family.

Correcting this is part of the feature, not an afterthought (Principle I).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The rejection says which setting caused it (Priority: P1)

An operator sets an effort level on a call site whose model rejects the
parameter. Calls fail. The error tells them which model was sent which level, so
the setting to change is immediate.

**Why this priority**: This is the feature. The failure is already loud (023);
what is missing is that it is not yet self-diagnosing.

**Independent Test**: With a client carrying an effort, return a 400 whose body
names the effort parameter; confirm the error names both the model and the level.

**Acceptance Scenarios**:

1. **Given** a call site with an effort set, **When** the provider rejects the
   request for that parameter, **Then** the error names the model, the level,
   and both remedies — unset the variable, or route the site to a model that
   accepts effort.
2. **Given** the same failure, **When** the caller reads the error, **Then** the
   provider's own message is still present, not replaced.

---

### User Story 2 - Every other failure is untouched (Priority: P2)

A failure unrelated to effort reads exactly as it did before.

**Why this priority**: The enrichment is a guess about *why* a request was
rejected. Applying it to a rejection with another cause would put a confident
wrong diagnosis in front of the operator — worse than the bare message.

**Independent Test**: Return a 400 on a client with no effort set, and a
non-400 failure on a client with an effort set; confirm both messages are
unchanged.

**Acceptance Scenarios**:

1. **Given** a client with no effort configured, **When** any request is
   rejected, **Then** the message is unchanged.
2. **Given** a client with an effort configured, **When** a failure is not a 400,
   **Then** the message is unchanged.
3. **Given** a client with an effort configured, **When** a 400 arrives whose
   body does not name the effort parameter, **Then** the message is unchanged.

---

### Edge Cases

- A provider body that names the parameter but rejects for a different reason:
  the diagnosis is appended, not substituted, so the operator sees both and can
  judge.
- A provider that changes its rejection wording: the guard stops matching and
  the message degrades to today's behaviour. Losing the enrichment is the safe
  direction — no false diagnosis, and no failure to start.
- Several call sites sharing one client: the message names the model and the
  level, which identifies the setting without needing the call site.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: When a request carrying an effort level is rejected for that
  parameter, the error MUST name the model that rejected it and the level that
  was sent.
- **FR-002**: The error MUST state both remedies: unset the variable covering
  the call site, or route the site to a model that accepts effort.
- **FR-003**: The provider's own message MUST remain in the error, not be
  replaced by the diagnosis.
- **FR-004**: A failure that does not carry an effort, is not a rejection of
  that parameter, or is not a client-error status MUST produce an unchanged
  message.
- **FR-005**: The server MUST NOT decide in advance which models accept the
  parameter; the fact comes from the provider at the moment it is true.
- **FR-006**: The corpus statements asserting universal family acceptance MUST
  be corrected, and the released changelog claim MUST be corrected in the
  next release block rather than rewritten in place.
- **FR-007**: The `PARALLAX_MODEL_*` and `PARALLAX_EFFORT_*` namespaces MUST be
  documented in the operator-facing configuration references, which enumerate
  every other environment variable and omit both.

### Key Entities

- **Rejection diagnosis**: The model, the level sent, and the two remedies —
  appended to a provider rejection that names the effort parameter.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator who has caused this failure can identify the setting
  to change from the error text alone, without consulting documentation or
  source.
- **SC-002**: A failure with any other cause produces a message identical to
  before this change.
- **SC-003**: A model family released after the binary was built is handled
  identically to one known at build time — no configuration is refused, and no
  diagnosis is missed.
- **SC-004**: No configuration that would have worked is prevented from
  starting.
- **SC-005**: No corpus statement asserts that every routed family accepts the
  effort parameter.

## Assumptions

- The provider's rejection body names the parameter. Observed once, on
  2026-07-25: *"This model does not support the effort parameter."* If the
  wording changes the guard stops matching, which degrades to today's message
  rather than to a wrong one.
- Naming the model and the level identifies the responsible variable adequately.
  Naming the variable itself would require one client per call site, discarding
  the pooling that keys on `(model, effort)`.
- Saving the search and fetch spend consumed before the failing phase is out of
  scope. That is a pipeline ordering property, not a knowledge gap, and post-023
  the loss is bounded at one run.
