# Feature Specification: Per-Hop Model Routing

**Feature Branch**: `018-model-routing`

**Created**: 2026-07-24

**Status**: Draft

**Input**: User description: "Per-mode model routing: let each model hop run on a model chosen for the work it does, instead of every hop sharing one global ANTHROPIC_MODEL."

## User Scenarios & Testing *(mandatory)*

The operator of a Parallax deployment is the actor throughout. They configure the
server, pay its provider bill, and read its cost records. The calling model is a
beneficiary, not an actor: it never chooses a model and cannot observe which one
answered.

### User Story 1 - Route mechanical work to a cheaper model (Priority: P1)

An operator is paying frontier-model rates for every model call the server makes,
including work that does not need frontier judgment. The costliest example is the
research pipeline's per-source claim extraction: it runs once for every fetched
page and consumes the largest share of a run's tokens, but the task is mechanical
transcription — pull the falsifiable statements out of a page. The operator wants
to point that work at a cheaper model while the judgment calls (verification,
choosing among options, surfacing objectives, reviewing a turn) stay on the
capable model they trust.

**Why this priority**: This is the entire motivation and the only story that
delivers savings. Everything else in the feature exists to keep this honest.
Shipped alone it is already a viable, valuable increment.

**Independent Test**: Run one research question twice — once with no routing
configured, once with the extraction hop routed to a cheaper model — and compare
recorded cost and the resulting answer. Delivers value when cost drops materially
and the answer's verified findings are unchanged.

**Acceptance Scenarios**:

1. **Given** no routing is configured, **When** the operator runs any tool, **Then**
   every model call uses the single default model and the run is indistinguishable
   from the same run before this feature existed.
2. **Given** the extraction hop is routed to a cheaper model, **When** the operator
   runs a research question, **Then** extraction runs on the cheaper model, every
   other hop runs on the default model, and the recorded cost is lower than the
   same question with no routing configured.
3. **Given** two hops are routed to the same model, **When** the server starts,
   **Then** they share one connection to that model rather than opening two.
4. **Given** a hop is routed to a model, **When** the operator inspects the answer
   returned to the caller, **Then** it carries no indication of which model
   produced it — routing is an operator concern, not part of the tool contract.

---

### User Story 2 - Keep cost attribution correct across models (Priority: P2)

Once a single invocation spans two models, "what did this call cost" stops having
a single answer. Today one invocation is recorded with one model name and one
token total, and the cost is that one model's rate applied to all of them. After
routing, that arithmetic is wrong under either model's rate — the operator would
be reading a number that is confidently incorrect, which is worse than no number.
The operator needs recorded cost that reflects what was actually spent, and needs
to see which models participated.

**Why this priority**: Without it, User Story 1 delivers savings the operator
cannot measure and mis-reports the spend it was adopted to reduce. It is second
only because savings exist even while the accounting is being fixed.

**Independent Test**: Run one invocation deliberately spanning two models, compute
the expected cost by hand from each model's published rates and its own token
counts, and compare against what the server recorded. Delivers value when the two
agree and the record names both models.

**Acceptance Scenarios**:

1. **Given** an invocation whose hops ran on two different models, **When** the
   operator reads its cost record, **Then** the recorded cost equals the sum over
   models of that model's tokens at that model's own rate.
2. **Given** the same invocation, **When** the operator reads its record, **Then**
   the record identifies every model that participated, not just one.
3. **Given** the same invocation, **When** the operator reads the exported
   telemetry, **Then** the exported cost and model attribution agree with the
   stored record.
4. **Given** no routing is configured, **When** the operator compares cost records
   against records written before this feature, **Then** the values are unchanged
   for equivalent runs.

---

### User Story 3 - Route safely across model families (Priority: P3)

Routing means several model families running inside one process at once, and they
do not all behave alike. Some families reason before answering unless told not to,
and that reasoning is charged against the same output budget as the answer — on a
budget sized for a short verdict, the answer can be cut off before it is finished.
Families also differ in whether that reasoning can be switched off at all, and
newer models may be absent from the server's price list. The operator wants to
route a hop to any model the provider offers and get either a correct result or a
clear, early failure — never a truncated verdict or a silently mispriced run.

**Why this priority**: It widens which models are safe to route to. The primary
saving in User Story 1 is reachable with a model family that has none of these
differences, so this can follow.

**Independent Test**: Route a hop to a model from each supported family in turn
and run the hop's tool. Delivers value when every family returns a complete result
and a correct cost, or fails at startup with a message naming the setting at fault.

**Acceptance Scenarios**:

1. **Given** a hop routed to a model that reasons before answering by default,
   **When** that hop runs, **Then** the result is complete rather than cut off
   mid-answer.
