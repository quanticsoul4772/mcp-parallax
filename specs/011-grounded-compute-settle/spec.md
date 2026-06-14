# Feature Specification: Grounded Compute-Settle

**Feature Branch**: `011-grounded-compute-settle`

**Created**: 2026-06-14

**Status**: Draft

**Input**: User description: "grounded_verify settles computable claims via the check
engine instead of abstaining. Today (010) when a majority of grounded_verify passes set
needs_computation, the server returns an inconclusive verdict routing the caller to
check. This feature makes grounded_verify actually settle the computable sub-claim
itself: the server already holds the verbatim bytes it read, so instead of handing the
work back, it computes the property (e.g. counts the lines of the named source) and runs
the existing deterministic check engine over that value to decide the claim. Example:
'src/server.rs is over 1000 lines' against the 1224-line file should return supported
with the executed form (1224 > 1000), not inconclusive. Detection reuses the per-pass
needs_computation signal from 010. The computation is limited to the simple, safe
properties the passes flag (line counts, byte/size measures, match counts, numeric
comparisons over those); anything it cannot compute cheaply still abstains with
inconclusive as 010 does. The check translation and engine are reused, not
reimplemented. This is the named follow-up deferred by 010 FR-005."

## User Scenarios & Testing *(mandatory)*

010 made `grounded_verify` stop emitting a confident wrong verdict on a computable
claim: when a majority of passes self-report `needs_computation`, it returns
`inconclusive` and tells the caller to route the claim to `check`. That removed the
*confidently-wrong* failure, but it stops one step short — the server is holding the
exact bytes the computation needs, yet hands the work back instead of finishing it.
This feature closes that step: for the narrow class of properties the passes already
flag as computable (a count, a measure, a numeric comparison over those), the server
computes the value deterministically from the bytes it read and lets the existing
`check` engine decide the claim — returning a settled `supported`/`refuted` with the
executed form, exactly as `check` does. The abstain path stays as the fallback for
anything outside that narrow, safe class, so 010's no-confidently-wrong guarantee is
never weakened.

### User Story 1 - A countable claim is settled, not bounced (Priority: P1)

A caller asks `grounded_verify` whether a named source has more than N lines (or
contains more than N matches of a literal, or is larger than N bytes). Today the passes
flag it computable and the tool returns `inconclusive`, leaving the caller to count the
lines themselves and call `check`. This story makes the server do that work: it counts
the property over the verbatim bytes it already read, runs `check` on the resulting
comparison, and returns the deterministic verdict with the executed form and the
engine's raw result — the same auditable shape `check` returns.

**Why this priority**: This is the whole feature — turning the abstention into an
answer for the cases where the server already has everything it needs. It directly
applies the project's deterministic-over-probabilistic principle: a counted, executed
result replaces both a judged guess (the pre-010 bug) and a punt (the 010 abstention).

