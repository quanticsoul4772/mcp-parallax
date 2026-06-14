# Research: Verification Reliability

Phase 0 decisions. The clarification settled the user-facing behavior (abstain +
route, `inconclusive` verdict); these resolve the mechanism.

## D1 — The lens set for `verify`

**Decision**: a small fixed array of named critical lenses, each a one-paragraph
directive injected into the prompt. Initial set (5):

- **literal** — read the claim at face value; does the plain reading hold?
- **counterexample** — actively seek a case or edge condition that breaks it.
- **definitional** — scrutinize the key terms; is the claim true only under a loose
  definition?
- **evidential** — what would have to be true for this claim; is that established or
  assumed?
- **scope** — is the claim overgeneralized (true sometimes, asserted always)?

**Rationale**: these are *critical perspectives*, not stance — they keep the
verifier blind to who made the claim while making the passes genuinely independent,
which is the whole point (the agreement ratio only means something if the passes
could disagree). Mirrors the corpus's "diverse lenses, not N identical critics"
(`NEW_SERVER_DESIGN.md` §Designing real independence) and the `research` layer's
existing multi-lens verify.

**Alternatives**: temperature/sampling variation only (the status quo — rejected, it
is exactly what fails); model-generated lenses per claim (rejected — non-determinism
and an extra hop for no clear gain).

## D2 — Lens ↔ `k` assignment

**Decision**: pass *i* gets `LENSES[i % LENSES.len()]`. With the default `k=3`, the
first three lenses run; if an operator sets `k` above the lens count, lenses cycle
(deterministic, stable order). `k=1` gets the first lens (degenerate but valid).

**Rationale**: deterministic and independent of `k`; no config surface. The
aggregation quorum/confidence math is unchanged and already handles any `k`.

## D3 — Stance-blindness is preserved

**Decision**: the lens is injected into the *critical-instruction* portion of the
prompt; the claim and optional context remain the only dynamic inputs about *what*
is judged. The prompt still has no slot for the caller's stance, identity, or
confidence.

**Rationale**: FR-003 / US1-AS4 — diversification must not reintroduce the bias the
tool exists to avoid. The lens changes *how* a pass scrutinizes, not *what it knows
about the asker*.

## D4 — Computable-claim detection: a per-pass self-report, NOT the `check` classifier

**Decision**: detect a computable-over-source claim via a **per-pass boolean**
(`needs_computation`) the model sets when the claim's truth hinges on an exact
computation of the read text (a precise count, a numeric measure) it cannot perform
reliably by reading. The server returns `inconclusive` (route to `check`) when a
majority of completed passes set it.

**Rationale — plan-discovered correction.** The clarification proposed *reusing the
`check` layer's checkability classifier*. On inspection that does **not** fit:
`check`'s classifier (`src/deterministic/translate.rs`) is **decline-biased** and
deems a claim checkable only if "its truth is computable by evalexpr/Z3 **with given
values**." A claim like "`server.rs` has > 1000 lines" requires first *counting* the
lines — not expressible to evalexpr/Z3 as stated — so `check` would classify it
**not checkable** and miss exactly the class US2 targets. The faithful signal is the
one the dogfooding run already surfaced: the passes themselves flagged "an exact line
count would be needed" (in `missing_evidence`). Promoting that to an explicit boolean
is the lightest reliable detector.

**Consistency with the clarification**: the inconclusive verdict stays
server-assembled and the passes still emit `supported`/`refuted` — the Q2 intent
holds. The only refinement is *one boolean* added to the grounded pass schema (the
clarification's "per-pass schema unchanged" assumed `check` would do the detecting).

**Alternatives**: a separate classifier model-call before the passes (rejected —
extra hop, and it would have to re-derive what the passes already judge); pure
`missing_evidence` heuristics with no explicit flag (rejected — "decisive" vs
incidental missing evidence is exactly what the boolean disambiguates).

## D5 — Where `inconclusive` lives

**Decision**: `grounded_verify`'s **output** verdict becomes a 3-value
`GroundedVerdictKind { Supported, Refuted, Inconclusive }`, server-assembled. The
**per-pass** `VerdictKind` (shared with `verify`) stays `{ Supported, Refuted }`, so
`verify`'s output is untouched (FR-009).

**Rationale**: a non-decision is a first-class outcome (the bug was a wrong *binary*
verdict). Keeping it out of the shared per-pass enum is what lets `verify` stay
unchanged. The result carries a short `reason` (e.g. "computable property — route to
`check`") so the caller knows what to do next.

**Mapping (server)**: after aggregation — if a majority of passes set
`needs_computation` → `Inconclusive` (reason: route to `check`); else the majority
`supported`/`refuted` as today. `needs_computation` is the **only** abstain trigger; a
non-empty aggregated `missing_evidence` stays the advisory completeness signal (008)
and does **not** force `Inconclusive` (no over-abstention — analyze remediation M1).

## D6 — Graduated confidence is a live-model property (testing)

**Decision**: SC-001 (real contestable claims yield non-extreme confidence) is
validated by a **live** acceptance battery, not offline. Offline tests cover the
mechanism: distinct lens prompts per pass (assert the *k* prompts differ), and
`aggregate_core` returning ≈0.67 / 0.5 / sub-quorum on constructed vote vectors
(FR-004/SC-005).

**Rationale**: a wiremock returns canned responses and cannot scatter across lenses,
so it cannot demonstrate graduated confidence. The mechanism is offline-testable; the
emergent property needs the real model. Noted so `/speckit-implement` doesn't chase a
wiremock proof of SC-001.
