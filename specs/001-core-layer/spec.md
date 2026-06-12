# Feature Specification: Core Layer — Working Server with First Corrective (Verify)

**Feature Branch**: `001-core-layer`

**Created**: 2026-06-11

**Status**: Draft

**Input**: User description: "Core layer: MCP server foundation — stdio transport, first corrective tool surface with schema-constrained results, Anthropic structured-outputs client behind ModelClient"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Verify a claim and get a structured verdict (Priority: P1)

An assistant (the calling model, via its MCP host such as Claude Code or Claude
Desktop) connects to Parallax, discovers the available tools, and invokes
**Verify** with a claim it wants checked. It receives back a structured verdict —
supported or refuted, with specific named findings and a confidence signal —
produced by an independent pass that did not see the conversation that produced
the claim. The result always matches the tool's declared output structure, so the
caller can consume it mechanically without parsing prose.

**Why this priority**: This is the entire value proposition made real for the
first time — an external, independent corrective the model can call. Every other
layer (watchdog, memory, deterministic) builds on this path existing and being
trustworthy. It is also the path the project's spike already validated.

**Independent Test**: Connect a standard MCP client to the server, list tools,
invoke Verify with a claim containing a known factual error, and confirm a
structurally valid verdict that names the error.

**Acceptance Scenarios**:

1. **Given** a running server and a connected MCP client, **When** the client requests the tool catalog, **Then** it sees the Verify tool with a description and declared input and output structure.
2. **Given** a claim containing a specific factual error, **When** Verify is invoked, **Then** the verdict refutes the claim and names the specific error — not a vague "may be inaccurate".
3. **Given** a sound claim, **When** Verify is invoked, **Then** the verdict supports it without inventing refutations.
4. **Given** any successful Verify invocation, **When** the result is returned, **Then** it validates against the tool's declared output structure — every time, with no free-text fallback.
5. **Given** a claim submitted with the requester's stated stance attached (e.g. "I'm confident that…"), **When** Verify runs, **Then** the verdict is the same as for the identical claim without the stance — the verifier is blind to it.

---

### User Story 2 - Failures are distinct, named, and never silent (Priority: P2)

An operator (or the calling assistant) hits a failure condition — the model
provider refuses the request, the response is truncated, the request times out,
retries are exhausted, or the server was started without its required
configuration. In every case they receive a distinct, descriptive error that
names what actually happened. They never receive a partial result, a
silently-degraded answer, or unparseable output.

**Why this priority**: A corrective tool that can fail ambiguously is worse than
no tool — the calling model would treat garbage as a verdict. Trustworthy error
behavior is what makes the P1 path safe to rely on; it is the difference between
a working demo and a dependable component.

**Independent Test**: Induce each failure class (refusal, truncation, timeout,
missing configuration) and confirm each produces its own clearly distinguishable
error and no partial verdict.

**Acceptance Scenarios**:

1. **Given** a request the model provider refuses, **When** Verify is invoked, **Then** the caller receives an error identifying it as a refusal — not a parse failure, not an empty verdict.
2. **Given** a response truncated before completion, **When** Verify is invoked, **Then** the caller receives an error identifying truncation, and no attempt is made to salvage the partial output.
3. **Given** an unreachable model provider or exhausted retries, **When** Verify is invoked, **Then** the caller receives an error naming the condition and the attempt count.
4. **Given** a server started without required configuration, **When** it starts, **Then** it exits immediately with an error naming the missing item — it does not start in a half-working state.
5. **Given** any failure, **When** the error is surfaced, **Then** nothing is written to the protocol channel except well-formed protocol messages — diagnostics go to the diagnostic stream only.

---

### User Story 3 - Every invocation is observable (Priority: P3)

An operator reviewing server behavior can see, for every tool invocation, a
structured record of what happened: which tool, which model, token usage, cost,
latency, outcome, and the session it belonged to. This exists from the first
release so usage analysis never starts from a blind spot.

**Why this priority**: The project's prior server lost its usage history because
metrics were never persisted, and the design treats observability as designed-in,
not bolted on. It is third only because the first two stories must exist for
there to be anything to observe.

**Independent Test**: Invoke Verify several times (including one failure), then
confirm each invocation produced one structured record with the required fields
and correct outcome classification.

**Acceptance Scenarios**:

1. **Given** a completed Verify invocation, **When** the operator inspects the invocation records, **Then** exactly one record exists for it carrying tool name, model identifier, token counts, cost, latency, outcome, and session identifier.
2. **Given** a failed invocation, **When** the operator inspects the records, **Then** the record classifies the failure by its distinct class (refusal, truncation, timeout, etc.).

---

### Edge Cases

