# Feature Specification: Per-Call Reasoning Effort

**Feature Branch**: `028-per-call-effort-argument`

**Created**: 2026-07-25

**Status**: Draft

**Input**: "Reasoning effort is unreachable by the caller. Feature 022 put effort in a `PARALLAX_EFFORT_*` environment namespace, mirroring 018's `PARALLAX_MODEL_*` shape. That was a copy, not a design decision."

## Context

### The control is in the wrong place

Feature 022 made reasoning effort settable per model call site through environment
variables. The only way to change one is to edit a configuration file and restart
the session — which destroys the context that motivated the change.

The consumer of these tools is a calling model, not a human at a terminal. A
setting the caller cannot reach is a setting the caller cannot use.

022 reached this shape by mirroring 018's model-routing namespace without asking
whether effort is the same kind of decision. It is not:

| Setting | Who decides | Why |
| --- | --- | --- |
| **Model** | Operator | Sets the rate the account is billed at — a deployment property |
| **Effort** | Caller | How much reasoning *this task* deserves — a per-task judgment |

### The precedent was already here

The `research` tool takes `depth` as a per-call parameter, plus `constraints`
carrying `budget_tokens`, `deadline_ms` and `max_sources`. Caller-facing
cost-and-rigor control was already designed on the one tool where it was designed
rather than copied. 022 walked past it.

### Why this feature follows 027 rather than preceding it

Effort support varies by model family — `claude-haiku-4-5` rejects the parameter.
Making effort per-call arguably makes a mismatch *more* likely, not less. 027
landed first so that a rejection names the model, the level, and both remedies. A
verification pass on that ordering returned supported 3/3, finding that per-call
selection raises the value of the diagnostic rather than lowering it.

027 also cannot currently be exercised without the very config edit and restart
this feature removes. Its verification falls out of this feature for free.

### The argument against, recorded rather than buried

A caller choosing its own reasoning depth makes spend unpredictable from
configuration alone, and the caller is a model that optimises for its own answer
quality rather than the operator's bill.

The counter: `research` already exposes `budget_tokens` and `deadline_ms` per
call, so that boundary was crossed deliberately once already; the environment
default remains in force whenever the caller says nothing; and in this deployment
the operator and the caller's user are the same person. This does not dissolve
the objection — it bounds it. The spec records the residual: **an operator can no
longer predict spend from configuration alone.**

### An assumption this feature corrects, not edits around

`tests/integration.rs:670` (018 T013) asserts that no tool input property names a
model or tier, under the stated rationale that *"routing is an operator concern,
not a caller one."* An `effort` property passes that assertion unchanged — effort
is neither a model nor a tier. But the rationale in its doc comment is precisely
the belief this feature narrows. The test must be re-grounded explicitly, stating
that **model** remains an operator concern while **effort** does not, rather than
being quietly left to pass on a technicality.

## Clarifications

### Session 2026-07-26

- Q: FR-007 requires the effort actually used to be determinable. Determinable
  where? → A: On the invocation record, not in the tool result. The record is the
  surface that restores what this feature gives up — spend stops being predictable
  from configuration, so it must remain explainable afterwards. Telemetry inherits
  it, since traces are derived from the same records.
- Q: FR-015 bounds per-call concurrency to lowering only. What bounds a
  caller-supplied verification pass count? → A: Lowering only, mirroring FR-015. A
  caller may request fewer passes than configured, never more.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The caller sets effort for one invocation (Priority: P1)

A calling model judges that a particular task warrants more or less reasoning
than the deployment's default. It says so in the call. Nothing is edited and
nothing is restarted.

**Why this priority**: This is the feature. Every other story is a property of
doing it safely.

**Independent Test**: Invoke a tool with an explicit effort and confirm the
outbound request carries that level, with no configuration change and no restart.

**Acceptance Scenarios**:

1. **Given** no effort configured anywhere, **When** a caller supplies an effort
   on one invocation, **Then** the request carries that level and the next
   invocation without one carries no effort at all.
2. **Given** an effort configured for the call site's tier, **When** a caller
   supplies a different effort, **Then** the caller's value is used for that
   invocation only.
3. **Given** a caller supplies an effort, **When** the same tool is invoked again
   without one, **Then** the configured default applies again — the per-call
   value does not persist.

---

### User Story 2 - Saying nothing changes nothing (Priority: P1)

A caller that supplies no effort gets exactly the behaviour it got before this
feature existed.

