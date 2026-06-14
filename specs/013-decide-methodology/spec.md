# Feature Specification: Decide — Methodology-Driven Choice

**Feature Branch**: `013-decide-methodology`

**Created**: 2026-06-14

**Status**: Draft

**Input**: User description: "Decide — a decision corrective for indecision and
miscalibration. When the model faces a choice among options under tradeoffs, in-context
deliberation tends to either stall (endless weighing) or commit on a gut feel poorly
calibrated to the actual criteria. Decide applies an explicit decision methodology —
weigh (score the options against named criteria), causal (trace what each option causes
or prevents), or probabilistic (reason under stated uncertainty) — and returns a single
recommended option with the reasoning surfaced: the criteria that drove it, the runner-up
and why it lost, and a calibrated confidence. Distinct from unstick (one next step when
looping) and verify (judge a claim true/false): Decide chooses among given options under
explicit tradeoffs. The methodology is matched to the decision's shape. The output is a
recommendation plus rationale, never a menu and never a hidden gut call. It reuses the
constrained-output contract: the methodology selection and per-option assessment are the
model's structured output; the recommendation and calibration are server-assembled from
that assessment so the choice traces to the stated criteria. Always in the catalog, no
new gate."

## User Scenarios & Testing *(mandatory)*

Parallax catches the ways the calling model reliably goes wrong from inside its own
context. **Indecision and miscalibration on a choice** is one of those: facing several
options under tradeoffs, in-context deliberation either stalls (re-weighing the same
factors without converging) or snaps to a gut pick whose confidence is untethered from
the criteria that should drive it. The model cannot audit its own decision process from
inside it. `Decide` is the external pass that imposes an explicit methodology — score the
options against named criteria, trace what each causes, or reason under the stated
uncertainty — and returns a recommendation whose rationale is visible and whose
confidence is grounded in the assessment, not a feeling.

It is the **complement** of the existing correctives: `verify` judges whether a claim is
true; `unstick` commits to one next step when looping (no options needed); `diverge`
opens up framings; `Decide` **chooses among given options under explicit tradeoffs** and
shows its work. Where `unstick` produces motion from a stuck state, `Decide` produces a
*justified selection* from a set of candidates.

### User Story 1 - A justified recommendation, not a gut pick (Priority: P1)

A caller has two or more options and a decision to make, and asks `Decide`. Today the
model, asked in-context to "just pick," tends to assert a choice with confidence that
doesn't trace to the criteria. `Decide` applies an explicit methodology: it assesses each
option against the decision's relevant factors, names the recommended option, names the
runner-up and **why it lost**, and reports a confidence that is derived from the
assessment (a close call reads as lower confidence than a dominant winner). The
recommendation traces to the stated factors — it is not an unexamined preference.

**Why this priority**: This is the whole value — turning an unexamined gut call into a
justified, auditable selection. It applies the project's deterministic-over-probabilistic
posture to the *assembly*: the model assesses, but the recommendation and its calibration
are server-derived from that structured assessment, so the choice cannot be a hidden
preference dressed up with confidence.

**Independent Test**: Submit a decision with 2–4 options and confirm the output names one
recommended option, identifies the runner-up with a reason it lost, surfaces the factors
that drove the choice, and reports a confidence consistent with how close the call is.
Fully testable through the tool's output.

**Acceptance Scenarios**:

1. **Given** a decision with several options under tradeoffs, **When** `Decide` runs,
   **Then** it returns exactly one recommended option, the runner-up and why it lost, the
   deciding factors, and a confidence — never a menu handed back unresolved.
2. **Given** a decision with a clearly dominant option, **When** `Decide` runs, **Then**
   it recommends that option with high confidence and names the factors that make it
   dominant.
3. **Given** a genuinely close call, **When** `Decide` runs, **Then** the reported
   confidence is **lower** than the dominant case — calibration tracks how close the
   options are, not a fixed high value.

### User Story 2 - The methodology matches the decision's shape (Priority: P1)

A caller's decision has a shape: it may turn on **multiple weighted criteria**, on the
**downstream effects** each option causes or prevents, or on **uncertainty** about which
outcome obtains. `Decide` selects and applies the methodology that fits — weighing,
causal tracing, or probabilistic reasoning — rather than forcing one frame onto every
decision, and the chosen methodology is surfaced so the caller can see how the choice was
reached.

**Why this priority**: P1 because a single fixed frame miscalibrates: scoring criteria on
a decision that actually turns on a causal chain (or on uncertainty) produces a confident
but wrong-shaped rationale. Matching the methodology to the decision is what makes the
rationale *fit* the choice, and surfacing it is what makes the choice auditable.

**Independent Test**: Submit a multi-criteria decision and a downstream-effects decision
and confirm the surfaced methodology differs appropriately, and that the rationale is
expressed in the terms of that methodology (criteria scores vs. caused/prevented effects).

**Acceptance Scenarios**:

1. **Given** a decision that turns on several named criteria, **When** `Decide` runs,
   **Then** the methodology surfaced is the weighing one and the rationale references the
   criteria the options were assessed against.
2. **Given** a decision that turns on what each option causes or prevents downstream,
   **When** `Decide` runs, **Then** the methodology surfaced is the causal one and the
   rationale references those effects.
