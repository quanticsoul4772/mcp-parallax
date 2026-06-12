# Feature Specification: Memory Layer — Recall Corrective with Verified-Before-Stored Memory

**Feature Branch**: `003-memory-layer`

**Created**: 2026-06-12

**Status**: Draft

**Input**: User description: "Memory layer (the Recall corrective + verified-before-stored memory): the highest-leverage layer per MEMORY_LAYER.md. Durable cross-session memory for the calling model — skills/lessons/world-state stored only after verification (the poisoning defense), recalled by semantic relevance. Corrects the 'no memory across turns / re-deriving prior work / drift' failure modes. Gated by the storage-extension spike."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Save now, recall in a later session (Priority: P1)

An assistant solves something worth keeping — a debugging approach that worked,
a lesson from a failure, a durable fact about the project. It saves the memory
with its kind (skill, lesson, or fact) and its provenance (where the knowledge
came from). Days later, in a different session, an assistant working on a
related task recalls by describing what it needs in natural language — not by
exact keywords — and receives the most relevant memories, ranked, each carrying
its kind, provenance, trust standing, and age. The expensive failure this
corrects is re-deriving solved problems: the literature finding is that
the has-memory vs no-memory gap often exceeds the gap between model backbones.

**Why this priority**: The save→recall round trip is the entire user value;
everything else in this feature qualifies or protects it.

**Independent Test**: Save a set of distinct memories, then issue recall
queries phrased differently from the saved wording; the right memory comes back
at the top.

**Acceptance Scenarios**:

1. **Given** a saved skill describing a debugging approach, **When** a later session recalls with a semantically related query using different words, **Then** that skill is the top result.
2. **Given** several saved memories of different kinds, **When** recall is filtered to one kind, **Then** only memories of that kind are returned.
3. **Given** a recall query with no relevant stored memories, **When** recall runs, **Then** it returns an empty result — never an error, never an irrelevant memory dressed up as relevant.
4. **Given** any successful save or recall, **When** the result returns, **Then** it conforms to the tool's declared output structure.
5. **Given** two memories equally relevant to a query, **When** both were stored at different times, **Then** the more recent one ranks higher (recency breaks ties).

---

### User Story 2 - Externally-sourced memory is never trusted unverified (Priority: P2)

A memory whose provenance is external content (a webpage, a document, a
repository — anything not first-hand experience of the calling session) is the
poisoning attack surface: a fabricated "successful experience" planted today
steers all future behavior. The system refuses to admit externally-sourced
memories as trusted: they are either independently verified at save time (and
admitted as verified) or stored quarantined — flagged untrusted, ranked below
trusted memories at recall, and clearly labeled in results. First-hand
memories record their stated provenance and rank normally.

**Why this priority**: This is the design move that makes accumulated memory
safe to rely on — "curated, not credulous." Without it the layer's upside
inverts into a liability.

**Independent Test**: Save one first-hand memory and one external-provenance
memory without verification; recall both; the external one is labeled
untrusted and ranked below the first-hand one at equal relevance. Save an
external memory with verification requested and a sound content claim; it is
admitted as verified.

**Acceptance Scenarios**:

1. **Given** a save with external provenance and no verification, **When** it is stored, **Then** its trust standing is untrusted and recall results label it as such.
2. **Given** a save with external provenance and verification requested, **When** the independent verification pass supports the content, **Then** it is admitted as verified; **When** verification refutes it, **Then** the save is rejected with the refutation's findings.
3. **Given** an untrusted and a trusted memory of equal relevance, **When** both match a recall, **Then** the trusted one ranks higher.

---

### User Story 3 - The capability is opt-in and inherits every core guarantee (Priority: P3)

Memory requires a second provider credential (for the semantic index). An
operator who has not configured it gets exactly the server they had before:
the existing two correctives, no memory tools in the catalog, no new network
connections. An operator who has configured it gets the same guarantees the
existing tools established — declared structures, the same distinct failure
classes (plus a distinct class for the new provider), one invocation record
per call, and a `forget` operation that permanently removes a memory by
identity (the privacy/compliance lever).

**Why this priority**: Off-by-default is constitutional for new egress; parity
is what keeps the tool surface trustworthy as it grows.

**Independent Test**: Without the credential: catalog shows only the existing
tools and the full existing test suite passes unchanged. With it: memory tools
appear; induced failures surface distinct classes; every call leaves one
record; forget removes a memory and subsequent recalls never return it.

**Acceptance Scenarios**:

1. **Given** no memory-provider credential, **When** the server starts, **Then** the catalog contains no memory tools and no connection to the memory provider is ever attempted.
2. **Given** the credential is configured, **When** the catalog is listed, **Then** save, recall, and forget appear with declared input/output structures.
3. **Given** a memory-provider outage, **When** save or recall is invoked, **Then** the error names the failure class distinctly and one record is written.
4. **Given** a forget for an existing memory id, **When** it completes, **Then** the memory never appears in any later recall; **Given** an unknown id, **Then** forget reports it distinctly as not found.

---

### Edge Cases

