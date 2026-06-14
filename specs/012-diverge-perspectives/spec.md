# Feature Specification: Diverge — Independent Perspectives

**Feature Branch**: `012-diverge-perspectives`

**Created**: 2026-06-14

**Status**: Draft

**Input**: User description: "Diverge — an independent-perspectives corrective for
anchoring and tunnel vision. When the calling model is locked onto one framing of a
problem, asking it to 'reconsider' in the same context just yields more confident
commitment to that frame. Diverge runs k stance-blind passes that each attack the
problem from a deliberately distinct vantage (invert the goal, change the
actor/stakeholder, shift the time horizon, attack the load-bearing assumption, reframe
the problem class) so genuinely different solution directions surface, not N
restatements of the anchored one. It returns a small set of distinct, named
perspectives — each a one-line framing plus its key implication — deduplicated by the
server, with the originating lens labeled. The divergence half of the corpus's 'use
diverse lenses, not N identical critics' mandate, reusing verify's stance-blind ensemble
machinery: the model writes only the per-pass perspective under a flat constrained-output
schema; the server assembles the deduplicated set. No new capability gate. Distinct from
unstick (commit to one next step) and verify (judge a claim true/false)."

## Clarifications

### Session 2026-06-14

- Q: How should Diverge deduplicate near-identical perspectives across passes? → A:
  **Deterministic, server-side** — the server collapses near-identical framings with
  deterministic logic (no extra model call), mirroring verify's server-assembled finding
  dedup and the deterministic-over-probabilistic principle. No model dedup/synthesis hop.
- Q: How many perspectives may each stance-blind pass emit? → A: **One perspective per
  pass** — each pass returns exactly one framing under its lens; the per-pass schema stays
  flat (like verify), and the returned set is the k passes deduplicated (≤ k distinct).

## User Scenarios & Testing *(mandatory)*

Parallax exists to catch the ways the calling model reliably goes wrong from inside its
own context. **Anchoring / tunnel vision** is one of those: once the model has committed
to a framing of a problem, more thinking in the same context deepens the commitment
rather than breaking it — "reconsider" produces a more confident version of the same
answer. The model cannot diverge from a frame it cannot see it is in. `Diverge` is the
external pass that supplies the missing vantage points: it returns a handful of genuinely
distinct framings of the problem so the caller can see the option space it had collapsed.

It is the **complement** of the existing correctives, not a duplicate: `verify` judges
whether a claim is true; `unstick` commits to one next step when looping; `Diverge`
*opens up* the set of framings when the caller is locked onto one. Where `verify`
converges (is this right?), `Diverge` deliberately scatters (what else could this be?).

### User Story 1 - Distinct framings instead of N restatements (Priority: P1)

A caller is stuck on one framing of a problem and asks `Diverge` for other ways to see
it. Today the model, asked in-context to "think of alternatives," tends to return
variations on the framing it already holds. `Diverge` runs `k` stance-blind passes, each
assigned a **distinct divergence lens** (e.g. invert the goal, change whose problem it
is, shift the time horizon, attack the load-bearing assumption, reframe the problem
class), so each pass is pushed off the anchored frame in a different direction. The
server collects the per-pass perspectives, **deduplicates** near-identical ones, and
returns a small set of distinct, named framings — each a one-line reframing plus its key
implication or what it would change — with the lens that produced it labeled.

**Why this priority**: This is the entire value of the tool — turning one frame into
several genuinely different ones. It is the divergence half of the corpus's
"diverse lenses, not N identical critics" mandate (`NEW_SERVER_DESIGN.md`), the
counterpart to the convergent `verify`.

**Independent Test**: Submit a problem with an obvious dominant framing and confirm the
returned perspectives are **materially distinct** from each other and from the submitted
framing (different lenses, not reworded restatements), each labeled with its lens.
Testable through the tool's output alone.

**Acceptance Scenarios**:

1. **Given** a problem stated under one clear framing, **When** `Diverge` runs at `k`
   passes, **Then** it returns multiple perspectives that differ from one another and
   from the input framing, each tagged with the divergence lens that produced it.
2. **Given** two passes that land on near-identical framings, **When** the server
   assembles the result, **Then** the duplicates are collapsed to a single perspective
   (no padded list of restatements).
3. **Given** a problem with genuinely few distinct angles, **When** `Diverge` runs,
   **Then** it returns the smaller honest set rather than manufacturing distinct-sounding
   but hollow framings.

### User Story 2 - Stance-blind, like the rest of the family (Priority: P1)

