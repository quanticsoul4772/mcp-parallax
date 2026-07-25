# Implementation Plan: Research Confidence Aggregation

**Branch**: `021-research-confidence-aggregation` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/021-research-confidence-aggregation/spec.md`

## Summary

`research` reports overall confidence as `mean(finding_confidences) * coverage`,
where coverage is derived by subtracting the length of the model's free-form gap list
from the count of scoped sub-questions. The two lists have no correspondence, and the
gap cap exceeds the sub-question cap, so the term reaches exactly zero by
construction — observed twice on live runs whose answers were correct and whose
per-claim support was ~0.78.

The fix separates the two quantities the number was folding together. The synthesis
hop gains an index-aligned `gap_targets` array saying which sub-question each gap
concerns; the server counts unclaimed sub-questions from it and publishes `coverage`,
a per-sub-question `settled` status, and a `refutation_rate` as their own fields.
`confidence` keeps the findings' mean support alone. Nothing about verification,
support labelling, or per-claim confidence changes.

Four design questions were settled ahead of this plan, each by a `decide` pass whose
recommendation was then put through a `verify` confirmation pass — two of the four
recommendations were overturned by their confirmation. They are recorded in
[spec.md](spec.md) with their rejected alternatives; [research.md](research.md)
resolves the technical unknowns that carrying them out raises.

## Technical Context

**Language/Version**: Rust, MSRV 1.94 (pinned via `rust-toolchain.toml`)

**Primary Dependencies**: unchanged — no crate added, removed, or bumped. `serde`,
`schemars` (mode schemas), `serde_json`.

**Storage**: N/A for this feature. Invocation records are written as today; no
schema or migration change.

**Testing**: `cargo test` — pure unit tests in `src/research/verdict.rs`, pipeline
tests through the `ModelClient` / `Fetcher` / `SearchProvider` / `TimeProvider`
seams in `src/research/pipeline_tests.rs`, and end-to-end via `tests/integration.rs`
against `wiremock`. No network, no credentials, no disk.

**Target Platform**: MCP server over stdio; platform-agnostic.

**Project Type**: Single Rust library plus binary.

**Performance Goals**: N/A — the change adds counting over at most 10 gaps and 7
sub-questions per run. No new model hop, no new network call, no measurable cost.

**Constraints**: Mode schemas must stay flat and closed (Principle II), which is why
the synthesis addition is parallel arrays rather than objects. Production paths carry
no `unwrap`/`expect`, and clippy `pedantic` + `nursery` are denied — relevant here
because the arithmetic involves integer-to-float casts that need explicit
`#[allow(clippy::cast_precision_loss)]` with the existing justification style.

**Scale/Scope**: ≤ 7 sub-questions and ≤ 10 gaps per run. Six files in
`src/research/` plus `tests/integration.rs`, none new; ~4 corpus/contract files
amended.

## Constitution Check

*GATE: evaluated before Phase 0, re-evaluated after Phase 1. Both passes below.*

| Principle | Verdict | Basis |
|---|---|---|
| **I. Design-Corpus Fidelity** | PASS | `RESEARCH_PRIMITIVE.md` and `specs/004-research-layer/` are amended in this same change (FR-012). The feature *is* a correction to a corpus-described formula, so leaving the corpus stating the old one would be the drift this principle forbids. No crate change, no dropped layer. |
| **II. Constrained-Output Contract** | PASS | `SynthOut` gains `gap_targets: Vec<u32>` and stays flat and closed. Nesting was rejected for this reason ([research.md](research.md) D1), as was a delimiter convention inside the gap text, which is free-text parsing by another name. The published tool contract may nest — it already does — because it is server-assembled, not model-constrained. |
| **III. Compiler-Enforced Discipline** | PASS | No new production `unwrap`/`expect`; no stdout write. An arity mismatch feeds the retry rather than being absorbed — [research.md](research.md) D2 rejects the "treat absent targets as none" reading precisely because it would be a fallback hiding a failure, and it would inflate coverage to full. On a second failure it demotes under its own `StopReason::MalformedSynthesis`, so the caller is not told a grounding gate rejected the answer when that gate was never reached (004 FR-007, honest accounting). |
| **IV. Seams, Composition, Tests** | PASS | No new external effect and no new seam. Every case in [quickstart.md](quickstart.md) is reachable through the existing mocks; the boundary rows (no sub-questions, no claims verified, out-of-range target) are pure-function tests. Tests are required, not optional. |
| **V. Deterministic Over Probabilistic** | PASS | Coverage and refutation rate are counted from run data by pure functions. The model supplies only which sub-question a gap it wrote concerns — it does not supply, or influence, either figure. The alternative of inferring the association by text matching was refuted 3/3 and is explicitly out of scope. |
| **VI. Capabilities Off By Default** | PASS | No new capability, no egress, no execution, no env var. Nothing to gate. |
| **VII. Simplicity and Scope Discipline** | PASS | No new module: two pure functions join `verdict.rs` (189 lines, well under the 500-line target). Gap-cap-aware truncation was considered and deliberately **not** built ([research.md](research.md) D6) — the caps make it near-unreachable and the spec does not ask for it. |

