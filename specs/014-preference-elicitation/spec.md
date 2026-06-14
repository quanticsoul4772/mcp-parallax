# Feature Specification: Preference Elicitation — the Wrong-Objective Corrective

**Feature Branch**: `014-preference-elicitation`

**Created**: 2026-06-14

**Status**: Draft

**Input**: User description: "Preference elicitation — the wrong-objective corrective. The
model reliably solves the assumed problem rather than the user's actual one: it commits to
an objective inferred from the surface of the request and never surfaces the preferences
and constraints that should govern the work. This corrective makes the operative objective
and preferences explicit before the model commits. Given a task plus the available signals
(the stated request, the conversation/context, and — when memory is configured — stored
verified preferences), it returns the objective it would otherwise have assumed, the
preferences/constraints that actually bear on the work (inferred from revealed signals and
stored memories, not by interrogating the user), and the points where the assumed
objective likely diverges from the real one. Stance: inference, not interrogation —
revealed preferences outrank stated ones; surface what can be inferred rather than dumping
a questionnaire. Scope: this is the elicitation/surfacing half; enforcement (holding an
action that conflicts with a stored preference) already exists as checkpoint_action over
memory and is not rebuilt. Reuses the constrained-output contract and, when present, the
memory recall seam. Distinct from verify/unstick/diverge/decide: it names the objective
itself and the preferences that should drive it. No signal means it says so — it does not
invent preferences."

## User Scenarios & Testing *(mandatory)*

Parallax catches the ways the calling model reliably goes wrong from inside its own
context. **Solving the assumed problem** is one of the most expensive: the model reads the
surface of a request, silently commits to an objective, and produces a technically-correct
answer to the *wrong question* — having never surfaced the preferences and constraints that
should actually govern the work. From inside its own frame the model cannot see the
objective it assumed; it just executes it. `Preference elicitation` is the external pass
that makes the operative objective and the governing preferences **explicit before the
model commits**, and names where the assumed objective likely diverges from the user's
real one — the questions worth resolving first.

It is the **complement** of the other correctives, and it runs *earlier*: `verify` judges a
claim, `unstick` commits to a step, `diverge` opens framings, `decide` chooses among given
options — but all of those presuppose the objective is right. This corrective checks the
objective *itself*. Its stance is **inference, not interrogation**: it infers from revealed
signals (what the user has actually done, chosen, corrected) and stored verified
preferences, weighting those above merely stated ones, and surfaces what it can infer
rather than handing back a questionnaire.

### User Story 1 - Surface the assumed objective and what should govern it (Priority: P1)

A caller is about to execute a task and asks `Preference elicitation` first. The tool
returns the **objective it would otherwise have assumed** from the request's surface, plus
the **preferences and constraints** that actually bear on the work — inferred from the
stated request, the context, and (when memory is configured) the user's stored verified
preferences. The caller sees, before committing, what objective it was about to pursue and
what should shape it.

**Why this priority**: This is the core value — making the silent objective visible. A
model that surfaces "I was about to optimize for X; your revealed preferences point to Y"
can correct course before producing the wrong answer, which is far cheaper than discovering
the mismatch after the work is done.

**Independent Test**: Submit a task whose surface reading implies one objective while the
context/preferences point to another, and confirm the tool surfaces both the assumed
objective and the governing preferences, with the preferences traced to their signals.
Testable through the tool's output.

**Acceptance Scenarios**:

1. **Given** a task with an obvious surface objective and context/preferences that qualify
   it, **When** the tool runs, **Then** it returns the assumed objective and the governing
   preferences/constraints, each tied to the signal it was inferred from.
2. **Given** a task with stored verified preferences relevant to it (memory configured),
   **When** the tool runs, **Then** those stored preferences appear among the governing
   preferences and are marked as the stronger (revealed/verified) signal.

### User Story 2 - Name the divergence points worth resolving (Priority: P1)

The same caller needs to know **where** the assumed objective likely departs from their
real one — the specific decisions that, if assumed wrong, would send the work astray.
`Preference elicitation` returns those **divergence points**: the assumptions baked into
the surface objective that the available signals call into question, framed as the
questions worth resolving before proceeding (not a generic questionnaire — only the points
where signal suggests a real gap).

**Why this priority**: P1 because surfacing the objective without naming where it might be
wrong is only half the value. The divergence points are the actionable output — they tell
the caller exactly what to confirm before committing, and they are *signal-driven*, so they
do not degrade into interrogation.

**Independent Test**: Submit a task where a baked-in assumption conflicts with a revealed
signal and confirm the tool names that specific divergence point; submit a task with no
conflicting signal and confirm it returns no manufactured divergence points.

**Acceptance Scenarios**:

1. **Given** a surface objective that rests on an assumption the signals contradict, **When**
   the tool runs, **Then** it names that divergence point as a question worth resolving,
   citing the conflicting signal.
2. **Given** a task where the signals are consistent with the surface objective, **When**
   the tool runs, **Then** it returns **no** divergence points — it does not manufacture
   doubt where there is none.

### User Story 3 - Inference, not interrogation; no signal means say so (Priority: P2)

A caller submits a task with little or no preference signal — no relevant context, no stored
preferences. `Preference elicitation` must **not** invent preferences or dump a list of
generic questions. It surfaces the assumed objective, states plainly that it has little
signal about the user's actual preferences, and returns at most the few genuinely
signal-grounded points (often none). Inference is bounded by signal.

**Why this priority**: P2 because it is the guardrail that keeps the tool from becoming the
very interrogation it exists to avoid. Without it, the tool would hallucinate preferences
and erode trust; with it, the tool is honest about the limits of what it can infer.

