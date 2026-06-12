# Data Model: Research Layer (004)

Wire types live in `research/contract.rs` (MCP-side, nested legal — 003 D6
precedent). Internal pipeline types in `research/mod.rs`. Pure functions in
`verdict.rs`/`grounding.rs`. Nothing here persists to SQLite except the
standard invocation record.

## 1. Request (tool input — contract `research.tool.json`)

| field | type | rules |
|---|---|---|
| question | string | required; non-empty after trim; ≤ INPUT_MAX_CHARS (FR-010) |
| depth | "quick" \| "standard" \| "deep" (nullable) | default "standard" |
| focus | [string] (nullable) | ≤ 8 entries, each 1..=200 chars; biases scope angles |
| constraints | object (nullable) | all fields optional, see below |

`constraints`: `max_sources` (1..=60), `domains_allow` ([string]),
`domains_deny` ([string]), `budget_tokens` (≥ 1000), `deadline_ms` (≥ 5000).
Explicit values override tier defaults (FR-006). Domain lists: ≤ 32 entries
each, matched by registrable-domain suffix.

## 2. Response (tool output)

| field | type | notes |
|---|---|---|
| answer | string | synthesis prose with inline `[sN]` citations; model-written |
| confidence | number 0..=1 | server-computed (§4) |
| key_findings | [KeyFinding] | server-assembled from verified claims |
| disagreements | [Disagreement] | contested claims, positions + sources |
| gaps | [string] | unanswered sub-questions + demoted-by-grounding content |
| sources | [Source] | only cited sources survive pruning |
| stats | Stats | honest accounting (FR-007) |

**KeyFinding**: `claim` (string), `support` ("confirmed" \| "contested" \|
"refuted" \| "unverified"), `confidence` (0..=1), `sources` ([string] of ids;
≥ 1 — FR-003). Refuted claims never appear here — they are dropped and
counted; SC-007 (a false premise must not be confirmed) is served by the
refuted list being handed to the synthesis prompt with an explicit
do-not-assert instruction, so the answer can note the refutation.

**Disagreement**: `claim` (string), `positions` ([{stance: string, sources:
[string]}], ≥ 2).

**Source**: `id` ("s1", "s2", … — run-scoped), `url`, `title`, `fetched_at`
(RFC 3339), `credibility` (0..=1, heuristic: domain class + corroboration —
conservative, explainable). Never a body (FR-012).

**Stats** (sources_found counts candidates post URL dedup and *before* the
domain filter, so policy-excluded candidates stay visible in the
accounting): `angles`, `searches`, `sources_found`, `sources_fetched`,
`claims_extracted`, `claims_after_dedup`, `claims_verified`, `claims_dropped`,
`tokens` (summed LLM usage), `elapsed_ms`, `stopped_early` (bool),
`stop_reason` ("budget" \| "deadline" \| "grounding" \| null).

## 3. Model-hop schemas (flat + closed — Principle II)

| call | schema | bound |
|---|---|---|
| scope | `{angles: [string], sub_questions: [string]}` | angles ≤ tier N; sub_questions ≤ 7; `focus` entries are woven into the scope prompt (FR-001) |
| extract (per source) | `{claims: [string]}` | ≤ 12 claims/source; span dropped (research.md D4) |
| verify (per claim) | existing verify schema (`{verdict, findings}`), refute-biased prompt variant (research.md D3) | K passes from tier |
| synthesize | `{answer: string, gaps: [string]}` | answer ≤ 8000 chars, gaps ≤ 10 × 500 chars — enforced by the local validator |

## 4. Pure functions (`verdict.rs`, `grounding.rs`)

- `support(passes, agreement, verdict, n_sources) -> Support` — mapping per
  research.md D7, **order-sensitive**: Contested first (winning-side share of
  passes ≤ 2/3, integer rule `3·majority ≤ 2·completed` — the verify ensemble
  resolves ties to refuted, so contested must be detected before the
  aggregate verdict is trusted), then Refuted (aggregate refuted at share
  > 2/3), then Confirmed (supported, n ≥ 2), then Unverified (supported,
  n = 1).
- `claim_confidence(agreement, n_sources, mean_credibility) -> f32` —
  clamped 0..=1, weights are constants (tuned offline, never at runtime).
- `overall_confidence(findings, settled, total_subqs) -> f32` —
  coverage-weighted mean penalized by unanswered sub-questions.
- `ground(answer, findings, sources) -> Result<Grounded, Violations>` —
  every `[sN]` token resolves; every finding's sources resolve; uncited
  sources pruned. Violations carry exact descriptions for the one retry.

## 5. Internal types (never on the wire)

- **ScopePlan**: angles ([string]), sub_questions ([string]).
- **SearchHit**: url (normalized), title, snippet; deduped by normalized URL
  across angles.
- **FetchedPage**: source id, url, title, readable_text, fetched_at.
- **Claim**: text, source_ids ([String], grows on dedup-merge), normalized
  key (dedup).
- **VerifiedClaim**: Claim + verdict, agreement, votes, Support label,
  confidence.

## 6. Outcome taxonomy extension

New class `search_provider` (`AppError::SearchProvider(String)`), pattern
identical to 003's `embedding_provider`: Brave 4xx terminal, 429/5xx/transport
retried then `retries_exhausted`, per-request timeout → `timeout`. The 001
invocation-record contract enum gains the value (addition). Individual
angle-search failures degrade the run (FR-013); only a fully-failed search
phase fails the invocation.

## 7. Error classes used by `research`

`invalid_input` (FR-010, bad constraints), `search_provider` (above),
`refusal`/`truncation`/`timeout`/`retries_exhausted` (LLM calls, existing),
`validation_failure` (a model hop fails its schema or the grounding retry
also fails *and* nothing is salvageable — when partial content exists, the
run returns success with `stop_reason: "grounding"` instead), `cancelled`.
