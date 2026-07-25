# Quickstart: Research Confidence Aggregation

**Feature**: 021 | **Date**: 2026-07-25

## What changes for a caller

Before — a factually correct answer, every claim supported at ~0.78:

```jsonc
{ "answer": "…correct…", "confidence": 0,          // asserts certainty of falsehood
  "gaps": ["…", "…", "…", "…", "…", "…", "…"] }
```

After:

```jsonc
{ "answer": "…correct…",
  "confidence": 0.78,        // support for what the answer asserts
  "coverage": 0.0,           // breadth: no sub-question settled
  "refutation_rate": 0.0,    // none of what was checked fell over
  "sub_question_status": [ { "sub_question": "which version is current?", "settled": false } ],
  "gaps": ["…"] }            // shape unchanged
```

The prior figure is still available: `confidence * coverage` = 0.

## Verifying the fix locally

Nothing here needs network or credentials — the trait seams cover the model client,
fetcher, search provider and clock.

```bash
cargo test research                 # pure arithmetic + pipeline
cargo test --test integration research
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test
```

## The cases that must hold

Each maps to a spec success criterion or edge case.

| Scenario | Expectation |
|---|---|
| Every sub-question targeted by a gap, claims supported | `confidence > 0`, `coverage == 0` — **the regression case** (SC-001) |
| Two runs, same support, different settled counts | identical `confidence`, differing `coverage` (SC-002) |
| Any run | `coverage` == fraction of `sub_question_status` with `settled` (SC-003) |
| No claim supported | `confidence == 0` (SC-004) |
| Two runs, same surviving support, different refuted proportion | identical `confidence`, differing `refutation_rate` (SC-006) |
| Scope produced no sub-questions | `coverage == 1.0`, no division by zero (FR-007) |
| No claim verified | `refutation_rate == 0.0`, no division by zero |
| Several gaps target one sub-question | that sub-question unsettled **once** (FR-004) |
| Gap target out of range | discarded; gap still published; run does not fail (FR-006) |
| Gap target `0` | published; `coverage` unaffected (FR-009) |
| `gap_targets` length ≠ `gaps` length | synthesis retry, then the existing demotion path |

## Live check after building

The observed failure is reproducible with any question whose authoritative answer
sits on one canonical page:

```text
research(question: "What is the current stable release of the Rust compiler,
                    and what were the headline changes in it?", depth: "quick")
```

Before: `confidence: 0` with a correct answer. After: non-zero `confidence`, with
`coverage` carrying whatever breadth the run achieved.

## Reviewing the change

Per the constitution's Development Workflow, this touches the tool surface and a
schema, so before merge it needs the `design-reviewer` agent against the design
corpus, and `code-reviewer` for Rust conventions.

Corpus files amended in this same change (Principle I, spec FR-012):
`docs/design/RESEARCH_PRIMITIVE.md` and `specs/004-research-layer/` — the contract
JSON, `research.md` D-decisions, and `data-model.md`.