**Independent Test**: Submit a task with no preference signal and confirm the output marks
the preference inference as low/absent and returns no fabricated preferences or divergence
points.

**Acceptance Scenarios**:

1. **Given** a task with no relevant preference signal, **When** the tool runs, **Then** it
   surfaces the assumed objective, reports that preference signal is low/absent, and returns
   no fabricated preferences or divergence points.

### Edge Cases

- **Stated vs revealed conflict**: the user *said* one preference but their stored/revealed
  behavior shows another. The tool weights revealed/verified over stated and surfaces the
  conflict as a divergence point, rather than silently trusting the stated one.
- **Memory not configured**: stored preferences are unavailable; the tool still runs on the
  stated request + context alone, and says it is working without stored-preference signal.
- **The request is already fully specified** (objective + constraints explicit): the tool
  confirms the objective and returns few or no divergence points — it does not invent gaps.
- **Enforcement is out of scope**: the tool surfaces preferences; it does **not** block,
  hold, or modify any action. Holding an action that conflicts with a stored preference is
  `checkpoint_action`'s job (it already exists); this tool runs earlier and only surfaces.
- **Empty / oversize task statement**: rejected before any model call, like the rest of the
  family (no silent trimming).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The tool MUST accept a task/decision statement plus optional context and
  return the **assumed objective** (the objective a surface reading would commit to) made
  explicit.
- **FR-002**: The tool MUST return the **governing preferences/constraints** that bear on
  the work, each **traced to the signal** it was inferred from (stated request, context, or
  a stored verified preference).
- **FR-003**: When memory is configured, the tool MUST consult **stored verified
  preferences** relevant to the task and weight them as the **stronger (revealed/verified)**
  signal over merely stated ones; when memory is absent, it MUST run on stated request +
  context alone and say so.
- **FR-004**: The tool MUST return **divergence points** — the specific assumptions in the
  surface objective that the available signals call into question — framed as questions
  worth resolving, each citing the conflicting signal.
- **FR-005**: The tool MUST be **inference-bounded**: it MUST NOT fabricate preferences or
  divergence points absent signal. With little/no signal it MUST report that explicitly and
  return few or none (no interrogation, no hallucinated preferences).
- **FR-006**: The tool MUST **only surface** — it MUST NOT block, hold, or modify any action
  (enforcement is `checkpoint_action`'s role, not rebuilt here). Its output is advisory.
- **FR-007**: The model produces the **structured inference**; the server assembles the
  surfaced objective, the traced preferences, and the divergence points (server-assembled
  output, per the constrained-output contract).
- **FR-008**: Input validation MUST reject an empty/whitespace or oversize task statement
  before any model call (no silent trimming).
- **FR-009**: The tool's behavior MUST be **unchanged in shape whether or not memory is
  configured** — memory only adds stored-preference signal; its absence is reported, not an
  error (the tool is always in the catalog).

### Key Entities

- **Task statement**: what the caller is about to do — the primary input whose surface
  objective is examined.
- **Assumed objective**: the objective a surface reading of the task would silently commit
  to, made explicit by the tool.
- **Governing preference**: a preference or constraint that should shape the work, with the
  signal it was inferred from and a strength (revealed/verified > stated).
- **Divergence point**: an assumption in the assumed objective that a signal calls into
  question — a question worth resolving before proceeding, with the conflicting signal.
- **Signal**: the source a preference or divergence point was inferred from — the stated
  request, the provided context, or a stored verified preference (when memory is present).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a task whose surface objective is qualified by context/preferences, the
  tool surfaces both the assumed objective and at least the governing preferences, each
  traced to its signal — the silent objective is made visible.
- **SC-002**: On a task where a baked-in assumption conflicts with a revealed/stored signal,
  the tool names that divergence point citing the conflicting signal — **100%** of such
  seeded conflicts surfaced on a fixed battery.
- **SC-003**: On a task with no preference signal, the tool fabricates **0** preferences and
  **0** divergence points and reports the low/absent signal — inference stays bounded.
- **SC-004**: When memory is configured, a relevant stored verified preference appears among
  the governing preferences marked as the stronger signal on **100%** of tasks where one
  exists; when memory is absent, the tool runs and reports the missing signal rather than
  erroring.
- **SC-005**: The tool **never** blocks, holds, or modifies an action — **0** enforcement
  actions in its output; it is purely advisory (enforcement remains `checkpoint_action`).

## Assumptions

- `Preference elicitation` is a corrective named in the design corpus
  (`NEW_SERVER_DESIGN.md` catalog — *wrong objective → preference elicitation + enforcement*;
  `PREFERENCE_ELICITATION.md`). This feature implements the **elicitation/surfacing half**;
  the constitution's design-corpus-fidelity check is an application, confirmed at
  `/speckit-plan`.
- The **enforcement half** (holding an action that conflicts with a stored verified
  preference) already exists as `checkpoint_action` over memory and is **not rebuilt** here
  — a named scope boundary (this tool surfaces up front; `checkpoint_action` enforces at
  action time).
- It reuses the constrained-output contract (the model emits the structured inference; the
  server assembles the surfaced output) and, **when memory is configured**, the existing
  memory recall seam for stored verified preferences. Whether memory presence *gates* the
  tool or merely enriches it is a **`/speckit-plan` decision** (the description leans toward
  always-on with memory as optional enrichment).
- The structured inference fields, how revealed/stated strength is represented, and the
  recall/relevance mechanism for stored preferences are **`/speckit-plan` decisions**
  (mirroring how prior correctives deferred mechanism to planning).
- The output is advisory for the caller to act on before committing — the tool does not
  execute the task, choose an option (that is `decide`), or enforce anything.
