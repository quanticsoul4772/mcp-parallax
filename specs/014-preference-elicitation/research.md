# Research: Preference Elicitation — the Wrong-Objective Corrective

Phase 0 decisions. The clarification settled availability (always-on; memory enriches) and
the stored-preference source (server recalls). These resolve the mechanism against the
existing memory recall seam and the flat+closed contract.

## D1 — Always-on mode with an optional memory dependency

**Decision**: register `elicit` **unconditionally** (always in the catalog), like
`verify`/`unstick`/`diverge`/`decide`. It carries an **optional** reference to the
existing `MemoryDeps` (the server's `memory: Option<MemoryDeps>`, present only when
`VOYAGE_API_KEY` is set). `elicit::run(client, mode, memory: Option<&MemoryDeps>, params,
max)` recalls stored preferences only when `memory` is `Some`.

**Rationale**: the clarification chose always-on with memory as enrichment. The tool adds
**no new gate or egress** — the embedder call (when memory is present) is the *existing*
memory capability, already gated on `VOYAGE_API_KEY` (Constitution VI). Without memory the
tool still surfaces the assumed objective and context-inferred preferences and reports the
missing stored-preference signal (FR-009/SC-004).

**Alternatives**: gate the tool on `VOYAGE_API_KEY` (rejected by the clarification — the
assumed-objective value doesn't need stored prefs); a new env gate (rejected — no new
capability is introduced).

## D2 — Server recalls stored preferences, injects them into the prompt

**Decision**: when `memory` is present, the server calls the existing
`memory::tools::recall(deps, &RecallParams { query: <the task>, kind: None, limit:
RECALL_LIMIT })` (a small constant, e.g. 5), keeps the **trusted** memories (the
verified/revealed signal), and formats them into the prompt as *"stored verified
preferences (revealed signal — these outrank merely stated ones)"*. The model sees them
and can (a) treat them as the stronger signal and (b) raise a **divergence point** when a
stated request contradicts a stored preference.

**Rationale**: FR-003 says the *server* consults stored preferences; reusing `recall`
(embed the task → cosine-rank → top-k) is the established machinery (no re-implementation,
Constitution IV). Injecting them into the prompt is what lets the model weight
revealed > stated and detect stated-vs-revealed conflicts (the divergence points).

**Alternatives**: the caller pre-supplies preferences (rejected by the clarification); the
server appends recalled prefs to the output *without* showing the model (rejected — then
the model can't detect stated-vs-revealed conflicts, gutting FR-004's divergence points).

## D3 — Per-pass constrained-output schema (flat + closed) via parallel arrays

**Decision**: the single pass emits the structured inference as flat fields, with the
variable-length per-item data carried as **parallel scalar arrays** (the 013 pattern;
arrays of objects are illegal):

- `assumed_objective`: string — the objective a surface reading would commit to.
- `preference_texts`: string[] · `preference_signals`: string[] (where each was inferred
  from) · `preference_strengths`: string[] (`"revealed"` or `"stated"`, **server-validated**
  — nullable-enum-free, the 011 H1 lesson; here a required non-null string in an array).
- `divergence_questions`: string[] · `divergence_signals`: string[] (the conflicting
  signal each cites).
- `signal_level`: enum `low | medium | high` — a **scalar enum** (flat-legal,
  grammar-enforced, like 013's `Methodology`); the model's self-report of how much
  preference signal it had (FR-005).

**Rationale**: flat + closed under `assert_flat` (string, arrays of scalars, one scalar
enum). The per-item arrays are index-aligned; the server zips them. `preference_strengths`
is a string (not an array-of-enums) and validated server-side, avoiding any enum-in-array
grammar surprise.

## D4 — Server assembly + well-formedness validation

**Decision**: after the pass, the server validates and zips:

1. Well-formedness (else a **loud failed pass**, the 013 M1 convention): the three
   preference arrays are equal length; the two divergence arrays are equal length; every
   `preference_strengths` value is `"revealed"` or `"stated"`.
2. Zip into `GoverningPreference { preference, signal, strength }` and `DivergencePoint
   { question, signal }`.
3. Assemble `ElicitResult { assumed_objective, governing_preferences, divergence_points,
   signal_level, memory_consulted }`, where `memory_consulted = memory.is_some()`.

Empty preference/divergence arrays are **valid** (low signal — FR-005); the server does not
fabricate. No enforcement field exists in the output (FR-006/SC-005 — structurally pure
advisory).

**Rationale**: deterministic assembly over the model's structured inference (Constitution
V applied to the assembly); loud-over-silent validation matches 013.

## D5 — Testing: mechanism + recall-integration offline; inference quality live

**Decision**: offline (`cargo test`, mocked model + mock embedder + in-memory storage):

- The per-pass schema registers flat + closed; arity/strength validation is a loud failed
  pass; the zip assembles the nested output; a low-signal canned inference → empty
  preferences/divergence (SC-003 shape); the output never carries an enforcement field
  (SC-005); `memory_consulted` reflects presence.
- **Recall integration (SC-004 mechanism):** seed a trusted memory + mock embedder; assert
  the recall reaches the **prompt** (the mock model captures a request body containing the
  memory content) and that `memory_consulted` is true — proving the server consults stored
  preferences. Without memory, assert the tool runs and the prompt notes no stored signal.

**Live** (dogfood): SC-001 (surfaces the *right* assumed objective), SC-002 (catches a
seeded stated-vs-revealed conflict as a divergence point), and that the model marks stored
prefs `revealed` — these are model-judgment properties a mock cannot produce.

**Rationale**: the recall + assembly are deterministic and offline-provable; only the
inference *quality* (right objective, real conflicts) needs the live model — a moderate
live surface, like `verify`/`diverge`.

## D6 — Output surface

`ElicitResult` (server-assembled, nested like `decide`'s assessments):

- `assumed_objective` (string), `governing_preferences` (`[{preference, signal,
  strength}]`), `divergence_points` (`[{question, signal}]`), `signal_level` (enum),
  `memory_consulted` (bool). **No** verdict, no chosen option, no action/hold — surfacing
  only (FR-006).
