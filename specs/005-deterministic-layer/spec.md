# Feature Specification: Deterministic Layer — Checkable Claims Settled by Execution, Not Judgment

**Feature Branch**: `005-deterministic-layer`

**Created**: 2026-06-12

**Status**: Draft

**Input**: User description: "Deterministic layer (DETERMINISTIC_LAYER.md, SDK_LANDSCAPE.md §deterministic): a large class of claims is checkable by execution, not judgment — and for those a solver beats a probabilistic critic: no judge to fool, no calibration knob, no sycophancy. One new MCP tool `check` implementing the translate→execute→feed-back pattern: the model translates a natural-language claim into a small typed formal target (arithmetic expression or logic/constraint problem), a deterministic engine executes it, and the result returns with the executed form shown — unforgeable. The symbolic feedback loop re-translates once on a REAL engine violation. The failure moves to translation, so the formal target stays small and typed, the engine's free signals (parse/infeasibility) are used, and 'not checkable' is an honest, first-class outcome with a bias toward declining (a false 'checkable' is the costliest mistake — a crisp wrong answer). v1 engines: arithmetic/quantitative evaluation and logic/constraint solving; schema validation already exists in-process. v1 defers, named: PAL-style arbitrary code execution (requires the off-by-default sandbox), planners, CAS."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Settle a checkable claim by execution (Priority: P1)

A calling model has a claim whose truth is computable — an arithmetic assertion
("a 37% reduction from 1,840 ms leaves 1,159 ms"), a constraint satisfaction
question ("these four scheduling rules admit no valid assignment"), a numeric
comparison buried in prose. Judging it with another model pass reintroduces the
exact failure mode being checked. Instead the caller hands the claim to
`check`: the claim is translated into a small formal problem, a deterministic
engine executes it, and the verdict comes back with the formal form and the
engine's result shown — so the caller (or a human) can audit exactly what was
computed. A solver cannot be argued past, and a fabricated result cannot occur.

**Why this priority**: This is the layer's thesis — the most reliable check in
the catalog precisely because there is no judge. Nothing else in the feature
matters without it.

**Independent Test**: Call `check` with a claim whose ground truth is known
(true arithmetic, false arithmetic, satisfiable constraints, unsatisfiable
constraints). Verify the verdict matches ground truth, the response carries
the executed formal form and engine result, and the verdict came from
execution (visible in the response), not from model opinion.

**Acceptance Scenarios**:

1. **Given** a true arithmetic claim, **When** `check` runs, **Then** the
   verdict is supported, with the evaluated expression and computed value in
   the response.
2. **Given** a false arithmetic claim, **When** `check` runs, **Then** the
   verdict is refuted, and the response shows the expression, the computed
   value, and the claimed value it contradicts.
3. **Given** a claim about constraints that are jointly unsatisfiable,
   **When** `check` runs, **Then** the verdict reflects what the solver
   proved, with the constraint formulation in the response.
4. **Given** a satisfiable constraint problem asserted to be impossible,
   **When** `check` runs, **Then** the verdict is refuted and the response
   includes a concrete satisfying assignment (the witness) from the solver.
5. **Given** the same claim checked twice, **When** both runs complete with
   the same formalization, **Then** the engine result is identical
   (deterministic execution).

---

### User Story 2 - Honest refusal on uncheckable claims (Priority: P2)

The caller hands `check` a judgment call ("this design is cleaner"), an
open-ended question, or a claim that depends on world knowledge no engine
holds. Forcing such a claim into a formal target produces false precision — a
crisp answer to the wrong question, the costliest failure this layer can have.
Instead the tool declines explicitly: a distinct not-checkable outcome naming
why, so the caller routes the claim to the probabilistic `verify` instead.
The classifier is biased toward declining: when checkability is uncertain,
the answer is "not checkable."

**Why this priority**: The design doc names the "looks checkable but isn't"
trap as a hard problem. Without an honest decline path, the layer silently
converts judgment calls into confident wrong answers — worse than not
existing.

