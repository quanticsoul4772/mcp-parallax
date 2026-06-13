# Feature Specification: Verification Reliability (verify lens-diversity + grounded_verify compute-or-abstain)

**Feature Branch**: `010-verification-reliability`

**Created**: 2026-06-13

**Status**: Draft

**Input**: User description: "Two reliability findings surfaced by dogfooding Parallax against its own codebase. (1) `verify` runs k *identical* passes, so its agreement-derived confidence is near-binary — across 8 live calls, including 5 deliberately borderline claims, it returned 1.0 every time; the graduated-confidence signal it advertises does not materialize. (2) `grounded_verify` routed a *computable* claim (a file's line count) to judgment passes, which estimated wrong and returned a confident refutation (confidence 1.0) of a true claim — while its own missing-evidence field said an exact count was needed. Fix both: lens-diversify verify's passes so confidence is a real signal; make grounded_verify settle computable sub-claims deterministically or abstain, and bound its confidence by its own missing-evidence signal."

## User Scenarios & Testing *(mandatory)*

Parallax exists to catch the calling model's confidently-wrong reasoning. Dogfooding
it against its own repository surfaced two places where the verification tools fall
short of that mandate — one a weak signal, one a confidently-wrong verdict. Both are
about *trustworthiness of the verdict*, not the plumbing around it.

Neither is a correctness regression in the common case: `verify` refutes false claims
with named errors, declines to over-refute true ones, and stays blind to asserted
authority (all live-verified). The gaps are narrower and specific, and each is
independently fixable and testable.

### User Story 1 - `verify` confidence is a meaningful signal (Priority: P1)

A caller uses `verify` when being confidently wrong is costly, and reads the returned
`confidence` to decide whether to trust the verdict or escalate. Today `verify` builds
one prompt and runs it `k` times unchanged (`src/modes/verify.rs` — the same `&prompt`
is handed to every pass), so the passes differ only by sampling noise and converge on
almost every input. Confidence is therefore effectively binary: 8 live calls, including
5 borderline claims, all returned `1.0`. The caller cannot distinguish "all critics
strongly agree" from "the model happened not to vary" — the escalation signal is inert.

This story makes the `k` passes apply **distinct critical lenses** (e.g. literal
reading, edge-case/counterexample seeking, definitional scrutiny) rather than one
shared prompt, so genuinely contestable claims scatter across lenses and the
agreement-ratio confidence spans the range it was designed to. The aggregation math
(majority verdict, ties→refuted, dedup from the majority side, confidence = majority /
completed, quorum `⌈k/2⌉`) is already correct and is unchanged — only the *inputs*
diversify.

**Why this priority**: This is the headline value of `verify` — a calibrated
"how sure" — and it currently does not work as documented. It directly contradicts the
project's own design principle (`docs/design/NEW_SERVER_DESIGN.md` §"Designing real
independence": *use diverse lenses, not N identical critics*), which the `research`
layer already honors and `verify` does not.

**Independent Test**: Run `verify` over a fixed set of contestable claims and confirm
at least some return a confidence strictly between 0 and 1 (a split), while a control
set of clear-error and clearly-true claims still returns the correct verdict at high
confidence. Fully testable through the tool's public output; delivers a usable
escalation signal on its own.

**Acceptance Scenarios**:

1. **Given** a claim on which independent critical lenses legitimately disagree,
   **When** `verify` runs at `k=3`, **Then** the returned `confidence` can be a
   non-extreme value (e.g. ≈0.67) reflecting the disagreement, not a near-constant 1.0.
2. **Given** a claim with a clear, nameable error, **When** `verify` runs, **Then** the
   lenses still converge to `refuted` with the concrete error named — no correctness
   regression.
3. **Given** a clearly-true claim, **When** `verify` runs, **Then** it returns
   `supported` with no manufactured findings — diversification does not induce
   over-refutation.
4. **Given** a context that vouches for a false claim (asserted authority), **When**
   `verify` runs, **Then** it still refutes on the merits — stance/authority blindness
   is preserved.
5. **Given** the aggregation logic, **When** exercised with constructed pass-vote
   vectors (2:1, an even-`k` 2:2 tie, and a sub-quorum case), **Then** it yields the
   documented verdict and confidence for each — covered by deterministic tests, not
   organic disagreement.

---

### User Story 2 - `grounded_verify` does not confidently judge a computable property (Priority: P2)

A caller asks `grounded_verify` to check a claim against named source. When the claim's
truth is a **computable property of the read text** — a line count, a count of matches,
a presence/absence check, a numeric comparison — the tool currently still routes it to
judgment passes, which read the bytes and *estimate*. In the dogfooding case the claim
"`src/server.rs` is over 1000 lines" (the file is 1224 lines) came back **refuted at
confidence 1.0** because the passes estimated ~850; the tool's own `missing_evidence`
field simultaneously said an exact line count was needed. A confident verdict against
the tool's own admission of missing decisive evidence is the exact failure mode
Parallax is built to prevent.

This story makes `grounded_verify` (a) settle a computable sub-claim by deterministic
computation over the exact bytes it already read — or return a "not judgment-verifiable;
route to a deterministic check" signal — instead of emitting a judgment verdict on a
property it did not compute; and (b) bound its reported confidence by its own
missing-evidence signal, so a run that flags the decisive evidence as unavailable can
never report maximal confidence.

**Why this priority**: P2 because it is narrower in blast radius than US1 (it bites only
on computable claims, a subset of `grounded_verify` usage) but it is the more dangerous
*kind* of failure — a confidently-wrong verdict, not merely a coarse signal. The
project already has the right home for computable claims: the deterministic `check`
layer. The fix applies that existing principle inside `grounded_verify`.

