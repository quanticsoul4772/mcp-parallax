# Specification Quality Checklist: grounded_verify Refuses Files Whose Names Mark Them as Secrets

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-11-17
**Feature**: [spec.md](../spec.md)

Every box below is unticked on purpose. They are ticked only by a reviewer of
the specification, not by its author.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs)
- [ ] Focused on user value and business needs
- [ ] Written for non-technical stakeholders
- [ ] All mandatory sections completed

## Requirement Completeness

- [ ] No [NEEDS CLARIFICATION] markers remain
- [ ] Requirements are testable and unambiguous
- [ ] Success criteria are measurable
- [ ] Success criteria are technology-agnostic (no implementation details)
- [ ] All acceptance scenarios are defined
- [ ] Edge cases are identified
- [ ] Scope is clearly bounded
- [ ] Dependencies and assumptions identified

## Feature Readiness

- [ ] All functional requirements have clear acceptance criteria
- [ ] User scenarios cover primary flows
- [ ] Feature meets measurable outcomes defined in Success Criteria
- [ ] No implementation details leak into specification

## Notes

- Second version. After two reviews of the first, the owner split the work: this specification is the secret-name refusal only; confining each session to its own project is deferred to its own specification.
- The owner chose the extended list of refused names, and that a glob leaves refused files out and lists them instead of failing the whole call.
- The specification names existing documents it must amend (FR-009). That is scope: each states something this feature makes untrue.
