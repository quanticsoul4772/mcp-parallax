# Feature Specification: decide per-option fields

**Feature Branch**: `056-decide-per-option-fields`

**Created**: 2026-11-20

**Status**: Draft

**Input**: User description: "decide per-option fields. decide's single pass returns option scores and rationales as parallel arrays that the structured-output grammar cannot length-constrain, so the model sometimes returns lists that don't match the number of options (28 validation failures since June, e.g. '3 options but 3 scores / 4 rationales'), failing the call after a paid retry. Replace the parallel arrays with a per-call flat, closed schema holding one required score property and one required rationale property per option (option_1_score, option_1_rationale, ...), so a mismatch cannot be produced. The schema varies with the option count instead of being fixed at startup; the flat-schema gate still applies; scores keep their 0-100 validation; the result, confidence and records callers see are unchanged. Includes a dated amendment to 013's design and a live check."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A decide call never fails because of a list-length mismatch (Priority: P1)

A calling model asks `decide` to score N options. Today the model behind `decide` sometimes returns a list of scores or a list of rationales whose length differs from N; the server rejects it, retries once with the violation, and fails the call if the retry is malformed too. After this change the answer's shape names every option individually, so the answer either has exactly one score and one rationale per option or it does not parse at all.

**Why this priority**: It is the defect. 28 recorded `decide` validation failures since June are of this kind (for example "3 options but 3 scores / 4 rationales", "4 options but 4 scores / 0 rationales", "5 options but 10 scores"), each one a failed call the caller paid for.

**Independent Test**: Run `decide` against a test double that returns a well-formed answer for N options, for several N; confirm every call produces one assessment per option in input order. Confirm the request's output format contains exactly one score field and one rationale field per option, all required, and no list fields for scores or rationales.

**Acceptance Scenarios**:

1. **Given** a decision with 3 options, **When** `decide` builds its request, **Then** the requested output format has required fields for option 1, 2 and 3 scores and rationales and no others for per-option data.
2. **Given** a decision with 5 options, **When** the model's answer fills every field, **Then** the result lists 5 assessments in input order with the same scores, ranking, runner-up, confidence and records as the current behaviour would produce for the same scores.
3. **Given** any option count, **When** the requested output format is checked by the flat-schema gate, **Then** it passes: flat, closed, no nested objects or arrays of objects.

---

### User Story 2 - Callers see no change other than fewer failures (Priority: P2)

A calling model reads `decide`'s result: the recommended option, runner-up, why it lost, methodology, deciding factors, per-option assessments and confidence. None of these fields change shape or meaning.

**Why this priority**: The fix must not shift work or risk onto callers or break stored records.

**Independent Test**: Existing `decide` result and record tests pass unchanged against the new request shape.

**Acceptance Scenarios**:

1. **Given** the same scores as today, **When** the result is assembled, **Then** recommended, runner-up, runner-up reason, confidence and assessments are identical to today's.
2. **Given** a `decide` call, **When** it is recorded, **Then** the invocation record has the same fields and meaning as today.

---

### User Story 3 - The design record explains the change (Priority: P3)

A maintainer reading 013's design documents learns why per-option data moved from parallel lists to per-option fields, and what was measured.

**Why this priority**: 013's research explicitly chose parallel arrays because arrays of objects fail the flat-schema gate; reversing that choice without a dated amendment would leave the design record wrong.

**Independent Test**: The 013 design documents carry a dated amendment naming the failure evidence and the new shape.

**Acceptance Scenarios**:

1. **Given** the 013 research and data-model documents, **When** read after this change, **Then** each carries a dated 2026-11 amendment describing the per-option fields and why.

### Edge Cases

- The minimum, 2 options, produces a valid output format with exactly two score and two rationale fields.
- More than 10 options is rejected before any model call, with an error naming the limit (FR-008); exactly 10 is accepted.
- A score outside 0–100 is still rejected by the existing validator and still feeds the single retry, as today.
- An empty or whitespace-only rationale is handled exactly as today.
- Option labels that repeat or contain unusual characters do not affect field names, because fields are named by position, not by label.
- Deciding factors stay a single list (their count is not tied to the options).
- The OpenAI-compatible backend receives the same per-call output format as the Anthropic backend.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `decide` MUST request an output format, built for each call, that contains exactly one required score field and one required rationale field for each option, named by the option's position (option 1, 2, ... N).
- **FR-002**: The per-call output format MUST NOT contain list fields for scores or rationales.
- **FR-003**: The per-call output format MUST pass the existing flat-schema gate (flat, closed, no nested objects, no arrays of objects) for every option count `decide` accepts.
- **FR-004**: Scores MUST keep their 0–100 range check, and a score outside it MUST keep feeding the existing single retry with the violation.
- **FR-005**: The result returned to callers (recommended, runner-up, runner-up reason, confidence, methodology, deciding factors, assessments) and the invocation record MUST be unchanged in shape and meaning; for identical scores they MUST be identical.
- **FR-006**: The methodology field and the deciding-factors list MUST keep their current form.
- **FR-007**: The 013 design documents MUST carry a dated amendment recording the parallel-array failures, the new per-option fields, and why the flat-schema gate still holds.
- **FR-008**: `decide` MUST accept at most 10 options; a call with more MUST be rejected before any model call with an error naming the limit (decided 2026-11-20: covers the counts seen in recorded inputs, 2 to 6, with room, and keeps the output format to at most 20 per-option fields).
- **FR-009**: A live check MUST confirm, on the real model, that `decide` calls with several option counts return one assessment per option.

### Key Entities

- **Per-call output format**: the structure the model is constrained to for one `decide` call; it has a methodology, a list of deciding factors, and one score and one rationale per option position.
- **Option assessment**: the server-assembled pairing of an option label with its score and rationale, in input order; unchanged from today.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Zero `decide` calls fail with a list-length mismatch after the change; the "arity mismatch" failure no longer exists as a possible outcome.
- **SC-002**: For any set of scores, the result callers receive is identical to what the current version returns for the same scores.
- **SC-003**: Every option count `decide` accepts produces an output format that passes the flat-schema gate.
- **SC-004**: A live check on at least three different option counts returns one assessment per option in each case.

## Assumptions

- The model provider accepts a different output format per request; formats are compiled once per distinct shape and cached, so only the first call for a given option count pays a compile delay.
- Naming fields by position (not by option label) avoids problems with repeated or unusual labels.
- The billing error recorded as a `decide` validation failure on 2026-11-17 is a separate defect (outcome classification) and is out of scope here.
- Other modes are not affected; they do not use per-option parallel lists.