A caller submits a problem along with their own leaning ("I think we should just rewrite
it"). `Diverge` must not anchor on the caller's stance — that would defeat the point, since
the caller's framing is exactly what the tool exists to break out of. Each pass sees only
the problem statement and optional neutral context, never the caller's preferred answer or
conversational history, so the perspectives are generated on the problem's merits.

**Why this priority**: P1 because stance-blindness is the structural property that makes
the divergence real — the same property `verify`/`grounded_verify` rely on. Without it,
`Diverge` would just elaborate the caller's existing frame more confidently, reproducing
the failure it targets.

**Independent Test**: Submit a problem with a stated preferred framing in the context and
confirm the returned perspectives still include framings that depart from (and may
contradict) the stated preference — the caller's leaning does not dominate the set.

**Acceptance Scenarios**:

1. **Given** a problem whose context asserts a preferred framing, **When** `Diverge`
   runs, **Then** the returned set still contains perspectives that depart from that
   preference — the stance does not collapse the divergence.
2. **Given** the same problem with and without the stated preference, **When** `Diverge`
   runs both, **Then** the set of framings is not materially narrowed by the presence of
   the stance.

### Edge Cases

- **A trivial or already-narrow problem** with one honest framing: `Diverge` returns the
  small true set (possibly a single perspective), not a padded list — over-divergence is
  itself a failure (manufacturing fake angles).
- **Passes converge despite distinct lenses** (the problem genuinely has one dominant
  reading): dedup collapses them; the output honestly reflects low divergence rather than
  forcing variety.
- **More lenses requested than the lens set holds**, or fewer: how lenses map to `k` is a
  planning decision (mirrors how `verify` assigns its lens set).
- **Empty / oversize problem statement**: rejected before any model call, like the rest of
  the family (no silent trimming).
- **A caller using `Diverge` to settle a factual question**: out of scope — `Diverge`
  opens framings, it does not judge truth (route to `verify`) or commit to a step (route
  to `unstick`). The output is framings, never a verdict or a chosen action.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `Diverge` MUST accept a problem statement (and optional neutral context) and
  return a set of distinct **perspectives**, each a short reframing of the problem plus
  its key implication or what it would change.
- **FR-002**: `Diverge` MUST run **k stance-blind passes**, each assigned a **distinct
  divergence lens**, so the passes are pushed off the anchored framing in different
  directions (not one prompt replicated `k` times). Each pass emits **exactly one
  perspective** under its lens, keeping the per-pass output schema flat (clarification).
- **FR-003**: Each returned perspective MUST be **labeled with the divergence lens** that
  produced it, so the caller can see why each framing differs.
- **FR-004**: The server MUST **deduplicate** near-identical perspectives across passes
  using **deterministic, server-side** logic (no model dedup/synthesis hop), returning a
  set of materially distinct framings rather than reworded restatements (clarification).
- **FR-005**: `Diverge` MUST be **stance-blind**: a pass sees only the problem statement
  and optional neutral context — never the caller's preferred framing, stance, identity,
  or conversation history (blindness is structural, as in `verify`).
- **FR-006**: `Diverge` MUST NOT manufacture hollow distinct-sounding framings to hit a
  count: when the problem honestly has few angles, it returns the smaller true set
  (no over-divergence).
- **FR-007**: `Diverge` MUST return **framings only** — never a truth verdict (that is
  `verify`) and never a single committed next step (that is `unstick`). Its output shape
  is a set of perspectives, server-assembled.
- **FR-008**: Input validation MUST reject an empty/whitespace or oversize problem
  statement before any model call (consistent with the family; no silent trimming).
- **FR-009**: `Diverge` MUST be **always in the catalog** — no new capability gate or env
  flag (like `verify` and `unstick`).

### Key Entities

- **Problem statement**: the framing the caller is anchored on, stated neutrally — the
  primary input a pass sees.
- **Divergence lens**: a named directive that pushes a pass off the anchored frame in a
  specific direction (e.g. invert-the-goal, change-the-actor, shift-the-horizon,
  attack-the-assumption, reframe-the-class). The set and the lens↔`k` assignment are a
  planning decision.
- **Perspective**: one pass's single output — a distinct reframing plus its key
  implication, carrying the lens that produced it. The server's returned set is the
  deterministic deduplicated union of the k passes' perspectives (≤ k distinct).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a problem with a clear dominant framing, `Diverge` returns **at least 3
  materially distinct** perspectives (distinct lenses, not reworded restatements) — the
  option space the caller had collapsed is reopened.
- **SC-002**: **100%** of returned perspectives are labeled with the divergence lens that
  produced them.
- **SC-003**: When a stated caller preference is added to the context, the set of distinct
  framings returned is **not narrowed** versus the same problem without it (stance does
  not collapse divergence) — measured on a fixed battery.
- **SC-004**: On a deliberately narrow problem (one honest framing), `Diverge` returns the
  small true set without padding — **0** manufactured hollow framings on the battery.
- **SC-005**: `Diverge` returns **only** perspectives — **0** truth verdicts and **0**
  single committed next steps in its output (it never does `verify`'s or `unstick`'s job).

## Assumptions

- `Diverge` is a corrective already named in the design corpus (`NEW_SERVER_DESIGN.md`
  four-layer catalog; `THEORY_OF_MIND.md` perspectives half) — this feature implements an
  existing catalog entry, not a new invention; the constitution's design-corpus-fidelity
  check is an application, not a deviation (confirmed at `/speckit-plan`).
- It reuses the existing stance-blind ensemble machinery (the `k`-pass orchestration and
  the constrained-output contract that `verify`/`grounded_verify` use); the model writes
  only the per-pass perspective under a flat schema, and the server assembles and
  deduplicates the set.
- The concrete divergence lens set and the lens↔`k` assignment rule are **`/speckit-plan`
  decisions** (mirroring how `verify`'s lens set was a planning choice). The dedup
  approach is settled (clarification): **deterministic, server-side**; the dedup
  *predicate* (how near-identical is detected) is the remaining plan detail.
- `VERIFY_ENSEMBLE_K` (or an analogous pass count) governs how many passes run; reusing
  the existing default is assumed unless planning decides otherwise.
- The output is advisory framings for the caller to act on — `Diverge` neither ranks the
  perspectives as better/worse nor selects one (selecting one is `unstick`'s job).
