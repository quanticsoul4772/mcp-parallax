# Specification Quality Checklist: Per-Hop Model Routing

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-07-24
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Notes

**Validation run 1 (2026-07-24)** — one item failed: two [NEEDS CLARIFICATION]
markers stood, on routing granularity and on the multi-model record shape. Both
were judged to have no reasonable default — each had two or more defensible answers
with materially different blast radius, and picking silently would have committed
the feature to a contract change the operator has to live with — so both were put
to the operator rather than guessed.

**Validation run 2 (2026-07-24)** — all items pass.

Both questions were answered and folded into the spec:

1. *Routing granularity* → tier default with per-call-site override, resolving
   most-specific-first. Added FR-001a, FR-001b, FR-001c, and FR-006a (reserved
   setting namespace, so a misspelled route is a startup error rather than a silent
   no-op). The corresponding edge case is now resolved rather than open.
2. *Multi-model record shape* → one audit row per invocation carrying a per-model
   usage breakdown, cost summed across models. Added FR-009a and tightened FR-009
   and FR-010. The one-record-per-invocation invariant and the one-span-per-
   invocation shape in `specs/007-observability-layer/contracts/telemetry.md` both
   survive unchanged, which was the deciding factor.

The Open Questions section became Resolved Decisions, recording each choice with
the alternatives rejected and why.

*Content-quality note (passing, recorded for transparency)*: the Assumptions
section states that configuration remains environment-variable based. That is a
mechanism, but it is an inherited project-wide constraint named in the feature
request rather than a design decision taken here, and the Assumptions section is
where such carried constraints belong. No language, framework, crate, type, or
file path appears anywhere in the spec.

**Clarification session (2026-07-24)** — three questions asked and answered, all
integrated; still passing.

The scan found three Partial categories beyond the two already resolved at specify
time. Each was a decision the spec named but did not settle:

1. *Domain & Data Model* — FR-001a said "a small number of named tiers" without
   saying which, or which call sites belong to them. Resolved to two tiers, `bulk`
   (research extraction alone) and `judgment` (the other eleven). FR-001a and the
   Tier entity now state membership.
2. *Edge Cases & Failure Handling* — the spec never said what happens when a routed
   model fails. Resolved to no cross-model fallback, added as FR-015a and FR-015b
   plus an edge case, matching the project's error protocol and the existing
   degradation behavior of each layer.
3. *Observability* — FR-005 required reporting routes "on demand" without naming a
   channel, leaving SC-007 untestable and implying a possible new tool. Resolved to
   a startup line on the diagnostic stream; FR-005 rewritten, FR-005a added to
   forbid a catalog entry, SC-007 rewritten to be checkable.

Two lower-impact candidates were resolved by documented default rather than by
spending a question, per the rule against asking where a reasonable default exists:
SC-001's fixed 30% target became a figure derived from the unrouted baseline's own
token split, and pre-existing invocation records are stated to remain valid as
single-model records with no backfill.

Terminology was normalized in the same pass: the spec now uses "call site"
throughout, with the Call site entity noting that the feature's title and branch
call the same thing a "hop". The verbatim user Input line is unchanged.

No items remain incomplete. The spec is ready for `/speckit-plan`.