**Why this priority**: Equal-first with US1. The environment namespace is the
default layer and must remain authoritative when the caller is silent; a feature
that perturbs the silent path would change every existing call.

**Independent Test**: With the effort namespace empty and no per-call argument,
confirm the request body is byte-identical to before this feature.

**Acceptance Scenarios**:

1. **Given** an empty effort namespace and no per-call effort, **When** any tool
   is invoked, **Then** the request carries no effort field at all.
2. **Given** an effort set for a call site, **When** a caller supplies none,
   **Then** the configured level applies unchanged.

---

### User Story 3 - Precedence is stated and observable (Priority: P2)

Where an effort came from is determinable, so a surprising level can be traced to
whatever supplied it.

**Why this priority**: Four layers now resolve to one value. 022 already made
precedence observable at startup for the two configuration layers; adding a third
without extending that would make the resulting level unattributable.

**Independent Test**: With a tier effort, a site effort, and a per-call effort all
present, confirm the per-call value wins and the supplying layer is identifiable.

**Acceptance Scenarios**:

1. **Given** all three layers set, **When** a call is made, **Then** the per-call
   value is used.
2. **Given** a per-call effort and no configuration, **When** a call is made,
   **Then** the per-call value is used and no configuration is required for it to
   take effect.

---

### User Story 4 - The caller sets a verification's pass count (Priority: P2)

A calling model judges that a claim warrants fewer independent passes than the
deployment's default — a cheap sanity check rather than a full ensemble — and says
so in the call. It cannot ask for more than the deployment allows.

**Why this priority**: Same class of correction as US1, on the setting whose
misplacement is next most visible. Ranked below US1 because the pass count also
changes how the *result* must be read, which US1 does not.

**Independent Test**: Invoke a verification with a lower pass count and confirm
that many passes ran and that the result says so; then request more than the
configured count and confirm a caller error naming the ceiling.

**Acceptance Scenarios**:

1. **Given** a configured count of three, **When** a caller asks for one pass,
   **Then** one pass runs and the result reports one.
2. **Given** a configured count of three, **When** a caller asks for five,
   **Then** the call is rejected as a caller error stating the ceiling, and no
   passes run.
3. **Given** any invocation, **When** the caller supplies no count, **Then** the
   configured count runs and the result still reports the count used.

---

### User Story 5 - The caller lowers a research run's concurrency (Priority: P3)

A calling model wants a research run to go easier on the fetched hosts, or to
reduce load, and lowers the concurrency for that run. It cannot raise it above
what the operator configured.

**Why this priority**: Lowest of the three, because the default is already
reasonable and the caller rarely has a reason to change it. It is in scope because
it is the same misplacement, and leaving one instance unfixed would make the rule
in FR-009 an assertion the code contradicts.

**Independent Test**: Run research with a lowered concurrency and confirm no more
than that many tasks run concurrently; then request more than the ceiling and
confirm the run proceeds at the ceiling.

**Acceptance Scenarios**:

1. **Given** a configured ceiling of eight, **When** a caller asks for two,
   **Then** at most two tasks run concurrently.
2. **Given** a configured ceiling of eight, **When** a caller asks for sixteen,
   **Then** the run proceeds at eight and the effective value is recorded.

---

### Edge Cases

- A caller supplies an effort the provider does not accept for the routed model:
  the call goes out and 027's enriched rejection surfaces (FR-010). No boundary
  refusal, because refusing would need the capability table 027 rejected.
- A caller supplies a level that is not one of the recognised values: rejected as
  a caller input error, distinguishable from the provider rejecting a valid level.
- A tool whose work never reaches a model: an effort argument would be inert and
  therefore misleading, which is why the memory tools do not carry one (FR-011).
- One tool invocation that fans out to several model calls: the supplied effort
  applies to every call that invocation makes, since the caller's judgment is
  about the task, not about an internal phase it cannot see.
- Concurrent invocations at different efforts: each must carry its own level, with
  no leakage between them.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: A caller MUST be able to set the reasoning effort for a single
  invocation without any file being edited and without the session restarting.
- **FR-002**: Effort MUST resolve most-specific-first: the per-call argument, else
  the call site's configured effort, else its tier's, else unset.
- **FR-003**: Unset at every layer MUST send no effort field at all, leaving the
  request byte-identical to a deployment without this feature.
- **FR-004**: A per-call effort MUST NOT persist beyond the invocation that
  supplied it, and MUST NOT affect any concurrent invocation.
