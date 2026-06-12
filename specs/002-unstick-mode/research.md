# Phase 0 Research: Unstick Mode

**Date**: 2026-06-12 · **Sources**: `docs/design/NEW_SERVER_DESIGN.md` §2/§4/§8,
`NEXT_REASONING_SERVER.md` (usage data), core-layer implementation. No
NEEDS CLARIFICATION markers; no new external dependencies to research.

## D1 — Single generation pass, no ensemble

- **Decision**: `unstick` runs exactly one `ModelClient::complete` pass
  (registry `ensemble_k = 1`); no aggregation, no agreement confidence.
- **Rationale**: Verify's k=3 exists to protect *judgments* from pushback and
  miscalibration (the spike's finding). Unstick is *generative* — it produces a
  step rather than evaluating a claim — and the corpus positions it as "the
  cheap structured step … the workhorse" (#1 organic tool at 67 uses). Tripling
  its cost would invert its value proposition. FR-007 encodes this.
- **Alternatives considered**: k=3 with "pick the most common step" — steps
  rarely collide textually, so majority aggregation degenerates; a judge pass
  to select the best step re-imports judge bias for no validated gain.

## D2 — Enforcing "exactly one step" across three layers

- **Decision**: split the one-step guarantee by enforceability:
  1. **Schema (grammar-enforced)**: the output type has a single `next_step`
     string field — there is no array for alternatives to land in.
  2. **Code (deterministic)**: post-validation checks reject an empty/blank
     step and a step that is a normalized restatement of a provided attempt
     (case-folded, trimmed, punctuation-insensitive equality) →
     `validation_failure` (FR-003).
  3. **Prompt (calibrated)**: instructs one concrete committed action, no
     option menus, no plans, and not repeating the attempts list — measured by
     the acceptance run (SC-003), not trusted blindly.
- **Rationale**: Constitution V — everything checkable is checked
  deterministically; only genuinely unjudgeable quality (is the step *good*?)
  stays with the model and is sampled by acceptance.
- **Alternatives considered**: an LLM judge pass scoring "is this one step?" —
  rejected: checkable-enough structurally, and a judge adds cost + bias.

## D3 — Registry reuse and the shared invocation wrapper

- **Decision**: register `unstick` through the existing `ModeRegistry`
  (`ensemble_k` field carries 1). Extract the per-invocation guard logic in
  `server.rs` (RecordGuard + ct-select + error mapping) into a private
  `run_recorded` helper used by both tool methods.
- **Rationale**: without the extraction, tool #2 copy-pastes ~30 lines of
  recording/cancellation plumbing — exactly the per-mode triplication the
  design kills (§6.3 "modes are data"). The extraction is behavior-preserving
  for verify; SC-006 (existing tests unchanged) is the regression gate.
- **Alternatives considered**: full generic executor keyed off the registry
  (one dispatch function for all modes) — premature: verify's ensemble run and
  unstick's single pass have different shapes; generalize at mode #3 when the
  pattern is visible. Duplicating the guard — rejected as the thing this
  feature exists to disprove.

## D4 — Input/output field shapes

- **Decision**: input `{ goal: string (required), blocked: string (required),
  tried: string[] (optional) }`; output `{ next_step: string, rationale:
  string, watch_for: string | null }`. All flat, closed, grammar-legal
  (nullable scalar via type union `["string","null"]`, which the grammar
  supports).
- **Rationale**: `goal` + `blocked` are the minimum frame the corrective needs;
  `tried` is what makes loop-breaking checkable (FR-003). `watch_for` is one
  optional pitfall — kept singular for the same anti-menu reason as the step.
- **Alternatives considered**: free-form single `situation` field — loses the
  checkable attempts list; structured multi-step plan output — reintroduces
  the failure mode (plan dumps are motion without commitment).

## Risks

- **Acceptance subjectivity**: "exactly one actionable step" is judged by
  inspection in the acceptance run; the deterministic checks bound the worst
  cases (menus can still be phrased inside one string). Acceptable for the
  cheap corrective; revisit if SC-003 misses.
- **`tried` restatement check is exact-normalized only** — paraphrased repeats
  pass the code check and are caught only by prompt + acceptance. Same
  tradeoff as verify's exact-string finding dedup.
