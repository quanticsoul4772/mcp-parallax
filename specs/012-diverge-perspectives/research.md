# Research: Diverge — Independent Perspectives

Phase 0 decisions. The clarification settled dedup (deterministic, server-side) and
per-pass output (one perspective). These resolve the mechanism against the existing
`verify` mode and the mode registry.

## D1 — The divergence lens set

**Decision**: a small fixed array of named divergence lenses, each a one-paragraph
directive that pushes a pass off the anchored frame in a specific direction. Initial
set (5):

- **invert** — flip the goal: what if the opposite of the stated aim were the point?
- **actor** — change whose problem this is: a different stakeholder/role/user sees it how?
- **horizon** — shift the time scale: what does this look like at 10× shorter or longer?
- **assumption** — name and deny the load-bearing assumption the framing rests on.
- **class** — reframe the problem category: what *kind* of problem is this, really —
  is it actually a different class than assumed?

**Rationale**: these are *generative* lenses (open the space), the divergence counterpart
to `verify`'s *critical* lenses (test the claim). They mirror `verify`'s
fixed-`LENSES`-array pattern exactly (`src/modes/verify.rs`), so the orchestration is
identical — only the directives differ. Honors the corpus's "diverse lenses, not N
identical critics" (`NEW_SERVER_DESIGN.md`).

**Alternatives**: model-generated lenses per problem (rejected — non-determinism and an
extra hop, same reasoning as verify D1); temperature variation only (rejected — it is
exactly the in-context "reconsider" failure Diverge targets).

## D2 — Lens ↔ `k` assignment

**Decision**: pass *i* gets `LENSES[i % LENSES.len()]` — identical to `verify` (D2 there).
The server knows each pass's lens by its index, so the **model never echoes the lens**;
the server labels each returned perspective with the lens it assigned (FR-003).

**Rationale**: deterministic, `k`-independent, no config surface. Reuses the exact
assignment `verify::run` already uses.

## D3 — Per-pass constrained-output schema (flat + closed)

**Decision**: each pass emits **one perspective** as two flat string fields:

- `framing`: the one-line reframing of the problem under this lens.
- `implication`: what this framing changes / its key consequence.

The lens is **not** a model field (the server assigns and labels it, D2). The schema is
flat + closed (two strings), satisfying Constitution II — nothing for the sanitizer to
strip, same shape class as `verify`'s `PassVerdict`.

**Rationale**: the clarification pinned one-perspective-per-pass, so the per-pass schema
stays flat (no array of perspectives). Divergence comes from distinct *lenses across
passes*, not multiple framings per pass.

## D4 — Deterministic dedup predicate

**Decision**: the server deduplicates the completed passes' perspectives with a
**deterministic token-similarity** rule over the normalized `framing` text:

1. Normalize: lowercase, strip punctuation, collapse whitespace, to a token set.
2. Two perspectives are near-identical when their token-set **Jaccard similarity ≥ 0.8**
   (a tunable constant); the **lower-index** (earlier-lens) perspective is kept, the
   later dropped.
3. Iterate in pass order so the result is stable and order-independent of HashMap.

**Rationale**: `verify` dedups findings by *exact* string equality, which is too strict
for "near-identical" framings worded differently. A token-set Jaccard threshold is the
lightest deterministic rule that catches reworded restatements without a model hop
(clarification: deterministic, server-side), and it is fully unit-testable on constructed
perspective sets. The 0.8 constant is the v1 default; it lives as a named constant, not a
config var (Scope discipline).

**Alternatives**: exact normalized equality (rejected — misses reworded near-duplicates,
the main case); embedding-similarity dedup (rejected — needs the embedder, a network hop,
and is non-deterministic — violates the clarification and Principle V); a model dedup pass
(rejected by the clarification).

## D5 — Aggregation: collect, do not vote

**Decision**: Diverge does **not** reuse `verify::aggregate_core` — there is no verdict,
no majority, no confidence. Its aggregation: collect every completed pass's perspective,
label each with its lens, run the D4 dedup, and return the set. If **zero** passes
complete, return the dominant failure class (reusing `verify::dominant_failure`-style
logic); otherwise return whatever completed, deduplicated. **No quorum** — a scatter tool
has no minority/majority to protect.

**Rationale**: `verify` converges (majority verdict + agreement confidence); Diverge
scatters (a set of distinct framings). Forcing the verdict math onto it would be wrong.
The only shared machinery is the ensemble orchestration (k parallel passes) and the
constrained-output contract, not the verdict aggregation.

## D6 — Testing: mechanism offline, real divergence live

**Decision**: offline tests (`cargo test`) cover the **mechanism** — the `k` lens prompts
are pairwise distinct, the per-pass schema is flat + closed, the dedup collapses
constructed near-identical sets and keeps distinct ones, stance-blindness is structural
(only problem + context slots), and zero-completion returns the dominant failure. The
**headline SC-001/SC-003** — that real problems actually scatter into ≥3 distinct framings
and that a stated stance does not narrow the set — are **live-model** properties (a
wiremock returns canned perspectives and cannot diverge), confirmed by a **live dogfood**,
exactly as `verify`'s SC-001 was (010).

**Rationale**: a mock cannot demonstrate genuine divergence; the mechanism is
offline-provable, the emergent property needs the running model. Noted so
`/speckit-implement` does not chase a wiremock proof of SC-001.