- **FR-005**: The effort argument MUST be optional on every tool that carries it.
- **FR-006**: An unrecognised level supplied by a caller MUST be rejected as a
  caller input error, distinct from a provider rejection of a valid level.
- **FR-007**: The **per-call effort override** MUST be recorded on the invocation
  record, alongside the model and depth already there, so a cost that is not
  predictable from configuration stays explainable after the fact. It MUST NOT be
  added to any tool's output schema. Telemetry inherits it without separate work,
  since traces and metrics are derived from the same records and the two surfaces
  cannot be allowed to disagree.

  The configured layers are deliberately **not** recorded, and this is a real
  tradeoff rather than a free one. The startup routing table goes to stderr and
  is **not persisted**, so a NULL-effort row read weeks later cannot be resolved
  to the level that ran without the log of that process — the same objection 018
  answered for models by recording the resolved model per invocation. What
  justifies the asymmetry is that a per-call override is the only part
  configuration cannot predict, and that is what this column exists to explain.
  Recording the *effective* level for the single-call-site tools is a
  defensible extension and is not taken here.

  A single field also cannot represent an invocation spanning several call
  sites at different configured levels, which `research` does by construction —
  though `research` does not carry the argument, so that case never writes this
  column today.
- **FR-007a**: An invocation with no override MUST record its absence, distinct
  from any level. Absent means *configuration applied*, which together with the
  tool name and the startup table determines the effort that was in force.
- **FR-008**: Model selection MUST remain unreachable by the caller. This feature
  narrows the "routing is invisible to callers" assumption to models only, and the
  test encoding that assumption MUST be re-grounded to say so explicitly rather
  than left to pass incidentally.
- **FR-009**: The design corpus MUST record why effort is caller-facing while
  model is not, so the distinction is not re-collapsed by a future feature copying
  a shape.
- **FR-010**: A per-call effort on a site whose routed model rejects the
  parameter MUST NOT be refused at the tool boundary. The call goes out and the
  provider's rejection surfaces with 027's diagnosis. No capability table is
  introduced, on the reasoning two adversarial reviews established for 027: such
  a table goes stale in the direction that refuses a configuration which would
  have worked.
- **FR-011**: The effort argument MUST appear on the seven correctives —
  `verify`, `unstick`, `diverge`, `decide`, `elicit`, `grounded_verify`,
  `check` — and MUST NOT appear on `research`, the memory tools, or the
  `checkpoint_*` tools. `research` already carries `depth` and `constraints` for
  this purpose; the memory tools' model hop is `save`'s verification gate, which is the server's trust boundary rather than the caller's task — a caller able to dial its effort down could weaken the check that keeps the store uncorrupted (`NEW_SERVER_DESIGN.md`: *verify before you store*); `checkpoint_*` are
  harness-triggered rather than caller-chosen.

### Functional Requirements — the other misplaced settings

Effort is one instance of a class: a control the operator was given because the
machinery was there, when the judgment it encodes belongs to the caller. All four
candidates are in scope.

- **FR-012**: `VERIFY_ENSEMBLE_K` MUST become settable per call on the tools that
  consume it — `verify`, `diverge`, and `grounded_verify` — resolving per-call
  first and falling back to the configured value.
- **FR-012a**: A caller-supplied pass count MUST NOT exceed the configured value,
  mirroring FR-015. Lowering is a caller judgment about how much rigor a claim
  warrants; raising spends the operator's budget on parallel model calls the
  operator did not authorise.
- **FR-013**: A result MUST report the pass count actually used, always rather
  than only when the caller supplied one. Confidence is derived from cross-pass
  agreement, so a confidence computed over fewer passes rests on a narrower basis
  and MUST NOT be indistinguishable from one computed over the configured
  default. Reporting unconditionally keeps the result shape constant; a field
  that appears only sometimes would make the common case the ambiguous one.
- **FR-014**: `RESEARCH_CONCURRENCY` MUST become settable per call on `research`,
  as an addition to the existing `constraints`, where the other per-call cost
  controls already live.
- **FR-015**: A caller-supplied concurrency MUST NOT exceed the configured value.
  Concurrency protects the operator's egress rate and the politeness budget owed
  to fetched hosts, neither of which is the caller's to spend. Lowering is a
  caller judgment; raising is not.