- Empty or whitespace-only content or query: rejected as invalid input before any provider call.
- Oversized content (beyond the configured input bound): rejected, never silently trimmed.
- Duplicate saves of near-identical content: both stored (no silent merging in this feature); recall returns the highest-ranked.
- Recall when the store is empty: empty result, success outcome.
- The memory store growing large: recall stays bounded (top-k with a maximum) and ranked — never a dump of everything.
- Memory-provider credential present but invalid: surfaces as the provider failure class on first use; startup is not blocked (the core correctives must keep working).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The server MUST expose a save tool accepting content (required), kind (one of: skill, lesson, fact), provenance (required: a stated origin and whether it is first-hand or external), optional tags, and an optional request for verification at save time.
- **FR-002**: The server MUST expose a recall tool accepting a natural-language query (required), an optional kind filter, and an optional result limit (bounded by a server-side maximum), returning ranked memories each carrying: identity, content, kind, provenance, trust standing, age, and relevance score.
- **FR-003**: The server MUST expose a forget tool that permanently removes a memory by identity; forgotten memories MUST never appear in subsequent recalls.
- **FR-004**: Recall ranking MUST combine semantic relevance to the query, recency, and trust standing — relevance dominating, recency breaking near-ties, and untrusted memories never outranking trusted ones of comparable relevance.
- **FR-005**: A save with external provenance MUST NOT be admitted as trusted without an independent verification pass at save time. Verification that supports the content admits it as verified; refutation rejects the save and surfaces the findings. External saves without verification are stored as untrusted and labeled as such at recall.
- **FR-006**: A save with first-hand provenance MUST record the stated origin and be admitted with first-hand trust standing without a verification pass.
- **FR-007**: The memory capability MUST be enabled only when its provider credential is configured; absent it, no memory tools appear in the catalog and no connection to the memory provider is ever made (off by default).
- **FR-008**: Memory tools MUST carry the same constrained-output guarantee as existing tools (declared flat, closed structures; local validation) and surface the same distinct failure classes, extended with a distinct class for memory-provider failures.
- **FR-009**: Every save, recall, and forget invocation MUST produce exactly one invocation record identifying the tool, with the established fields.
- **FR-010**: Input validation MUST reject empty content/query and oversized input before any provider call.
- **FR-011**: Memories MUST persist across server restarts and sessions in the existing data store; enabling memory MUST NOT alter the behavior of the existing correctives.
- **FR-012**: Recall MUST be read-only with respect to the store (no silent rewriting, merging, or decay-driven deletion in this feature).

### Key Entities

- **Memory**: identity, content, kind (skill | lesson | fact), provenance (origin statement + first-hand/external), trust standing (first-hand | verified | untrusted), tags, created time, and a semantic index entry.
- **Recall result**: the ranked list of memories with per-memory relevance scores.
- **Provenance**: where the knowledge came from — the field the poisoning defense pivots on.
- **Invocation record**: unchanged; `tool` now also takes the three memory tool identities.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On an acceptance set of at least 12 saved memories and 10 recall queries phrased differently from the saved wording, the intended memory ranks in the top 3 for at least 9 of 10 queries, and top 1 for at least 7 of 10.
- **SC-002**: 100% of save/recall/forget results conform to their declared output structures across the acceptance run.
- **SC-003**: In the trust scenarios, 100% of unverified external saves surface as untrusted at recall, 0 unverified external memories are ever labeled trusted, and a refuted external save is rejected with findings.
- **SC-004**: A recall completes in under 5 seconds and a save without verification in under 10 seconds at default settings.
- **SC-005**: With the memory credential absent, the complete pre-existing test suite passes unchanged and the catalog contains exactly the pre-existing tools.
- **SC-006**: 100% of memory-tool invocations (successes and failures) leave exactly one correctly attributed invocation record.
- **SC-007**: After a forget, 0 subsequent recalls return the forgotten memory (verified across restarts).

## Assumptions

- **Pull-only in this feature.** The corpus is explicit that manual-only recall is what killed the prior server's memory tool (0 uses), and that push (auto-surfacing relevant memories unprompted) is the fix — but push requires the watchdog/event integration that does not exist yet. This feature ships the durable store, the trust model, and the pull tools with strong routing descriptions; push lands with the watchdog layer. Named deferral, not an oversight.
- **Single shared store per database** (the existing data store location). This is a single-operator dev tool; per-user isolation is the multi-tenant question and is out of scope.
- **No importance scoring, reflection, consolidation, merging, or decay in this feature.** The canonical read-path score includes an importance term and the write path includes consolidation; both require LLM passes per write and are deferred until usage data justifies their cost. Ranking here is relevance + recency + trust.
- **Verification at save time reuses the existing verification corrective** (same ensemble, same calibrated standard) applied to the memory's content as a claim. Content that is not claim-like (e.g. a workflow) can still be saved external-untrusted or first-hand.
- **The memory provider credential is a new, separate secret**; its absence is the off switch. Costs of embedding calls are recorded on invocation records like model calls.
- **Forget is hard deletion** (the compliance lever), not archival.
