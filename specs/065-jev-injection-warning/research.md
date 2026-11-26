# Research: Jev injection warning on research sources

All spec clarifications are answered (spec, Clarifications, 2026-11-25). Each decision below names what was chosen, why, and what else was weighed.

## D1. Where the call happens

- **Decision**: in `src/research/pipeline.rs`, where 063 screens each kept page (`screen_page` on the title and `extract::capped_text`), the Jev call is prepared from the same capped text and joined with that page's `extract_source` future; the source's screening result is attached when the joined result is assembled.
- **Rationale**: per the spec's Clarifications (beside extraction); extraction takes seconds, so Jev adds no time when it answers first; the capped text is what extraction reads and what 063 screens.
- **Alternatives**: before extraction (adds time); after extraction (adds time); full text split into segments (rejected).

## D2. Modes and the first `on`

- **Decision**: `JevMode` gains `On`; `JevSite` gains `ResearchInjection` (suffix `RESEARCH_INJECTION`, record value `research_injection`). Config accepts `on` only for `RESEARCH_INJECTION` and only when `policy::injection_threshold(model)` returns a threshold; `on` for the other sites stays a startup error naming the variable, as 064 FR-004 requires.
- **Rationale**: spec FR-006; plan §5.1 "`on` for a site without a threshold at the pinned version is a startup error naming both variables".
- **Alternatives**: a separate boolean variable (rejected: a second namespace).

## D3. Pinned threshold

- **Decision**: a const table in `src/jev/policy.rs`, `INJECTION_THRESHOLDS: &[("jev-1.14.0", 0.35)]`, bound by a test to the measurements doc's value; a decision is `warned` when the validated probability is `>= threshold`.
- **Rationale**: plan §7.7 pins thresholds to the Jev version; the 064 back-test measured 0.35 on `jev-1.14.0`.

## D4. Question

- **Decision**: `directs_ai`, `QuestionKind::Noul`, version 1, with options `("true", criteria true)` and `("false", criteria false)` so the client sends them as criteria (PR #170); the wording is the earlier back-test's, verbatim, the same text `jev_backtest injection` sent. It moves from the back-test's question file into `src/jev/questions.rs`; a test binds the two.
- **Rationale**: FR-001; one source of the wording.

## D5. Deadline and retries

- **Decision**: per call, a deadline of `min(2000 ms, run time left)` measured from the first attempt. Shadow makes one attempt. `on` retries 429, 529 and 5xx at most three times; each wait is the larger of the backoff step (100, 200, then 400 ms) and the response's `Retry-After` (`client::retry::retry_after`), and no wait runs past the deadline, which it is compared with rather than added to (065 review findings 1 and 2). The client applies this rule itself when the request's `retry` flag is set, as the Exa client does; the flag is false for shadow and the 064 sites, and error bodies are still never logged (064 FR-001 unchanged).
- **Rationale**: per the spec's Clarifications (2000 ms, 064's claim deadline); plan §6.1 "retries via the shared helpers are enabled for `on` sites only".
- **Alternatives**: no retries (rejected by the plan); jitter (no randomness dependency, as in 064).

## D6. What the result shows

- **Decision**: with the site `on`, each source gains `jev_injection: { screened, probability, warned, reason }` and stats gain `jev_warned_pages` (count) and `jev_unscreened_pages` (count); both are omitted (`skip_serializing_if`) when the site is off or in shadow, so those results are byte-identical to today's. 063's `injection_patterns`, `pages_flagged` and `flagged_pages` are untouched.
- **Rationale**: per the spec's Clarifications (leave them out); SC-001 and SC-002.

## D7. Records

- **Decision**: one `jev_records` row per page request, `site = research_injection`, linked to the invocation id and the source id, collected in the run's existing `JevShadow` accumulator and written with the other Jev rows when the call ends (cancellation included); `label_json = { "patterns063": [names] }`; `applied = true` only for an `on` row whose decision was `warned`. Jev cost joins the invocation's `jev_cost_usd`. No new column.
- **Rationale**: per the spec's Clarifications (063 names as the label); 064's accumulator pattern; FR-008.

## D8. Docs and wording

- **Decision**: the research tool description gains one sentence: the Jev warning is advisory, "no warning" does not mean clean, and the measured rates (recall 0.86; 10 of 413 clean pages warned, 9 about prompt injection). `RESEARCH_PRIMITIVE.md` and the README env table are amended; CHANGELOG `[Unreleased]` gets an entry.
- **Rationale**: FR-005; CLAUDE.md conventions.

## D9. Telemetry

- **Decision**: following 064, the research invocation span gains `parallax.jev.injection.warned` and `parallax.jev.injection.unscreened` counts (numeric only); `telemetry.md` is updated.
- **Rationale**: the gap clarify left for the plan; 064's numeric-only `parallax.jev.*` attributes.

## D10. Live acceptance

- **Decision**: `examples/acceptance_jev_injection.rs` runs one research question with the site `on` against the live services and asserts SC-005: one row per kept page, all joined, answered rows carry `jev-1.14.0`, every kept page shows screened or a reason; it exits non-zero on failure. Run after merge and rebuild, as in 064 T037.