**Independent Test**: Issue the reproduction claim ("`src/server.rs` is over 1000
lines" against the 1224-line file) and confirm the verdict is `supported` with an
executed form equivalent to `1224 > 1000` and the engine's result — not `inconclusive`,
and not a judged refutation. Fully testable through tool output.

**Acceptance Scenarios**:

1. **Given** a claim that is a numeric comparison over a countable property of a single
   named source ("file X has more than N lines"), **When** `grounded_verify` runs and
   the passes flag it computable, **Then** the server counts the property over the read
   bytes, runs `check` on the comparison, and returns `supported`/`refuted` with the
   executed form and the engine's raw result.
2. **Given** the reproduction claim — "`src/server.rs` exceeds 1000 lines", the file is
   1224 lines — **When** `grounded_verify` runs, **Then** it returns `supported` with
   an executed form equivalent to `1224 > 1000`, **not** `inconclusive` and **not** a
   judged refutation.
3. **Given** a computable claim whose comparison is false ("file X has more than 5000
   lines", file is 1224), **When** `grounded_verify` runs, **Then** it returns
   `refuted` with the executed `1224 > 5000` form — the deterministic engine decides
   direction, not a judge.

### User Story 2 - Anything not cheaply computable still abstains (Priority: P1)

A caller asks `grounded_verify` a claim the passes flag as needing computation, but the
property is one the server cannot compute cheaply and safely from the bytes (it needs
parsing the server does not do, spans multiple sources in a way the count is ambiguous,
or the comparison does not reduce to the supported value class). This story guarantees
that case falls back to 010's behavior exactly: an `inconclusive` verdict routing to
`check`. The server never guesses a count it cannot derive, and never weakens 010's
no-confidently-wrong guarantee.

**Why this priority**: P1 because it is the safety boundary of US1 — the compute path
is only allowed to settle the narrow class it can compute exactly. Without this
explicit fallback, broadening detection would risk a computed-but-wrong verdict, which
is the very failure 010 removed. The two stories ship together: US1 is the new answer,
US2 is the guarantee it never overreaches.

**Independent Test**: Issue a computable-flagged claim whose property is outside the
supported class (e.g. "the busiest function in file X is over 50 lines" — requires
parsing) and confirm the verdict is `inconclusive` (route to `check`), never a computed
verdict over a value the server could not actually derive.

**Acceptance Scenarios**:

1. **Given** a passes-flagged-computable claim whose property the server cannot compute
   from the bytes without parsing it does not perform, **When** `grounded_verify` runs,
   **Then** it returns `inconclusive` (route to `check`), exactly as 010 — no computed
   verdict over an underived value.
2. **Given** a claim the passes do **not** flag computable (`needs_computation` not a
   majority), **When** `grounded_verify` runs, **Then** the stance-blind judgment path
   is unchanged — `supported`/`refuted` from the passes, no computation attempted.

### Edge Cases

- **Ambiguous count over multiple sources**: a comparison whose property would be
  counted across several locators (e.g. "these files total more than N lines"). The
  decomposition decides whether a well-defined aggregate count exists; if it is
  ambiguous, abstain (US2). (`/speckit-plan` settles the aggregation rule.)
- **The computed comparison and a judgment both matter** (compound claim — part
  countable, part judged): which result governs the verdict, and is the executed form
  still surfaced? (`/speckit-plan`.)
- **A property the passes flag but the comparison target is non-numeric** (e.g. "file X
  has an even number of lines" — a parity predicate, not a `>`/`<`): whether the
  supported class includes such predicates or abstains on them. (`/speckit-plan`.)
- **Count of zero / empty file**: the computed value is `0`; the comparison must still
  execute deterministically (`0 > 1000` → refuted), not be treated as missing.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: When a majority of `grounded_verify` passes flag a claim computable
  (`needs_computation`, 010) **and** the claim reduces to a supported computable
  property of the read source compared numerically (US1's class), the server MUST
  compute the property over the verbatim bytes it already read and settle the claim
  via the existing `check` engine — returning `supported`/`refuted` rather than
  `inconclusive`.
- **FR-002**: The settled output MUST carry the **executed form** (the concrete
  comparison decided, e.g. `1224 > 1000`) and the engine's raw result, so the verdict
  is auditable in the same way a direct `check` call is.
- **FR-003**: The server MUST compute the property deterministically from the bytes it
  read — never a model estimate. The model's role stays bounded to flagging
  computability (010) and identifying the property/threshold; the *value* is counted by
  the server and the *decision* is the engine's.
- **FR-004**: The supported computable class MUST be limited to simple, safe properties
  derivable from the read bytes without external execution or parsing the server does
  not already perform: at minimum line counts, byte/size measures, and counts of a
  literal match, each compared with a numeric threshold. Properties outside this class
  MUST fall through to FR-005.
- **FR-005**: When the claim is flagged computable but the property is **not** in the
  supported class (FR-004) — or the value cannot be derived unambiguously — the server
  MUST fall back to 010's `inconclusive` verdict (route to `check`). The compute path
  MUST NEVER emit a verdict over a value it could not actually derive (no regression of
  010's no-confidently-wrong guarantee).
- **FR-006**: The stance-blind judgment path for non-computable claims (010 FR-007)
  MUST be unchanged: a claim the passes do not flag computable is judged by the passes
  as today, with no computation attempted.
- **FR-007**: The `check` translation and engine MUST be reused, not reimplemented. The
  server constructs the comparison from the counted value and the model-identified
  threshold and submits it to the existing deterministic engine.
- **FR-008**: The feature MUST ship with acceptance tests reproducing the motivating
  case: "`src/server.rs` is over 1000 lines" against the 1224-line file returns
  `supported` with the `1224 > 1000` executed form; a false-comparison variant returns
  `refuted`; an out-of-class computable claim returns `inconclusive`.

### Key Entities

- **Computable property**: a value derivable deterministically from the read source
  bytes — a line count, a byte/size measure, a count of a literal match. Bounded to the
  supported class (FR-004).
- **Executed form**: the concrete comparison the engine decided (counted value vs
  threshold), surfaced for audit alongside the verdict (FR-002), mirroring `check`.
- **Threshold / comparison**: the numeric bound and operator the claim asserts (e.g.
  `> 1000`), identified from the claim; combined with the computed value to form the
  executed comparison.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The reproduction claim "`src/server.rs` exceeds 1000 lines" returns
  `supported` with an executed form equivalent to `1224 > 1000` for the actual
  1224-line file — not `inconclusive`, not a judged verdict.
- **SC-002**: A false-comparison variant ("exceeds 5000 lines") on the same file
  returns `refuted` with the executed `1224 > 5000` form — the engine decides direction.
- **SC-003**: A computable-flagged claim whose property is outside the supported class
  returns `inconclusive` (route to `check`) on **100%** of runs — the compute path
  never emits a verdict over an underived value (010 guarantee preserved).
- **SC-004**: Non-computable claims retain **100%** of their 010 judgment-path verdicts
  — no regression from introducing the compute path.
- **SC-005**: Every settled compute verdict includes the executed form and the engine's
  raw result, so the decision is auditable without re-running it.

## Assumptions

- 010 is merged and provides the per-pass `needs_computation` signal and the
  server-assembled `inconclusive` verdict that this feature's fallback (FR-005) reuses.
- The deterministic `check` engine (005) is the reference settler; this feature feeds it
  a server-counted value rather than re-implementing arithmetic/constraint solving.
- The supported computable class (FR-004) and the property/threshold extraction and
  decomposition rules are **`/speckit-plan` decisions**, consistent with how 009/010
  deferred mechanism choices to planning. The narrow class named here (line/byte/match
  counts compared to a numeric threshold) is the v1 floor; broadening it is a later
  follow-up, not this feature.
- `grounded_verify`'s root-confinement, locator model (008/009), evidence manifest, and
  the stance-blind judgment path are unchanged; this feature adds a settle path between
  the abstain decision and the `inconclusive` return.
- The model still never authors the value or the verdict — it flags computability and
  the property; the server counts and the engine decides (deterministic-over-
  probabilistic, the principle 010 began applying here).