- Empty or whitespace-only claim: rejected as invalid input with a descriptive error before any model call is made.
- Oversized claim (beyond what a single verification pass can hold): rejected with a descriptive error, not silently trimmed.
- Concurrent invocations from one client: each completes independently with its own record; results are never crossed.
- Client disconnects mid-invocation: the server remains healthy and available for the next client; the abandoned invocation is recorded with its outcome.
- Verdict value constraints the generation grammar cannot enforce (e.g. confidence bounds): enforced by local validation before the result is returned; a violating result is an error, not a returned verdict.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The server MUST complete the standard MCP handshake over stdio and advertise its tool catalog to any conforming MCP client.
- **FR-002**: Every tool MUST declare both an input structure and an output structure in its catalog entry, and every successful result MUST be returned as structured content conforming to the declared output structure.
- **FR-003**: The server MUST expose a Verify tool accepting a claim (and optional context), returning a verdict that states whether the claim is supported or refuted, names specific findings (each refutation MUST cite a specific concrete error, per the calibrated-verifier standard), and carries a confidence signal.
- **FR-004**: Verification MUST be performed by an independent model pass that receives only the claim and its provided context — never the requester's stance, conversation history, or identity.
- **FR-005**: Model output MUST be constrained to the declared output structure at generation time, and value constraints that generation-time enforcement cannot express MUST be enforced by local validation before the result is returned. A result failing local validation is an error, never a returned verdict.
- **FR-006**: All tool output structures MUST be flat and closed — no nesting beyond one level of named fields, no undeclared fields accepted.
- **FR-007**: Each failure class — provider refusal, truncated response, timeout, exhausted retries, invalid input, configuration error, validation failure — MUST surface as a distinct, descriptive error. The server MUST NOT return partial results or fall back to interpreting unstructured output.
- **FR-008**: The server MUST emit no diagnostic output on the protocol channel; all logging goes to the diagnostic stream.
- **FR-009**: The server MUST load all configuration from its environment at startup and MUST refuse to start (with an error naming the missing or invalid item) when required configuration is absent.
- **FR-010**: Every tool invocation MUST produce exactly one structured invocation record carrying: tool name, model identifier, input/output token counts, cost, latency, outcome classification, and session identifier.
- **FR-011**: The server MUST make no network connections other than to the configured model provider, and MUST introduce no capability (beyond responding to MCP clients and calling that provider) that is not explicitly enabled by configuration.

### Key Entities

- **Corrective mode**: A named corrective the server offers (Verify is the first). Defined by its identity, instruction template, output structure, and selection hints — data, not bespoke machinery per mode.
- **Verdict**: The result of a Verify invocation — supported/refuted status, a list of specific findings, and a confidence signal.
- **Invocation record**: The observability record of one tool call — tool, model, tokens, cost, latency, outcome class, session.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A stock MCP client connects, lists tools, and completes a Verify call with no setup beyond providing the one required credential.
- **SC-002**: 100% of successful Verify results validate against the declared output structure across an acceptance run of at least 20 varied claims — zero parse failures, zero free-text fallbacks (the standard the project's spike set: 15/15).
- **SC-003**: Verification catches seeded errors: across an acceptance set of at least 10 claims with known planted errors and at least 6 sound claims, at least 90% of the seeded errors are refuted with the specific error named, and 0 of the sound claims are refuted (the spike's calibrated profile achieved 6/6 catch with 0/6 false positives).
- **SC-004**: Stance-blindness holds: presenting the identical claim with and without requester confidence framing changes the verdict in 0 of the acceptance cases.
- **SC-005**: In induced-failure testing, an operator can identify the failure class (refusal vs truncation vs timeout vs configuration) from the error alone in 100% of cases.
- **SC-006**: A single Verify call completes in under 30 seconds at default settings.
- **SC-007**: 100% of invocations (successes and failures) leave exactly one invocation record with all required fields populated.

## Assumptions

- **Verify is the first and only corrective in this feature.** The design corpus validates it by spike and every other layer depends on this path; additional primitives (Step, Decide, Diverge, Search, Recall, Research) are separate features.
- **One model provider (Anthropic) in this feature.** Provider portability is an open design question explicitly deferred; the constrained-output contract is the stable boundary.
- **Stateless by default.** No session memory or recall semantics in this feature; the session identifier on invocation records is for observability correlation only.
- **Invocation records are the observability foundation, not a dashboard.** This feature requires the records to exist and be complete; aggregation and display are out of scope.
- **The four pre-build spikes named in the design corpus** (schema sanitization fidelity, provider happy path, structured round-trip, thinking-mode compatibility) are planning/implementation concerns and do not change this specification's scope.
- **Cost on invocation records** is computed from token counts and the configured model's published pricing; exactness to the provider's invoice is not required.
