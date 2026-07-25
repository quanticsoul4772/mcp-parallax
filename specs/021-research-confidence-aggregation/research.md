# Phase 0 Research: Research Confidence Aggregation

**Feature**: 021 | **Date**: 2026-07-25

The two design questions this feature turns on were settled before the spec, each
by a `decide` pass put through a `verify` confirmation pass, and two more were
settled in `/speckit-clarify` the same way. Those four are recorded in
[spec.md](spec.md) and are not re-opened here. What follows resolves the technical
unknowns that carrying them out raises.

## D1 — How keyed gaps are represented in a flat, closed schema

**Decision**: two index-aligned parallel arrays on the synthesis output —
`gaps: Vec<String>` unchanged, plus `gap_targets: Vec<u32>` where entry *i* is the
1-based sub-question that gap *i* concerns, and `0` means it concerns none.

**Rationale**: Constitution Principle II requires every mode schema to be flat and
closed. A gap cannot become an object carrying its own key, even internally — the
grammar the provider is constrained to forbids the nesting. Parallel arrays are the
established idiom for exactly this in this codebase: `decide` emits `option_scores`
and `option_rationales` index-aligned to the caller's options and checks their arity
at assembly. Following it keeps one pattern rather than inventing a second.

The 1-based encoding with `0` as "no sub-question" avoids negative sentinels, which
the schema grammar handles poorly, and gives the unattributed case (a gap raised by
the grounding gate rather than by an unanswered question — spec FR-009) a natural
representation rather than a special case. Out-of-range values are discarded per
FR-006; the encoding makes the check a single bound comparison.

**Alternatives considered**:

- *A single array of objects* — forbidden by Principle II. Not available.
- *A delimiter convention inside the gap string* (`"3: still unclear"`) — this is
  free-text parsing wearing a costume, which Principle II forbids by name, and it
  would corrupt the published gap text.
- *Asking the synthesis for a per-sub-question settled flag instead of keyed gaps* —
  a real option, scored 55 against 85 in the `decide` pass recorded in the spec. It
  has the model assert its own success directly, which is the thing D7 exists to
  avoid. Rejected there, not re-opened here.

## D2 — What happens when the two arrays disagree in length

**Decision**: an arity mismatch is a `ValidationFailure` that feeds the synthesis
pass's **existing** retry, and a second failure takes the **existing** demotion path.

**Rationale**: `synthesize_grounded` already has both — a violation-fed retry and a
demotion that returns an honest answer listing what remains. Adding a second failure
mode to a loop that already exists costs nothing new and behaves consistently with
the grounding failure beside it.

The alternative of accepting the mismatch and treating absent targets as `0` was
rejected: Principle III forbids fallbacks that hide failures, and silently
reinterpreting a malformed response as "no gap concerns any sub-question" would
inflate coverage to full — the server overstating what it established, which is the
specific failure this whole feature exists to correct.

**Alternatives considered**:

- *Enforce equal length in the JSON Schema* — not expressible. Cross-array arity is
  outside what JSON Schema can state, which is precisely why the local validator
  exists and why `decide` checks arity in code.
- *A dedicated retry loop for arity* — more machinery for no gain over the loop
  already there.

## D3 — Where the coverage and refutation figures are computed

**Decision**: two new pure functions in `src/research/verdict.rs`, beside
`overall_confidence`, which itself loses its coverage parameter.

**Rationale**: `verdict.rs` is where this tool's server-assembled arithmetic already
lives, it is 189 lines with room under the 500-line target, and its functions are
pure and directly unit-tested. Putting the new arithmetic anywhere else would split
one concept across two modules. Principle V is satisfied by construction: both
figures are counted from run data, with no model judgment in the computation.

**Alternatives considered**:

- *A new `coverage.rs` module* — Principle VII says build only what the spec asks;
  two small pure functions do not warrant a module, and `verdict.rs` has room.
- *Computing inline in `pipeline.rs`* — that file is 647 lines and already carries
  an `#[allow(clippy::too_many_lines)]` on its spine. Adding untested arithmetic to
  it is the wrong direction.

## D4 — Where the new fields sit in the published output

**Decision**: `coverage`, `refutation_rate`, and the per-sub-question status list are
top-level fields on `ResearchResult`, beside `confidence` — not inside `stats`.

**Rationale**: `stats` holds run mechanics (searches, sources fetched, tokens,
elapsed, stop reason) — what the run *did*. The new fields are quality signals about
what the run *established*, which is what `confidence` is and where it sits. The
clarified decision to publish rather than hide these exists so a caller reads them;
burying a quality signal among counters works against that.

## D5 — How the observed defect's own case is proven fixed

**Decision**: the regression test asserts the exact arithmetic of the two observed
runs — all sub-questions claimed by gaps, per-claim support around 0.78 — and
requires non-zero confidence with zero coverage.

**Rationale**: SC-001 names reproducing the observed runs. A test that only checks
"confidence > 0 sometimes" would pass against several wrong implementations. Pinning
the specific input shape that produced the collapse is what makes the test a
regression test rather than a smoke test.

## D6 — Truncation ordering (spec defect D3)

**Decision**: coverage is derived from every target the synthesis returned, and the
per-sub-question statuses are published, so the caller reconciles against the
statuses rather than against the gap list. Gap text remains best-effort under the
cap.

**Rationale**: the original defect was that a penalty was computed from a list the
caller could not see. Publishing the statuses removes the invisibility, which is the
actual harm, rather than reordering two statements and leaving the figure
unauditable. Reordering alone would keep the caller unable to check the number.

Deliberately **not** doing: making truncation prefer to retain one gap per distinct
sub-question. The schema bounds gaps at 10 and sub-questions at 7, so the cap is
rarely reached, and Principle VII says not to build it until the spec asks.

## Resolved unknowns

No `NEEDS CLARIFICATION` markers remain. Every technical unknown above is resolved
against an existing pattern in the codebase or an explicit constitutional
constraint; none required a new dependency, and the dependency stack is unchanged.
