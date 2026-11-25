# Feature Specification: Jev foundation and shadow answers on turn review and research claims

**Feature Branch**: `064-jev-first-pass`

**Created**: 2026-11-24

**Status**: Draft

**Input**: plan `~/.claude/plans/warm-kindling-shell.md` §4-§5 (approved 2026-11-24).

## Clarifications

### Session 2026-11-24

- Q: Which of the plan's §5.4 measurements belong in 064? → A: The ones that need no labeling time from the user (noise, self-agreement, option order, base rate, injection back-test); hand-labeled correctness moves to 065/066.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Jev answers beside the big model (Priority: P1)

With `PARALLAX_JEV_MODE_TURN_REVIEW=shadow` and `PARALLAX_JEV_MODE_RESEARCH_CLAIMS=shadow` set, every end-of-turn review and every verified research claim also gets Jev's typed answers, recorded beside the big model's own answer for the same item; every tool's output is unchanged.

**Independent Test**: with a stand-in Jev, one Jev record per reviewed turn and per claim request, linked to its checkpoint or invocation record; outputs equal the Jev-off run except latency fields.

---

### User Story 2 - Nothing leaves unredacted and no memories are sent (Priority: P1)

Every request passes one mandatory redactor; stored memories never go to Jev.

**Independent Test**: secret fixtures and recalled memory text never appear in captured request bodies.

---

### User Story 3 - The records support measurement (Priority: P2)

Records hold Jev's answers, the big model's labels and enough provenance for the plan's §7 promotion rule to be computed by the harness.

**Independent Test**: the harness computes the §7 tables from stored records alone.

### Edge Cases

- **No key**: no calls, one startup line; a mode set without a key is a startup error.
- **Bad key**: a 401 disables Jev for the process, with one log line.
- **Rate limit / overload / server error**: no retry in shadow, recorded.
- **Malformed answer or model mismatch**: treated as missing, recorded as `validation_failure`.
- **Truncated state**: recorded, no decision.
- **Deadline**: Jev never delays a review or a research run beyond its budget.
- **Excluded claims** (numbers, dates, versions, 063-flagged sources): never sent, counted.
- **Review not running**: no Jev call.
- **Review failing open**: row with no label.
- **Cancelled research run**: rows still written.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001 Client**: a TypeSafe client behind a mockable `Classifier` seam; `POST /v1/systemone` with bearer auth (header marked sensitive); model pinned to `jev-1.14.0` and the answering model read from the response; at most 4 requests in flight per process; named deadlines (turn 1000 ms; claim 2000 ms capped by the run's remaining time and never longer than the claim's verify); no retries in shadow; a 401 disables Jev for the process; error bodies never stored or logged; tests inject the base URL through a constructor, with no environment variable for it.
- **FR-002 Validator**: an answer is rejected unless probabilities are in [0,1], a choice is a declared option, every option has a probability summing to 1 within 0.01, question ids and count match the request, and the answering model equals the pinned one. Rejected or missing answers never pass a threshold.
- **FR-003 Redactor**: the client accepts only redacted state. Redaction removes exact values of every secret environment variable the process can see, `key=value` secrets, `-p` and `-u` credential forms, URL user info, Authorization headers, connection strings, PEM blocks, JWTs and high-entropy runs of 20+ characters; on redactor error nothing is sent; the count is recorded.
- **FR-004 Config**: `TYPESAFE_API_KEY` (blank counts as absent); `PARALLAX_JEV_MODE_TURN_REVIEW` and `PARALLAX_JEV_MODE_RESEARCH_CLAIMS`, values `off|shadow`, default `off`; unknown suffix, bad value, or a mode without a key is a startup error naming the variable; `PARALLAX_JEV_MODEL` optional, default `jev-1.14.0`; a startup line per enabled site names the destination host and data class; `TYPESAFE_API_KEY` is removed from the claude_cli child environment.
- **FR-005 Records**: one `jev_records` row per request: session ids, site, mode, per-question answers with version and option order, models requested and answered, outcome (bound to the outcome list; new outcome `classifier_provider`), request id, latency, tokens, cost, links (checkpoint record id, invocation id, claim key, source ids), enumerated label fields and their provenance, research-only redacted state for replay, redaction count. Jev cost is recorded as `jev_cost_usd` on checkpoint and invocation records, separately from model tokens.
- **FR-006 Turn review shadow**: when the end-of-turn review runs, Jev is asked `contradiction` and `capture_worthy` concurrently with it under the turn deadline, on redacted state (turn activity summary, transcript-only statement pairs with their "observed between" field, final assistant text); memory-derived pairs are never sent; the review returns its raw answers so they are stored as the label; the checkpoint record and the Jev row are written by the same record step, the Jev row second; a failed Jev write is logged and never affects the turn; `capture_worthy` is asked only when memory is configured; truncation is flagged and makes no decision.
- **FR-007 Research claim shadow**: for each verified claim, Jev is asked `relation` (options in the order `says_nothing`, `contradicts`, `supports`) and `same_subject` per source, concurrently with the verify passes, never longer than them; claims with digits, month or weekday names, date words, version strings, or a 063-flagged source are excluded in code and counted; answers and the claim's final label and vote counts are collected during the run and written in one transaction when the call ends, including on cancellation; Jev tokens stay out of the research token ceiling.
- **FR-008 Measurement**: a tested metrics module computes the plan's §7 rule (bounds, baselines, clustering, promotion decision with reasons); a back-test example uses it; committed fixtures are synthetic or scrubbed with provenance markers and a secret scan; real records stay out of git.
- **FR-009 Shadow only**: no Jev answer changes any decision, call or output; every row has `applied=false`.
- **FR-010 Setup**: after merge and rebuild, with the user's approval of exact keys, the project's Stop hook is installed and both sites are set to shadow in the user's Parallax settings (only those keys change; backup first).
- **FR-011 Measurements**: using the harness, 064 runs and records: (a) Jev noise: 5 calls on each of 100 held-out items per question, reporting the within-item spread and the decision flip rate; (b) big-model self-agreement: the turn review and the verify ensemble each re-run twice on about 100 items, reporting how often each agrees with itself; (c) option-order bias: each held-out choice item under all 6 option orders, reporting the decision flip rate; (d) base rate: the share of turn reviews that flagged or captured, from existing checkpoint records, with the read-only query approved first; (e) injection back-test: the `directs_ai` question on `jev-1.14.0` against InjecAgent, BIPIA and at least 300 real research pages, with datasets downloaded into an isolated folder. Results are saved in claudedocs.