**Independent Test**: Call `check` with clearly uncheckable claims (taste,
prediction, open-ended) and verify each returns the distinct not-checkable
outcome with a reason, having consumed only the translation attempt — never a
fabricated formal verdict.

**Acceptance Scenarios**:

1. **Given** a judgment claim ("X is more elegant than Y"), **When** `check`
   runs, **Then** the result is the distinct not-checkable outcome with a
   stated reason, not a verdict.
2. **Given** a claim requiring unavailable world knowledge ("the third-floor
   meeting room is the largest"), **When** `check` runs, **Then** the result
   is not-checkable — never an invented formalization.
3. **Given** a mixed claim (a checkable arithmetic core wrapped in judgment),
   **When** `check` runs, **Then** either the checkable core is checked —
   with the returned formal form making the checked scope explicit and
   auditable (the formal form IS the coverage statement) — or the claim is
   declined; never a verdict that silently covers the judgment part.

---

### User Story 3 - The symbolic feedback loop and translation defenses (Priority: P3)

Execution is now exact, so the failure surface is translation: the model can
formalize the wrong problem. The layer defends the narrowed surface: the
engine's free signals (parse failure, type error, infeasibility) trigger one
re-translation with the real violation fed back — a re-prompt on ground truth,
not on a critic's opinion; the formal target stays small and typed; and the
response always carries the formal form so an unfaithful translation is
auditable rather than hidden. Failures of translation after the retry surface
as a distinct outcome, never as a confident verdict.

**Why this priority**: The design doc's "one honest catch" — without these
defenses the layer trades a visible probabilistic failure for an invisible
one.

**Independent Test**: Force a first translation that the engine rejects
(unparseable form) and verify a single violation-fed retry occurs; force both
attempts to fail and verify the invocation surfaces a translation failure
class, not a verdict. Verify every successful response carries the executed
formal form.

**Acceptance Scenarios**:

1. **Given** a first translation the engine rejects as malformed, **When**
   the engine reports the real violation, **Then** exactly one re-translation
   carrying that violation occurs, and a valid second form proceeds to
   execution.
2. **Given** two failed translations, **When** the retry also fails, **Then**
   the invocation fails with a distinct translation-failure class — no
   verdict is synthesized.
3. **Given** any successful check, **When** the response is inspected,
   **Then** it contains the formal form that was executed and the engine's
   raw result, sufficient to audit the translation without re-running it.

---

### Edge Cases

- A claim that is checkable but whose formalization times out in the engine →
  the timeout class with a solver-naming message; never an opinion-based
  fallback verdict.
- The translation is syntactically valid but the engine result is neither
  supports nor refutes the claim (e.g. the formalization answered a different
  question detectably — result type mismatch) → translation failure, not a
  verdict.
- An empty, whitespace-only, or oversized claim → rejected before any model
  call under the invalid-input class naming the configured limit.
- The model declines to translate (refusal) → the existing refusal class
  surfaces unchanged.
- Numeric claims involving tolerance ("roughly", "about") → the translation
  must make the tolerance explicit in the formal form (visible, auditable);
  a claim too vague to bound is not checkable.
- Division by zero, overflow, or domain errors during evaluation → engine
  violation, fed to the single retry; if inherent to the claim, the result
  reports the domain error honestly.
- Cancellation mid-check → one record with the cancelled outcome.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide a `check` tool accepting a
  natural-language claim plus optional context, and returning: the verdict
  (supported | refuted), the formal form that was executed, the engine's raw
  result, the engine used, and a plain-language explanation tying the result
  to the claim.
- **FR-002**: Verdicts MUST come only from deterministic engine execution.
  The model's roles are exactly: classifying checkability, translating to
  the formal target, and explaining the engine result — never deciding the
  verdict.
- **FR-003**: v1 MUST support two engine families: arithmetic/quantitative
  evaluation (numeric expressions, comparisons, explicit tolerances) and
  logic/constraint solving (satisfiability with witnesses, proofs of
  unsatisfiability). The engine choice for a claim is part of translation
  and MUST be visible in the response.
- **FR-004**: Claims classified as not checkable MUST return a distinct
  not-checkable outcome with a stated reason — never a forced
  formalization. The classifier MUST be biased toward declining when
  uncertain.
- **FR-005**: A real engine violation (parse failure, type error, malformed
  constraint set) MUST trigger exactly one re-translation carrying the
  violation verbatim; a second failure MUST surface as a distinct
  translation-failure class. The system MUST NOT synthesize a verdict from a
  failed translation.
- **FR-006**: Engine execution MUST be bounded: a per-check engine timeout
  that, when exceeded, surfaces under the timeout class with a message
  naming the solver (message-distinct, per the established class-naming
  convention). Solver and evaluator run in-process with no network and no
  filesystem effects.
- **FR-007**: Every response MUST carry the executed formal form and raw
  engine result (auditability — the translation-faithfulness defense); a
  refuted satisfiability claim MUST include the solver's witness when one
  exists.
- **FR-008**: Input MUST be validated before any model call (non-empty,
  within the configured input bound) under the invalid-input class.
- **FR-009**: Every invocation MUST leave exactly one invocation record on
  every exit path with cost/latency/token/outcome accounting consistent
  with the existing tools.
- **FR-010**: The tool requires no new credentials and is always in the
  catalog (the engines are in-process and pure — no effects beyond the
  process, so no capability gate is needed; the existing model credential
  covers translation).
- **FR-011**: Arbitrary code execution (PAL-style), CAS, and planners are
  explicitly out of scope for v1 — named deferrals; code execution remains
  off pending the sandboxed-execution capability.

### Key Entities

- **Check request**: the claim; optional context.
- **Check result**: verdict (supported | refuted), engine id, formal form
  (text), raw engine result (text), witness (optional — satisfying
  assignment or counterexample), explanation, translation attempts used.
- **Formal target (internal)**: the small typed translation contract per
  engine family — an arithmetic comparison with explicit tolerance, or a
  constraint problem over typed variables with an asserted
  satisfiability/unsatisfiability polarity.
- **Engine verdict (internal)**: the deterministic execution outcome —
  evaluated value / sat-with-model / unsat — mapped to supported/refuted by
  pure comparison against the claim's asserted polarity.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On an acceptance set of ≥ 20 claims with known ground truth
  (true/false arithmetic, sat/unsat constraints), 100% of returned verdicts
  match ground truth — the engine, not the model, decides.
- **SC-002**: On a set of ≥ 6 clearly uncheckable claims, 100% return the
  not-checkable outcome; 0 produce a verdict.
- **SC-003**: 100% of successful responses carry the executed formal form
  and raw engine result.
- **SC-004**: An induced malformed first translation recovers via exactly
  one violation-fed retry; induced double failure surfaces the
  translation-failure class. 0 verdicts synthesized from failed
  translations.
- **SC-005**: The complete pre-existing test suite passes unchanged, and
  `check` appears in the catalog with no new environment variables
  required.
- **SC-006**: 100% of invocations (success and every failure class) leave
  exactly one correctly attributed invocation record.
- **SC-007**: A repeated check with the same formalization produces an
  identical engine result (determinism).

## Assumptions

- Two engine families in v1 (arithmetic, logic/constraints); the schema
  validator already serves the format-check row internally. Engine crates
  are chosen at planning time per `SDK_LANDSCAPE.md` §deterministic.
- The checkability classification and the translation happen in the same
  model call (one constrained output that either declines or produces a
  formal target) — fewer hops, and the decline bias is in the prompt.
- "Not checkable" is a successful invocation outcome (the tool did its job),
  not an error class; translation failure after retry IS an error class.
- The formal targets are deliberately small: arithmetic comparison with
  explicit tolerance; constraint problems over integer/real/boolean
  variables with linear arithmetic — expressiveness grows only with
  evidence of need (YAGNI).
- Back-translation/round-trip checking and multi-formalization ensembles
  are deferred until live acceptance shows translation faithfulness is a
  measured problem at this target size (the design doc's "keep the target
  small" defense is v1's primary mitigation).
- English-language claims are the acceptance target.
- The solver dependency builds vendored/bundled (no system installation
  required); a build-time spike validates this on Windows and CI before the
  pipeline depends on it.
