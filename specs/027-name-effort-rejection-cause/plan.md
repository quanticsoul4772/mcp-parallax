# Implementation Plan: Name the Cause When the Provider Rejects an Effort Level

**Branch**: `027-name-effort-rejection-cause` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

## Summary

When the provider rejects a request for the effort parameter, append a diagnosis
naming the model and the level sent. The fact comes from the provider at the
moment it is true, so nothing goes stale and no configuration is refused.

One arm of one match. The corpus correction is larger than the code.

## Technical Context

**Language/Version**: Rust, MSRV 1.94. **Dependencies**: unchanged.
**Storage**: N/A. **Testing**: `wiremock`, beside the existing effort-on-the-wire
tests in `src/client/anthropic.rs`. **Performance**: N/A — a string format on an
already-failing path.

**Constraints**: The enrichment is a *guess* about why a request was rejected.
Principle III forbids hiding failures; this hides none — it appends to the
provider's own text and never replaces it. The guard must be narrow enough that
a rejection with another cause is untouched, because a confident wrong diagnosis
is worse than a bare message.

## Constitution Check

*GATE: evaluated before implementation.*

| Principle | Verdict | Basis |
|---|---|---|
| **I. Design-Corpus Fidelity** | PASS | Implements 022 `spec.md:186` as written rather than reversing it. Five corpus locations corrected in this change, enumerated below — including a false statement already released, which is why this section is not a formality here. |
| **II. Constrained-Output Contract** | PASS | No mode schema touched. No model call added. |
| **III. Compiler-Enforced Discipline** | PASS | No new `unwrap`/`expect`. The provider's message is preserved, never swallowed or replaced. |
| **IV. Seams, Composition, Tests** | PASS | The change sits at the existing `ModelClient` implementation and is tested through `wiremock` — no network, no credentials. |
| **V. Deterministic Over Probabilistic** | PASS | A string match on a provider response. No judge. |
| **VI. Capabilities Off By Default** | PASS | A deployment setting no effort variable has `effort: None`, so the guard cannot fire. Byte-identical behaviour. |
| **VII. Simplicity and Scope Discipline** | PASS | No new module, no new type, no new configuration. The rejected alternative added an enum, a ten-row table, a validator, a report column, and a startup failure mode. |

### What this deliberately does not do

Prevent the failure. It cannot, without either a compiled table (rejected — see
spec Context) or a startup probe (out of scope). Post-023 the cost is one run's
search and fetch spend, and the error now names its own cause.

## Structure

```text
src/client/anthropic.rs   # the 400 arm at :189-192, plus three tests
docs/design/SDK_LANDSCAPE.md          # :283-285 false claim
specs/018-model-routing/research.md   # :168-170 same false claim
specs/022-per-call-site-effort/spec.md # :128, :186 — record what the surfaced error now says
CHANGELOG.md              # [Unreleased] Fixed; corrects the released :61 claim
CLAUDE.md, README.md      # both namespaces, undocumented since 018
```

**Structure Decision**: The knowledge stays at the client boundary, where the
provider's response already is. `routing.rs` is untouched — its module doc's
claim to hold no provider knowledge survives this change, which the rejected
alternative would have had to rewrite.

## Phase 2 preview

1. The guard and the message, with the three negative tests first.
2. Prove the enrichment test fails without it.
3. Corpus: the two false statements, the 022 amendment, the changelog correction.
4. Document both namespaces in `CLAUDE.md` and `README.md`; fix the stale
   `REQUEST_TIMEOUT_MS` default while there (`src/config.rs:92` says 30000,
   `:161` reads 120_000, both docs repeat the stale value).

## Complexity Tracking

No violations.