### Key Entities

- **Jev record**: one Jev request about one turn review or one research claim, with its answers, the big model's label for the same item, provenance, cost and links.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For identical inputs, `CheckpointResult` (minus `latency_ms`), the checkpoint record (minus `id` and `latency_ms`), `ResearchResult` (minus `stats.elapsed_ms`), the research prompt log, `stats.tokens` and model usage (minus the Jev entry) are identical with Jev off and on, for a confident, erroring, rejected and never-returning Jev - shown by tests.
- **SC-002**: A Jev that never returns still lets every turn review and research run complete within their existing deadlines - shown by tests; live, Jev time on the critical path is reported per run.
- **SC-003**: No secret fixture and no memory text appears in any captured request body - shown by tests on the whole serialized body.
- **SC-004**: A live run with both sites in shadow produces at least 5 turn-review rows and at least 10 research-claim rows, which all join to their records and carry a bound outcome; every row Jev answered carries the pinned answering model; none is applied; an excluded claim leaves only an `excluded` row with no request - asserted by the live example, which exits non-zero on failure.
- **SC-005**: From stored records alone, the harness prints the §7 tables for both sites - shown on the live run's records.
- **SC-006**: The five FR-011 measurements are saved with their numbers, and the harness reproduces them from saved inputs.

## Assumptions

- The operator's TypeSafe key is in the environment.
- TypeSafe's terms (no training on inputs; retention as long as reasonably necessary; telemetry unrestricted; zero retention enterprise-only) are acceptable for redacted turn text and public excerpts.
- The agent research reports are saved to `claudedocs/jev-research-2026-11-24-agents.md` as the source for Jev facts.
- Out of scope: acting on any Jev answer (065, 066), turn signals (066), tool calls (067), memory and other roadmap uses, retries, a circuit breaker, hand-labeled correctness (065, 066).