- **FR-016**: `MEMORY_RECALL_LIMIT` requires no change. `recall` already accepts a
  per-call `limit`, with the setting serving as its default — the codebase already
  did for recall what this feature does for the rest. This MUST be confirmed by
  test rather than assumed, and the corpus MUST cite it as the existing precedent
  alongside `research`'s `depth`.

**Why effort is recorded but the pass count is returned.** The two are not
inconsistent. Effort changes what a call costs, not how its answer should be
read, so it belongs on the record where cost is explained. The pass count is the
basis for the confidence in the result itself, so a reader of that result cannot
interpret the number without it.

### Key Entities

- **Effort level**: One of the recognised reasoning levels, or absent. Absent is a
  distinct state, not a synonym for any level.
- **Effort source**: Which layer supplied the level in force — per-call, call
  site, tier, or none.
- **Pass count**: How many independent passes a verification ran. Bounded above by
  the configured value; reported with every result that has one.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A caller can raise or lower reasoning effort for a single
  invocation with zero file edits and zero restarts.
- **SC-002**: With no per-call effort supplied and no effort configured, outbound
  requests are byte-identical to before this feature.
- **SC-003**: A per-call effort affects exactly one invocation; the following
  invocation reverts to the configured default.
- **SC-004**: 027's rejection diagnosis becomes observable without editing
  configuration or restarting — invoking a tool with a rejected level on a call
  site routed to a rejecting model surfaces the model, the level, and both
  remedies.
- **SC-005**: The recorded rationale distinguishes operator-owned settings from
  caller-owned ones in terms a later feature can apply without re-deriving it.
- **SC-006**: A caller can narrow a verification's pass count for one claim,
  cannot widen it beyond the configured value, and every result states the count
  it actually used.
- **SC-009**: Every invocation carrying a per-call effort records it, so a cost
  that configuration alone does not explain can be attributed to the call that
  caused it without reproducing it.
- **SC-007**: A caller can lower a research run's concurrency for one call and
  cannot raise it above the configured ceiling.
- **SC-008**: Each of the four settings this feature examines either has a
  per-call override or is recorded as deliberately operator-only with the reason
  stated. The corpus states the test by which any *future* setting is placed, so
  the question can be answered without re-deriving it.

## Assumptions

- The environment namespace stays as the default layer. This feature adds a more
  specific layer above it; it removes nothing.
- The set of recognised levels is unchanged from 022.
- Spend becomes unpredictable from configuration alone. Accepted knowingly: the
  same boundary was crossed by `research`'s per-call budget controls, and the
  operator and the caller's user are the same person in this deployment.
- Whether the provider accepts a level for a given model is not knowable at
  startup and no capability table will be introduced — settled by 027 after two
  adversarial reviews rejected one.
- Two of the four settings carry residuals that were raised before the scope was
  chosen and are accepted knowingly, not overlooked:
  - **Pass count and confidence.** `verify`'s confidence is derived from
    cross-pass agreement. A caller lowering the count narrows that basis. FR-013
    keeps the number honest by reporting the count used rather than by refusing
    the caller's judgment.
  - **Concurrency and egress.** A caller raising concurrency would be spending
    the operator's egress rate and the politeness budget owed to fetched hosts.
    FR-015 makes the per-call value a ceiling-respecting floor: it may lower,
    never raise.
- **Lowering-only is the rule for anything that multiplies spend**, not a
  one-off. Both the pass count (FR-012a) and concurrency (FR-015) may be reduced
  by the caller and not increased, because each raise buys work the operator did
  not authorise. Effort is deliberately not bounded this way: it selects how the
  provider spends a single call's budget rather than multiplying the number of
  calls, and `MAX_TOKENS` already caps that call.
- `recall`'s existing per-call `limit` is treated as prior art, not as work. If
  the confirming test shows it is not in fact reachable, FR-016 becomes
  implementation work rather than a confirmation.

## Plan-Level Decision (not a spec question, recorded so it is not glossed)

The client carries its effort from construction, which is why the client pool
keys on `(model, effort)`. A per-call effort has no pooled client. The plan MUST
choose between:

1. **The completion seam gains an effort parameter** — explicitly decided against
   by 018 research D2, and touching roughly thirty production call sites plus
   every mock.
2. **A client is constructed per call when an explicit effort is supplied** —
   preserves the seam and the pool for the default path, but builds an object per
   invocation on the explicit path.

The plan must pick one and say why. Neither is free, and the spec does not
prejudge it.
