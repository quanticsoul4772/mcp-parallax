# Phase 1 Data Model: Research Confidence Aggregation

**Feature**: 021 | **Date**: 2026-07-25

## 1. Synthesis output (internal — `prompts::SynthOut`)

The constrained output of the synthesis model hop. Flat and closed per Principle II.

| Field | Type | Bound | Change |
|---|---|---|---|
| `answer` | string | ≤ 8000 chars | unchanged |
| `gaps` | string[] | ≤ 10 entries, ≤ 500 chars each | unchanged |
| `gap_targets` | u32[] | ≤ 10 entries | **new** |

`gap_targets[i]` is the 1-based position of the sub-question that `gaps[i]` concerns;
`0` means it concerns no single sub-question. The two arrays are index-aligned, the
same contract `decide` uses between `option_scores` and `option_rationales`.

**Validation, at assembly:**

- `gap_targets.len() == gaps.len()`, else `ValidationFailure` → the existing
  synthesis retry, then the existing demotion path (research.md D2).
- Each entry is `0..=sub_questions.len()`. Out-of-range entries are discarded — the
  gap is still published, it simply targets nothing (FR-006).
- Entries are **not** required to be distinct. Several gaps may target one
  sub-question; it counts unsettled once (FR-004). This is the precise arithmetic
  whose absence caused the observed collapse.

## 2. Sub-question status (published — new)

| Field | Type | Notes |
|---|---|---|
| `sub_question` | string | The scoped question, verbatim. Not a position — sub-questions are not otherwise in the output, so an index would reference nothing the caller can see (spec Assumptions). |
| `settled` | bool | True when no retained gap targets it. |

One entry per sub-question the scope phase produced, in scope order. At most 7.
Empty when the run scoped none.

## 3. `ResearchResult` (published — `contract::ResearchResult`)

| Field | Type | Change |
|---|---|---|
| `answer` | string | unchanged |
| `confidence` | f32 0..=1 | **redefined** — mean support of the findings the answer asserts, with no coverage factor |
| `refutation_rate` | f32 0..=1 | **new** — proportion of verified claims that verification refuted |
| `coverage` | f32 0..=1 | **new** — proportion of sub-question statuses marked settled, or 1.0 when none were scoped |
| `sub_question_status` | SubQuestionStatus[] | **new** — the basis for `coverage`, so it is checkable from the output |
| `key_findings` | KeyFinding[] | unchanged |
| `disagreements` | Disagreement[] | unchanged |
| `gaps` | string[] | unchanged — deliberately still plain strings (clarification Q1) |
| `sources` | SourceRef[] | unchanged |
| `stats` | Stats | unchanged |

Three additive fields; one redefinition. The prior combined figure stays recoverable
as `confidence * coverage` (SC-005).

## 4. Arithmetic (pure — `verdict.rs`)

```text
confidence      = mean(finding_confidences)                    // no coverage factor
coverage        = settled_count / sub_question_count           // 1.0 when count == 0
refutation_rate = refuted_count / verified_count               // 0.0 when count == 0
```

`finding_confidences` are the surviving (non-refuted) findings' confidences, exactly
as today — the change is the removal of the multiplier, not of the input set
(clarification Q2).

**Boundary behaviour**, each an FR or edge case:

| Condition | `confidence` | `coverage` | `refutation_rate` |
|---|---|---|---|
| No sub-questions scoped | unaffected | `1.0` (FR-007) | unaffected |
| No claim supported | `0.0` (FR-008) | unaffected | unaffected |
| Every claim refuted | `0.0` | unaffected | `1.0` |
| No claim verified | `0.0` | unaffected | `0.0` |
| Every sub-question targeted | unaffected — **this is the fix** | `0.0` | unaffected |

The last row is the defect: `coverage = 0` must no longer reach `confidence`.

## 5. What is not changing

Verification, the support labels including the two-independent-sources bar for
`confirmed`, per-claim confidence, claim extraction, dedup, the grounding gate, and
the `gaps` wire type. The defect was in aggregation alone; the per-claim confidences
in both observed runs were sound (spec Assumptions).
