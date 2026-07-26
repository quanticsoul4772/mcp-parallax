# Specification Quality Checklist: Per-Call Reasoning Effort

**Purpose**: Validate specification completeness and quality before proceeding to planning

**Created**: 2026-07-25

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

All items pass. The three open questions were answered rather than guessed:

- **Scope**: all four candidate settings, not effort alone. One of the four
  (`MEMORY_RECALL_LIMIT`) turned out to be already caller-reachable via
  `recall`'s existing `limit`, so it becomes a confirming test and a corpus
  citation rather than implementation work.
- **Tool surface**: effort goes on the seven correctives only.
- **Rejection**: no boundary refusal; 027's enriched provider error is the
  surface, consistent with 027's rejection of a capability table.

Two residuals are recorded in Assumptions rather than resolved, because both
were raised before the scope was chosen and the choice was made with them in
view: pass count narrowing the basis for confidence (FR-013 answers it by
reporting the count used), and per-call concurrency spending operator egress
(FR-015 answers it by permitting only lowering).

The client-pooling decision in "Plan-Level Decision" is deliberately **not** a
[NEEDS CLARIFICATION] marker. It is an implementation choice, not a
specification gap, and belongs to `/speckit-plan`. It is recorded in the spec
only so the plan cannot gloss it.