**Pre-Phase-0**: PASS, no violations.
**Post-Phase-1**: PASS, unchanged. The design added no dependency, no module, no
seam, and no capability, so no gate moved. **Complexity Tracking is empty by
consequence, not by omission.**

### One residual, named rather than papered over

The design accepts two ways the model can misreport, and design review found the
plan had named only one.

**In range but wrong**: a key pointing at a sub-question the gap does not concern is
indistinguishable from a correct one at the server. FR-006 covers out-of-range; no
requirement can close this without inferring the association from text, which was
refuted. *Narrowed after review*: the synthesis prompt now presents the
sub-questions **numbered** rather than bulleted, matching what `decide` does for the
options it asks the model to index. That removes the largest source of these — the
model miscounting unlabelled lines — though it does not eliminate the class.

**Omission**: a synthesis that reports no gaps at all receives `coverage: 1.0` with
no evidence anything was settled. Coverage rewards silence, and nothing in FR-003
through FR-009 constrains under-reporting. This is the weaker direction of the same
trust, and it was unnamed in the original plan.

Both are bounded by the same structural fact: coverage no longer reaches
`confidence`, so a mis-keyed or omitted gap cannot inflate the support figure — only
the breadth figure, which is published beside the list it is computed from.

Decision 1 took this trust knowingly: the status quo already trusted the same
synthesis pass for the same number, with less structure and no bound at all. Both
residuals are recorded here so they survive into review rather than living only in
the checklist notes.

## Project Structure

### Documentation (this feature)

```text
specs/021-research-confidence-aggregation/
├── plan.md                       # This file
├── spec.md                       # Feature spec (4 settled decisions recorded)
├── research.md                   # Phase 0 — 6 technical decisions
├── data-model.md                 # Phase 1 — schema deltas + boundary table
├── quickstart.md                 # Phase 1 — before/after, cases, review path
├── contracts/
│   └── output-contract.md        # Phase 1 — delta to the 004 tool contract
├── checklists/
│   └── requirements.md           # Spec quality checklist (all pass)
└── tasks.md                      # Phase 2 — NOT created here
```

### Source Code (repository root)

```text
src/research/
├── prompts.rs      # SynthOut gains gap_targets; SYNTH_PROMPT_TEMPLATE explains the keying
├── synthesis.rs    # arity + range validation; carries targets out; demotion path unchanged
├── verdict.rs      # overall_confidence loses its coverage arg; coverage() and
│                   #   refutation_rate() added — the pure arithmetic, unit-tested
├── contract.rs     # ResearchResult gains coverage, refutation_rate, sub_question_status
├── pipeline.rs     # settled-status assembly replaces the length subtraction at :355
└── pipeline_tests.rs  # seam-level cases

tests/
└── integration.rs  # end-to-end through the MCP surface against wiremock

specs/004-research-layer/          # amended: contract JSON, research.md, data-model.md
docs/design/RESEARCH_PRIMITIVE.md  # amended: the confidence/coverage description
CHANGELOG.md                       # Unreleased: Fixed + Changed (contract redefinition)
```

**Structure Decision**: Single Rust project, existing layout, no new module. The
change is confined to the research layer's own files plus the corpus and contract
they are described by. `verdict.rs` receives the new arithmetic because that is where
this tool's server-assembled arithmetic already lives and where it is already
unit-tested; `pipeline.rs` (647 lines, already carrying a `too_many_lines` allowance)
receives only the assembly wiring.

## Phase 2 preview — what `/speckit-tasks` will decompose

Not authoritative; recorded so the shape is visible before the task list exists.

1. **Pure arithmetic first** — `coverage()`, `refutation_rate()`, and
   `overall_confidence()` minus its coverage parameter, with the boundary table from
   [data-model.md](data-model.md) as tests. Independently verifiable, no pipeline.
2. **Synthesis hop** — `gap_targets` on `SynthOut`, prompt text explaining the
   keying, arity and range validation into the existing retry/demotion path.
3. **Assembly and contract** — settled-status construction in `pipeline.rs`, the
   three new `ResearchResult` fields, the contract JSON amendment.
4. **Regression and end-to-end** — the observed-run case (SC-001) asserted against
   the exact arithmetic that collapsed, plus the SC-002/003/004/006 relations.
5. **Corpus amendment** — Principle I, in this same change, not a follow-up.

## Complexity Tracking

No Constitution Check violations. No entries.