3. **Given** a decision dominated by uncertainty about which outcome obtains, **When**
   `Decide` runs, **Then** the methodology surfaced is the probabilistic one and the
   rationale references the likelihoods/uncertainty.

### Edge Cases

- **A single option (or none)**: there is no choice to make — `Decide` must reject the
  call as invalid input rather than manufacture a fake comparison. (Two options minimum.)
- **No criteria provided**: the methodology must derive the relevant factors from the
  decision itself; `Decide` does not require the caller to pre-supply criteria, but it
  surfaces the factors it used so the choice stays auditable.
- **A tie on the merits**: when the assessment genuinely does not separate the top two,
  the output must say so (lowest-confidence band) rather than fabricate a separating
  factor — explicit non-separation over a manufactured tiebreak.
- **The decision is actually a factual question, a stuck loop, or a computable claim**:
  out of scope — route a truth question to `verify`, a stuck loop to `unstick`, a
  computable comparison to `check`. `Decide` chooses among options under tradeoffs; it
  does not establish facts.
- **Empty / oversize decision statement**: rejected before any model call, like the rest
  of the family (no silent trimming).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `Decide` MUST accept a decision (the question plus **two or more** options)
  and optional context/criteria, and return a single **recommended option** with its
  rationale.
- **FR-002**: The output MUST include the **runner-up** and a stated reason it lost, the
  **deciding factors**, and a **confidence** — the tradeoffs are made explicit, never a
  bare pick.
- **FR-003**: `Decide` MUST select and **surface a decision methodology** matched to the
  decision's shape (multi-criteria → weighing; downstream effects → causal; uncertainty →
  probabilistic), and express the rationale in that methodology's terms.
- **FR-004**: The recommendation and the confidence MUST be **server-assembled from the
  model's structured per-option assessment** — the choice traces to the assessed factors,
  not an unexamined model preference asserted directly.
- **FR-005**: The reported confidence MUST be **calibrated to how close the decision is**:
  a dominant winner yields high confidence; a near-tie yields low confidence (it is not a
  fixed or model-self-reported constant).
- **FR-006**: `Decide` MUST return a **resolved recommendation**, never a menu handed back
  unresolved — it does the choosing (distinct from listing options).
- **FR-007**: `Decide` MUST stay within scope: it chooses among given options under
  tradeoffs; it MUST NOT be the path for a truth claim (`verify`), a stuck loop
  (`unstick`), or a computable comparison (`check`). The output is a recommendation, never
  a truth verdict or a single next action divorced from the option set.
- **FR-008**: Input validation MUST reject a decision with **fewer than two options** or
  an empty/oversize statement before any model call (no silent trimming; no fabricated
  comparison).
- **FR-009**: `Decide` MUST be **always in the catalog** — no new capability gate or env
  flag (like `verify` and `unstick`).

### Key Entities

- **Decision**: the question to settle plus the set of candidate options (two or more) and
  optional context/criteria — the primary input.
- **Option assessment**: the model's structured evaluation of one option under the chosen
  methodology (its standing on the deciding factors). The set of assessments is what the
  server derives the recommendation from.
- **Methodology**: the named decision frame applied — weighing (criteria), causal
  (effects), or probabilistic (uncertainty) — surfaced in the output.
- **Recommendation**: the server-assembled result — the chosen option, the runner-up and
  why it lost, the deciding factors, the surfaced methodology, and the calibrated
  confidence.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a decision with a clearly dominant option, `Decide` recommends that
  option and reports **high** confidence with the dominating factors named — the choice is
  justified, not asserted.
- **SC-002**: On a genuinely close call, `Decide` reports a **lower** confidence than the
  dominant case on a fixed battery — calibration tracks closeness, not a constant.
- **SC-003**: **100%** of `Decide` outputs include a recommended option, a runner-up with
  a stated reason it lost, the deciding factors, and a surfaced methodology — never a bare
  pick or an unresolved menu.
- **SC-004**: The surfaced methodology **matches the decision's shape** on a fixed battery
  of multi-criteria, causal, and uncertainty decisions (each yields the fitting
  methodology), demonstrating the frame is not one-size-fits-all.
- **SC-005**: A decision with fewer than two options is rejected as invalid input on
  **100%** of attempts — no fabricated comparison.

## Assumptions

- `Decide` is a corrective named in the design corpus (`NEW_SERVER_DESIGN.md` four-layer
  catalog — *indecision / miscalibration → methodology: weigh / causal / probabilistic*);
  this feature implements an existing catalog entry, and the constitution's
  design-corpus-fidelity check is an application, not a deviation (confirmed at
  `/speckit-plan`).
- It reuses the constrained-output contract: the per-option assessment and the
  methodology selection are the model's flat structured output; the recommendation, the
  runner-up determination, and the calibrated confidence are **server-assembled** from
  that assessment.
- The concrete methodology set, how the per-option assessment is structured, the
  server-side recommendation/calibration rule (how confidence maps to closeness), and
  whether a single pass or an ensemble is used are **`/speckit-plan` decisions** (mirroring
  how `verify`/`diverge` deferred lens/mechanism choices to planning).
- The minimum option count is **two**; the maximum and the input bound reuse the existing
  `INPUT_MAX_CHARS` family limit unless planning decides otherwise.
- The output is an advisory recommendation for the caller to act on — `Decide` does not
  execute the decision, and selecting one option here is its job (distinct from `unstick`,
  which commits to a next *step* without an option set).