2. **Given** a hop routed to a model whose price the server does not know, **When**
   the hop runs, **Then** the run still completes, the cost is estimated
   conservatively rather than under-reported, and the estimate is marked as such
   so the operator can tell it apart from a known price.
3. **Given** a routing setting that names something the server cannot use, **When**
   the server starts, **Then** it refuses to start and names the offending setting
   and value.
4. **Given** a hop in the checkpoint layer routed to an unreachable model, **When**
   that checkpoint fires, **Then** the turn proceeds unimpeded and the failure is
   recorded — the checkpoint layer never blocks work on its own malfunction.

---

### Edge Cases

- A routing setting is present but empty, or names a model the server cannot use.
  Per the project's configuration convention this is an error at startup, never a
  silent fall back to the default.
- A routing setting is misspelled such that the server would never read it. Left
  undetected the operator believes a hop is routed when it is not, and the only
  symptom is a bill that does not fall. Because routing settings occupy a reserved
  namespace, the server can see a name it does not recognise and refuse to start
  (FR-006a), rather than ignoring it.
- A call site is routed both by its tier and by its own setting. The more specific
  setting wins; the tier still governs every other call site assigned to it.
- Every hop is routed to the same model. This must behave exactly like setting the
  default model to that value, with no duplicated connections.
- An invocation's hops all run on one model. Its cost record must remain as simple
  and as accurate as it is today; the multi-model accounting must not distort the
  common single-model case.
- A run stops early — budget exhausted, deadline reached, a hop fails and its work
  is dropped. Partial work still consumed tokens on whichever model performed it,
  and must still be costed to that model.
- A model is routed for a hop that never executes on a given run (for example the
  research synthesis hop when nothing survived verification). It must contribute
  nothing to cost and must not appear as a participant.
- The provider ships a new model after this feature is built. Routing to it must
  work without a code change, even though its price is not yet known.

## Requirements *(mandatory)*

### Functional Requirements

**Routing**

- **FR-001**: Each model call site MUST be able to run on a model chosen
  independently of the other call sites.
- **FR-001a**: Call sites MUST be grouped into a small number of named tiers by the
  kind of work they do, and an operator MUST be able to route a whole tier with one
  setting.
- **FR-001b**: An operator MUST be able to override any individual call site,
  overriding whatever its tier says.
- **FR-001c**: The model for a call site MUST resolve in a fixed, documented order:
  its own setting if present, otherwise its tier's setting if present, otherwise the
  server-wide default.
- **FR-002**: With no routing configured, the server MUST behave exactly as it did
  before this feature: one model for every call site, identical recorded costs, and
  no change to any tool's response.
- **FR-003**: Routing MUST be operator-configured only. No tool argument, prompt
  content, or caller-supplied value may influence which model answers, so the
  calling model cannot select a judge likely to agree with it.
- **FR-004**: Call sites routed to the same model MUST share one client; distinct
  models MUST get distinct clients.
- **FR-005**: The server MUST report, on demand, which model each call site is
  actually configured to use, so an operator can confirm a route took effect.
- **FR-006**: A routing setting that is present but unusable MUST stop the server
  at startup with a message naming the setting and the value, never a silent
  fallback to the default.
- **FR-006a**: Routing settings MUST occupy a reserved namespace, and a setting in
  that namespace whose name the server does not recognise MUST stop the server at
  startup naming the unrecognised setting, so a misspelled route cannot be silently
  ignored.

**Cost and attribution**

- **FR-007**: Token usage MUST be accounted per model within a single invocation.
- **FR-008**: An invocation's recorded cost MUST equal the sum, over the models
  that participated, of that model's own tokens priced at that model's own rate.
- **FR-009**: One invocation MUST continue to produce exactly one audit record, and
  that record MUST identify every model that participated and carry each model's own
  token usage alongside the single summed cost.
- **FR-009a**: An invocation whose call sites all ran on one model MUST record the
  same single cost figure it records today; the per-model detail is additional, not
  a replacement.
- **FR-010**: The exported telemetry surface MUST carry the same cost and model
  attribution as the stored record, derived so the two cannot disagree, and MUST
  keep emitting one span per invocation.
- **FR-011**: The server's price list MUST cover the models it ships knowing about,
  including current ones.
- **FR-012**: A model with no known price MUST be costed conservatively — never
  under-reported — and the resulting figure MUST be distinguishable from one
  computed at a known price.

**Model-family differences**

- **FR-013**: A call site MUST return a complete result regardless of whether its
  model reasons before answering by default; the output budget must accommodate
  both the reasoning and the answer, or the reasoning must be switched off where
  the family permits it.
