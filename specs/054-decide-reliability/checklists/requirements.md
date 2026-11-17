# Specification Quality Checklist: Decide Stops Claiming Calibrated Confidence

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-11-17
**Feature**: [spec.md](../spec.md)

Every box below is unticked on purpose. The first version of this checklist
was ticked by the specification's author and two independent reviews then
found the "testable and unambiguous" and "measurable" items were not met. The
boxes are ticked only by a reviewer of the current version, not by its author.

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

- Third version. After the first review the owner narrowed the scope to one pass only. After the second review showed the author had overstated the evidence, the owner chose to change words only and leave the confidence number unchanged; and to commit scores and scripts without input texts.
- The specification names existing documents it must change (FR-006 to FR-009). That is scope: the feature exists because those documents state something the measurement contradicts.
