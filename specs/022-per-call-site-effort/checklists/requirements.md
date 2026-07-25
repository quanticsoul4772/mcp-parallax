# Specification Quality Checklist: Per-Call-Site Reasoning Effort

**Purpose**: Validate specification completeness and quality
**Created**: 2026-07-25
**Feature**: [spec.md](../spec.md)

## Content Quality

- [X] No implementation details (languages, frameworks, APIs)
- [X] Focused on user value and business needs
- [X] Written for non-technical stakeholders
- [X] All mandatory sections completed

## Requirement Completeness

- [X] No [NEEDS CLARIFICATION] markers remain
- [X] Requirements are testable and unambiguous
- [X] Success criteria are measurable
- [X] Success criteria are technology-agnostic
- [X] All acceptance scenarios are defined
- [X] Edge cases are identified
- [X] Scope is clearly bounded
- [X] Dependencies and assumptions identified

## Feature Readiness

- [X] All functional requirements have clear acceptance criteria
- [X] User scenarios cover primary flows
- [X] Feature meets measurable outcomes defined in Success Criteria
- [X] No implementation details leak into specification

## Notes

**This checklist validates a retrofitted spec.** The implementation preceded it,
so "the spec is complete" here means it accurately describes finished code, not
that it constrained what would be built. That is a materially weaker claim and
the spec's *Process deviation* section says so.

One item deserves scrutiny rather than a tick: *"requirements are testable"*
passes trivially when the tests already exist. Every FR here maps to a test that
was written before this document, so the direction of evidence is reversed — the
requirements were read off the tests rather than the tests written to the
requirements. FR-003 and FR-006 are the two where that matters least, because
both are asserted on the wire and in the pool respectively, and both were
verified to fail when the behaviour is removed.

Deliberately **not** specified: which effort level suits which call site. The
feature ships the control; choosing levels is an operator decision that wants
measurement, and shipping a recommended default would be scope the spec does not
ask for and evidence nobody has yet.
