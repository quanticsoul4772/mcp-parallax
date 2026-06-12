# Feature Specification: Unstick Mode — Second Corrective on the Registry

**Feature Branch**: `002-unstick-mode`

**Created**: 2026-06-12

**Status**: Draft

**Input**: User description: "Unstick mode: second corrective on the existing mode registry — the Step/Unstick primitive from the design corpus (corrects stuck/looping: externalize one structured next step). Single constrained-output tool like verify; validates the 'modes are data — mode #2 is a registry entry, not a new subsystem' claim."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Get one concrete next step when stuck (Priority: P1)

An assistant (the calling model, via its MCP host) is stuck: it has a goal, it
has tried things, and it is producing plausible motion that goes nowhere. It
invokes **Unstick** with the goal, what it has tried, and where it is blocked.
It receives back exactly **one** concrete, immediately actionable next step
with a short rationale — never a menu of options, never a multi-step plan. The
single committed step is the corrective: an external frame breaks the loop the
model cannot see from inside.

**Why this priority**: In the prior server's 5.5 months of organic usage, the
cheap structured step was the #1 most-used corrective (67 invocations) —
stuckness is the most common failure the catalog corrects. This story is the
entire user value of the feature.

**Independent Test**: Invoke Unstick with a realistic stuck scenario (goal +
tried list + blocker) and confirm a structurally valid result containing
exactly one actionable step that is not a restatement of an already-tried item.

**Acceptance Scenarios**:

1. **Given** a goal, a list of attempts, and a description of the blocker, **When** Unstick is invoked, **Then** the result contains exactly one next step, stated as a concrete action (not "consider X or Y").
2. **Given** the attempts list names specific things already tried, **When** Unstick is invoked, **Then** the returned step is not a restatement of any listed attempt.
3. **Given** any successful invocation, **When** the result is returned, **Then** it conforms to the tool's declared output structure — every time, with no free-text fallback.
4. **Given** only a goal and a blocker (no attempts list), **When** Unstick is invoked, **Then** it still returns one concrete step grounded in what was provided.

---

### User Story 2 - The second corrective inherits every core guarantee (Priority: P2)

An operator (and the calling assistant) gets the same guarantees from Unstick
that the first corrective established: it appears in the tool catalog beside
Verify with declared input and output structures, its failures surface as the
same distinct named classes, and every invocation leaves exactly one
observability record identifying which corrective ran. Nothing about the first
corrective's behavior changes.

**Why this priority**: This feature exists partly to prove the architecture
claim that adding corrective #2 is a data addition, not a new subsystem. The
proof is behavioral: all existing guarantees hold for both tools, and the
first tool's behavior is untouched.

**Independent Test**: List the catalog (both tools present with structures);
induce a failure on Unstick (same distinct error classes); inspect records
(one per invocation, correctly attributed per tool); run the entire existing
test suite unchanged (all pass).

**Acceptance Scenarios**:

1. **Given** a connected MCP client, **When** it requests the tool catalog, **Then** it sees both correctives, each with a description and declared input/output structures.
2. **Given** an Unstick invocation that fails (provider refusal, timeout, invalid input), **When** the error surfaces, **Then** it names the same failure class it would for the first corrective.
3. **Given** invocations of both tools, **When** the operator inspects invocation records, **Then** each record identifies which corrective ran, with all required fields.
4. **Given** the existing test suite for the first corrective, **When** it runs after this feature lands, **Then** every test passes unchanged.

---

### Edge Cases

- Empty or whitespace-only goal: rejected as invalid input with a descriptive error before any model call.
- Oversized input (goal + attempts + blocker beyond the configured limit): rejected with a descriptive error, never silently trimmed.
- Vague blocker ("it just doesn't work"): still returns one concrete step — typically a diagnostic action — grounded in whatever was provided.
- A response containing multiple alternatives or an empty step: fails local validation and surfaces as an error, never returned as a result.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The server MUST expose an Unstick tool accepting a goal (required), a description of where the caller is blocked (required), and an optional list of attempts already made.
- **FR-002**: The result MUST contain exactly one next step stated as a single concrete action, a short rationale explaining why it breaks the loop, and optionally one pitfall to watch for — never multiple alternative steps and never a multi-step plan.
- **FR-003**: The returned step MUST NOT be a restatement of an item in the provided attempts list.
- **FR-004**: Unstick MUST carry the same constrained-output guarantee as the existing corrective: output constrained to the declared structure at generation time, value constraints enforced by local validation, structures flat and closed.
- **FR-005**: Unstick MUST surface the same distinct failure classes as the existing corrective and MUST produce exactly one invocation record per call, identifying the corrective that ran.
- **FR-006**: Input validation MUST reject an empty goal or blocker and oversized input before any model call, classified as invalid input.
- **FR-007**: One Unstick invocation MUST use a single verification-free generation pass — it is the cheap workhorse corrective, and its cost MUST stay proportionate (one model call per invocation).
- **FR-008**: Adding Unstick MUST NOT change any externally observable behavior of the existing corrective.

### Key Entities

- **Unstick request**: goal, blocker description, optional attempts list.
- **Next step**: the single committed action, its rationale, and an optional pitfall.
- **Invocation record**: unchanged from the core layer; the tool identity now distinguishes correctives.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A stock MCP client lists both correctives, each with declared input and output structures, and completes an Unstick call with no new setup.
- **SC-002**: 100% of successful Unstick results conform to the declared output structure across an acceptance run of at least 10 varied stuck scenarios.
- **SC-003**: In the acceptance run, 100% of results contain exactly one actionable step (zero option-menus or plans), and 0 results restate an already-tried item.
- **SC-004**: A single Unstick call completes in under 15 seconds at default settings (single pass — faster than the ensemble corrective).
- **SC-005**: 100% of Unstick invocations (successes and failures) leave exactly one invocation record correctly identifying the tool.
- **SC-006**: The existing corrective's full test suite passes unchanged after the feature lands — zero modified assertions.

## Assumptions

- **Single pass (k=1).** Unstick is generative, not evaluative: it produces a step rather than judging a claim, so the ensemble/agreement machinery that protects Verify's verdicts adds cost without validated benefit here. The per-mode pass count is already mode data.
- **Stateless.** Unstick sees only what the caller provides in the call — no memory of prior invocations (Recall is a later layer). Loop-breaking across repeated calls relies on the caller passing an updated attempts list.
- **Same provider, model, and configuration** as the core layer; no new environment variables are required beyond what exists.
- **"One step" is a product decision, not a model limitation**: the design's corrective for stuckness is commitment to a single externalized step; option menus reintroduce the indecision failure mode this catalog treats separately (the future Converge/Decide corrective).