**Independent Test**: Issue the reproduction claim (`server.rs` > 1000 lines, file is
1224) and confirm the verdict is correct (or an explicit abstention) and not a confident
refutation; and confirm that any run whose missing-evidence list names decisive missing
evidence does not report confidence 1.0. Testable through tool output alone.

**Acceptance Scenarios**:

1. **Given** a claim whose truth is a computable property of the named source (e.g.
   "file X has more than N lines"), **When** `grounded_verify` runs, **Then** the
   property is decided by deterministic computation over the read bytes, or the tool
   returns an explicit not-judgment-verifiable signal — **never** a confident judgment
   verdict it did not compute.
2. **Given** the reproduction case — claim "`src/server.rs` exceeds 1000 lines", file is
   1224 lines — **When** `grounded_verify` runs, **Then** it returns `supported` (or an
   explicit abstention), and **does not** return `refuted` at confidence 1.0.
3. **Given** a run whose passes flag the decisive evidence as missing
   (`missing_evidence` non-empty and decisive), **When** the verdict is aggregated,
   **Then** the reported confidence is bounded below maximal — a confident verdict is
   not emitted over self-reported missing decisive evidence.
4. **Given** a genuine judgment claim about source content (not computable — "does this
   function handle the empty-input case"), **When** `grounded_verify` runs, **Then** it
   uses the stance-blind passes as today — no regression for the judgment path.

---

### Edge Cases

- **Compound claims** (part computable, part judgment): how is the claim split, and does
  the verdict combine a deterministic sub-result with a judged sub-result coherently?
  (`/speckit-plan` decides the decomposition rule.)
- **Spurious single-lens disagreement** in `verify`: one diversified lens hallucinates an
  error on a clearly-true claim. The quorum/majority rule must prevent one bad lens from
  flipping a verdict, and dedup must not surface its finding from the minority side.
- **A computable claim grounded_verify cannot compute cheaply** (e.g. requires parsing it
  does not do): it must abstain explicitly, not fall back to a confident guess.
- **Lens count vs `k` mismatch**: if the lens set is smaller or larger than
  `VERIFY_ENSEMBLE_K`, how are lenses assigned/cycled? (`/speckit-plan`.)

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `verify` MUST execute its `k` passes under **distinct critical lenses**,
  not a single shared prompt replicated `k` times.
- **FR-002**: `verify` confidence MUST be capable of producing graduated, non-extreme
  values that reflect genuine cross-lens disagreement on contestable claims.
- **FR-003**: `verify` MUST preserve its current correct behaviors with no regression:
  named-error refutation of false claims, no over-refutation of true claims, and
  stance/authority blindness.
- **FR-004**: The `verify` aggregation paths — majority, tie→refuted, dedup from the
  majority side, confidence = majority/completed, sub-quorum → dominant failure — MUST
  have deterministic test coverage driven by constructed vote vectors, independent of
  organic pass disagreement.
- **FR-005**: `grounded_verify` MUST settle a **computable** sub-claim (counts,
  presence/absence, numeric comparison over the read text) by deterministic computation
  over the bytes it read, OR return an explicit not-judgment-verifiable signal — it MUST
  NOT emit a confident judgment verdict on a property it did not compute.
- **FR-006**: `grounded_verify`'s reported confidence MUST be bounded by its own
  missing-evidence signal: a run whose `missing_evidence` names decisive missing
  evidence MUST NOT report maximal confidence.
- **FR-007**: `grounded_verify` MUST preserve the stance-blind judgment path unchanged
  for genuine judgment claims about source content.
- **FR-008**: Both fixes MUST ship with acceptance tests that reproduce the two
  dogfooding cases (the all-1.0 borderline battery for US1; the `server.rs` line-count
  miss for US2).

### Key Entities

- **Verification pass**: one independent evaluation of the claim. Today carries a verdict
  and findings; gains an associated **lens** (the critical perspective it applies).
- **Lens**: a named critical perspective assigned to a pass (e.g. literal, edge-case,
  definitional). The set and assignment rule are a planning decision.
- **Computable sub-claim**: the portion of a grounded claim decidable by deterministic
  computation over the read source rather than by judgment.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a fixed battery of contestable claims, `verify` returns a confidence
  strictly between 0 and 1 on a meaningful fraction of them — versus the current
  **0 of 8** observed — demonstrating the signal is graduated.
- **SC-002**: `verify` retains **100%** of its current correct verdicts on the
  regression set (named-error refutation, no over-refutation, authority-blindness).
- **SC-003**: The reproduction claim "`src/server.rs` exceeds 1000 lines" returns a
  correct or explicitly-abstaining result — **never** a confident refutation — for the
  actual 1224-line file.
- **SC-004**: `grounded_verify` reports maximal confidence on **0%** of runs whose own
  missing-evidence list names decisive missing evidence.
- **SC-005**: The `verify` split / tie / sub-quorum aggregation branches each have direct
  passing unit coverage.

## Assumptions

- The model client and the native-structured-output contract are unchanged; this feature
  changes pass *orchestration* and *verdict assembly*, not the provider API.
- The concrete lens set, the lens↔`k` assignment rule, and the computable-claim
  decomposition/computation mechanism are **`/speckit-plan` decisions** (mirroring how
  009 deferred engine/crate choice to planning).
- `VERIFY_ENSEMBLE_K` semantics (default 3, the quorum rule) are unchanged; only what
  each pass does differs.
- `grounded_verify`'s root-confinement, locator model (008/009), and evidence manifest
  are unchanged.
- The deterministic `check` layer (005) is the reference model for how a computable claim
  should be settled; US2 applies that principle inside `grounded_verify` rather than
  re-implementing a solver.