- **FR-014**: Where a model family rejects a request shape the server would
  otherwise send, the server MUST send a shape that family accepts rather than
  failing at call time.
- **FR-015**: The checkpoint layer MUST remain fail-open: any routing or provider
  failure inside it results in silence and a recorded failure, never a blocked
  turn.

**Non-goals (stated as requirements so they are testable)**

- **FR-016**: The server MUST NOT expose model choice in any tool's input schema.
- **FR-017**: The server MUST NOT change any tool's output schema as a result of
  this feature.

### Key Entities

- **Call site**: One named place in the server that asks a model for a
  schema-constrained answer. The routable unit. Today's set: the five cognitive
  correctives (verify, unstick, diverge, decide, elicit), grounded verification,
  the deterministic layer's translation step, the four research steps (scope,
  extract, verify, synthesize), and the end-of-turn checkpoint review.
- **Tier**: A named grouping of call sites by the kind of work they do — mechanical
  transcription versus judgment. Tier membership is a property of the call site,
  fixed by the server; which model a tier uses is the operator's to set.
- **Route**: The operator's assignment of a model, either to a tier or to a single
  call site. Absent at both levels means "use the server-wide default".
- **Client pool**: The set of live model connections, one per distinct model in
  use, shared across the call sites routed to it.
- **Per-model usage**: Tokens consumed, attributed to the model that consumed
  them, accumulated within one invocation.
- **Invocation record**: The existing per-call audit row — still exactly one per
  invocation — extended with the set of participating models and their individual
  usage, its single cost figure now derived from all of them.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Routing the research extraction step to a cheaper model reduces the
  recorded cost of a standard research question by at least 30% relative to the
  same question with no routing configured.
- **SC-002**: The same routed question yields the same set of verified findings as
  the unrouted question, allowing for the run-to-run variation already observed
  without routing.
- **SC-003**: For any invocation spanning more than one model, the recorded cost
  matches a hand computation from published rates and per-model token counts, to
  within rounding.
- **SC-004**: With no routing configured, every recorded cost matches what the
  server recorded for the equivalent run before this feature, and the full existing
  test suite passes without modification to its expectations.
- **SC-005**: A misconfigured route stops startup and names the offending setting,
  with no tool call ever reaching a provider.
- **SC-006**: Every model in the shipped price list can serve every call site
  without a truncated or malformed result, measured across one run of each tool.
- **SC-007**: An operator can determine which model served each call site without
  reading the server's source or its provider bill.

## Assumptions

- The operator is the only party who configures routing. Callers are untrusted for
  this purpose by design, per FR-003.
- Configuration remains environment-variable based, matching every other setting in
  the server, with loud failure on an unusable value.
- The feature is off by default. An operator who sets nothing gets today's
  behavior, so adopting the release carries no forced migration.
- Cheaper models are appropriate for mechanical extraction and transcription work
  but not for judgment. This feature provides the mechanism; it does not decide the
  policy, and it ships with no hop routed anywhere by default.
- Existing per-run budget ceilings, deadlines, and concurrency limits continue to
  operate unchanged and are counted in tokens, not currency, so they are unaffected
  by which model consumed them.
- Model prices are compiled into the server as they are today; keeping them current
  remains a maintenance task, and FR-012 keeps an out-of-date list from
  under-reporting.
- The provider's behavioral differences between model families are stable enough to
  encode. Where they are not, FR-013 and FR-014 are satisfied by the conservative
  choice (a budget large enough for both reasoning and answer).

## Resolved Decisions

Two decisions materially changed the feature's shape and had no obvious default.
Both were put to the operator on 2026-07-24 and answered.

- **Routing granularity — tier default with per-call-site override.** Call sites are
  grouped into work-kind tiers an operator can route with one setting each, and any
  individual call site can be overridden on top. Resolution runs most-specific
  first: call site, then tier, then the server-wide default. Chosen over
  per-call-site-only (a dozen settings to express the common case) and tiers-only
  (no escape hatch — re-tiering one call site would be a code change). It also
  matches an existing pattern in the server, where research depth tiers supply
  defaults that explicit per-run constraints override. Because routing settings live
  in a reserved namespace, an unrecognised name in that namespace is a startup
  error, which is what makes the misspelled-route failure detectable. Captured as
  FR-001a through FR-001c and FR-006a.
- **Multi-model record shape — one row carrying a per-model breakdown.** An
  invocation keeps producing exactly one audit record and one exported span; the
  record gains the set of participating models and their individual token usage, and
  its cost becomes the sum across them. Chosen over one row per call site, which
  would break the server's one-record-per-invocation invariant, multiply row counts,
  and force a revisit of every existing per-invocation query and of the span model in
  the observability contract. Captured as FR-009, FR-009a, and FR-010.
