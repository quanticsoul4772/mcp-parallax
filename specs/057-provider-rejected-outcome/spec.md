# Feature Specification: Provider rejections recorded as their own outcome

**Feature Branch**: `057-provider-rejected-outcome`

**Created**: 2026-11-20

**Status**: Draft

**Input**: User description: "Record provider HTTP rejections as their own outcome. Today every provider HTTP error other than 429 and 5xx (a billing/credit-balance 400, a 401 bad key, a 403, a 404 unknown model, a 400 rejected request) becomes a client error recorded as validation_failure, the same outcome as malformed model output, which distorts the failure counts used to size fixes (a credit-balance error was recorded as a decide validation failure). Add one outcome, provider_rejected, carrying the HTTP status, for both the Anthropic and OpenAI-compatible clients; update the published outcome taxonomy, the record contract and the telemetry mapping deliberately; malformed-output failures keep validation_failure; 429/5xx retry behaviour is unchanged. Includes a dated design-doc amendment."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Failure counts separate provider rejections from malformed output (Priority: P1)

An operator counts failures by outcome to decide what to fix. Today, when the model provider refuses a request outright (the account is out of credit, the key is wrong, the key lacks access, the model name is unknown, or the request itself is rejected), the call is recorded as `validation_failure`, the same outcome as a model answer that came back malformed. After this change such a call is recorded as `provider_rejected`, and the recorded error text still names the HTTP status and the provider's message.

**Why this priority**: It is the defect. A credit-balance error was recorded as a `decide` validation failure, and validation-failure counts are what sized the fix for decide's malformed answers.

**Independent Test**: Against a test double of the model provider, return each of the five rejections below and confirm the call's recorded outcome is `provider_rejected` and its error text contains the status; return a malformed answer and confirm it is still `validation_failure`.

**Acceptance Scenarios**:

1. **Given** the provider answers HTTP 400 with a credit-balance message, **When** a tool call is made, **Then** the record's outcome is `provider_rejected` and its error text contains `400` and the provider's message.
2. **Given** the provider answers HTTP 401, 403 or 404, **When** a tool call is made, **Then** the record's outcome is `provider_rejected` and its error text contains that status.
3. **Given** the provider answers HTTP 400 rejecting the request, **When** a tool call is made, **Then** the record's outcome is `provider_rejected`.
4. **Given** the provider returns a successful response whose content breaks the output contract, **When** a tool call is made, **Then** the record's outcome is still `validation_failure`.

---

### User Story 2 - Both model backends record rejections the same way (Priority: P2)

An operator running the OpenAI-compatible backend gets the same outcome for the same kind of rejection as one running the default backend.

**Why this priority**: The same mislabel exists on both backends; fixing one would leave counts inconsistent between deployments.

**Independent Test**: Repeat User Story 1's rejections against the OpenAI-compatible backend's test double; confirm the same outcome. Confirm that a rejection of a structured-output strategy still moves to the next strategy rather than ending the call.

**Acceptance Scenarios**:

1. **Given** the OpenAI-compatible backend receives a terminal HTTP 4xx (other than 429), **When** a tool call is made, **Then** the record's outcome is `provider_rejected`.
2. **Given** the OpenAI-compatible backend receives a 400 that rejects the structured-output strategy it tried, **When** a tool call is made, **Then** the next strategy is tried, as before.

---

### User Story 3 - The published surfaces state the new outcome (Priority: P3)

Anyone reading the invocation-record contract, the telemetry contract or the design documents finds `provider_rejected` listed with what it means.

**Why this priority**: The outcome list is a published surface; a value the code emits but the contracts omit is the drift the project's contract checks exist to prevent.

**Independent Test**: A test compares the outcome values the code can record with the values each published contract lists; they must be equal.

**Acceptance Scenarios**:

1. **Given** the record contract and the telemetry contract, **When** the outcome list in code changes, **Then** a test fails until both contracts list the same values.
2. **Given** the design documents, **When** a reader looks up the outcome taxonomy, **Then** a dated amendment records `provider_rejected` and why it was added.

### Edge Cases

- HTTP 429 and 5xx keep their current behaviour: they are retried, and a call that exhausts its retries is still `retries_exhausted`.
- A rejection of the reasoning-effort setting keeps its remedy text (the model, the level, and how to remove it) in the error message; only its outcome changes.
- Transport-level failures (no HTTP response at all) keep their current outcome.
- The embedding and search providers keep their own outcomes (`embedding_provider`, `search_provider`).
- Records written before this change are not rewritten.
- A rejection is not billed by the provider; the recorded token counts for it stay zero, as today.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: A model-provider HTTP response that is unsuccessful and is neither 429 nor a server error MUST be recorded with the outcome `provider_rejected`, on both model backends.
- **FR-002**: The recorded error text for such a call MUST contain the HTTP status and the provider's message, as it does today; no new record field is added.
- **FR-003**: A successful provider response whose content breaks the output contract MUST keep the outcome `validation_failure`.
- **FR-004**: Retry behaviour for HTTP 429, server errors and transport errors MUST be unchanged.
- **FR-005**: On the OpenAI-compatible backend, a 400 that rejects the structured-output strategy MUST still move to the next strategy rather than end the call.
- **FR-006**: The invocation-record contract MUST list `provider_rejected` among the outcome values.
- **FR-007**: The telemetry contract MUST list `provider_rejected` among the values of the outcome attribute, and exported telemetry MUST carry it for such calls.
- **FR-008**: The design documents MUST carry a dated 2026-11-20 amendment adding `provider_rejected` to the outcome taxonomy, with the reason.
- **FR-009**: The CHANGELOG MUST record the new outcome under `[Unreleased]`.

### Key Entities

- **Outcome**: the single class recorded for each invocation; gains one value, `provider_rejected`, meaning the model provider refused the request with an HTTP 4xx other than 429.
- **Invocation record**: one per tool call; unchanged in shape, its outcome field may now hold `provider_rejected`.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Each of the five example rejections (credit-balance 400, 401, 403, 404, rejected-request 400) is recorded as `provider_rejected` on both backends in tests: 10 of 10 cases.
- **SC-002**: Every existing test of malformed model output still records `validation_failure`: zero changes to those expectations.
- **SC-003**: The set of outcome values the code can record equals the set listed in the record contract and the set listed in the telemetry contract, checked by a test.
- **SC-004**: After the change, no failure recorded as `validation_failure` comes from a provider HTTP 4xx response.

## Assumptions

- No new record column: the status already appears in the recorded error text, and the outcome alone is enough to separate the counts.
- Transport-level errors are out of scope; their classification is a separate question.
- Past records stay as written; a reader correcting old counts can match the error text, which names the status.
