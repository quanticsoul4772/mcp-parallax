# Research and decisions: 064

Sources:

- the seven research-agent reports of 2026-11-24, saved to `claudedocs/jev-research-2026-11-24-agents.md` in this feature;
- `claudedocs/jev-research-2026-11-24.md`;
- the two review rounds on the approved plan.

Round 1 had three reviewers: design, accuracy, gaps. Round 2 had five new ones (security, calibration, test strategy, systems integration, architecture), plus design and accuracy re-reviews. Review findings are cited as reviewer plus finding number.

## D1. Client behavior

- **Chosen:**
  - `POST /v1/systemone` with `.bearer_auth()`, which marks the header sensitive (as `src/client/voyage.rs:85` does).
  - Model pinned to `jev-1.14.0`; the answering model is read from the response.
  - One shared HTTP client; at most 4 requests in flight per process.
  - Named deadlines: turn 1000 ms; claim 2000 ms, capped by the run's remaining time and never longer than the claim's verify.
  - **No retries in shadow.** 429, 529 and 5xx are recorded as `classifier_provider` and dropped.
  - A 401 disables Jev for the life of the process, with one log line.
  - Error bodies are never stored or logged; only status, error code and request id are kept.
- **Why:**
  - The rate limit is per organization and shared with the user's other Jev projects and with other sessions. Retrying in every process multiplies load exactly when the limit is hit, and could silently switch off exa-mcp-server's injection screen (Security M8; Integration I2–I3).
  - Shadow data doesn't need every answer.
  - A 422 `detail[]` can echo the input back (Security L13).
- **Rejected:**
  - The SDK's 2-retry default: it multiplies load at the limit.
  - Copying Exa's retry rule as-is: Exa retries every transport error (Accuracy-re W4). The helpers are moved to `src/client/retry.rs` for 065's `on` sites instead of being copied a third time.

## D2. Redactor

- **Chosen:**
  - The client accepts only a `RedactedState` newtype, so a request can't bypass redaction.
  - Redaction removes:
    - the exact values of every secret environment variable the process can see (`*_KEY`, `*_TOKEN`, `*_SECRET`, `*PASSWORD*`, and every Parallax provider key);
    - `key=value` pairs whose key names match pass, pwd, secret, token, key, auth, credential, cookie or session;
    - `-p<value>` and `-u user:pass` forms;
    - URL user info;
    - Authorization headers;
    - connection strings;
    - PEM blocks;
    - JWTs;
    - high-entropy runs of 20 or more characters.
  - If the redactor errors, nothing is sent. Redaction counts are recorded.
- **Why:** assistant text often repeats keys, environment values and config. The TypeSafe key itself was once printed into a session transcript. TypeSafe keeps some data indefinitely (Security H2; Architecture I13).
- **Rejected:**
  - Relying on the 055 secret-name check: it judges file names only (Accuracy A2).
  - Redacting at U3 only: turn text needs it too.

## D3. Configuration

- **Chosen:**
  - `PARALLAX_JEV_MODE_TURN_REVIEW` and `PARALLAX_JEV_MODE_RESEARCH_CLAIMS`, values `off|shadow`, default `off`.
  - The suffix set is closed; an unknown suffix is a startup error, modeled on `src/routing.rs:471-483`.
  - A mode without a key is a startup error.
  - `PARALLAX_JEV_MODEL` is optional; the default constant is `jev-1.14.0`.
  - `TYPESAFE_API_KEY` is read once; a blank value counts as absent.
  - `TYPESAFE_API_KEY` is added to the claude_cli child's `ENV_REMOVE`.
  - The user's setup is switched to shadow as FR-010.
- **Why:**
  - The constitution requires outbound features to ship off and be enabled deliberately (`:92-94`).
  - exa-mcp-server reads `JEV_MODEL` and other `JEV_*` names. A shared environment would override Parallax's pin or trip the closed namespace (Design-re B2; Architecture I6; Security M5).
  - Decided: off in code, on in the operator's setup (2026-11-24).
- **Rejected:**
  - "Shadow whenever the key is set": an upgrade would turn on data sending because of a key set for another project.
  - A base-URL environment variable: it could redirect the key (Security M6). Tests use constructor injection instead.

## D4. Outcomes

- **Chosen:** a new outcome, `classifier_provider`, for transport and provider failures, following the `embedding_provider` and `search_provider` precedent.

  | Case | Outcome |
  |---|---|
  | Validator reject, or model mismatch | `validation_failure` |
  | 4xx other than 429 (including 400 `max_tokens_exceeded`) | `provider_rejected` |
  | Redaction failed (`redaction:too_large` / `redaction:pattern`); nothing sent | `invalid_input` |
  | No time left in the run (`no_time_left`); nothing sent | `timeout` |
  | No request slot before the deadline (`no_slot`); nothing sent | `timeout` |
  | Deadline mid-request (`deadline; cost unknown`) | `timeout` |
  | Verify finished first; call abandoned (`verify_finished; cost unknown`) | `timeout` |
  | Refused after an earlier 401 (`disabled_after_401`); nothing sent | `provider_rejected` |
  | 429, 529, 5xx, connect error | `classifier_provider` |
  | Excluded claim | Decision `excluded`, no call |

- **Why:**
  - `Decode` doesn't exist (`src/error.rs:37-69`; Accuracy-re W3).
  - Mapping onto other providers' outcomes would mislabel the failure (Design-re I3; Architecture I4).
  - The 057 binding test forces the contract updates.

## D5. Cost

- **Chosen:** a `jev_cost_usd` field on invocation and checkpoint records, following the `search_cost_usd` pattern, plus a price row for `jev-1.14.0` at $0.042 per million input tokens.
- **Why:** `NON_MODEL_USAGE` usage stays in `usage_by_model` and in `cost_usd`, and is exported to the token metric. Using both routes would count Jev twice (Accuracy-re W1; Architecture I5).
- **Rejected:** the `NON_MODEL_USAGE` route.

## D6. Concurrent with the big model, not detached

- **Chosen:**
  - **Turn review:** Jev runs alongside `review_once` via `tokio::join!` under the turn deadline. `record()` then writes the checkpoint record (with `jev_cost_usd`) and the Jev row.
  - **Research:** Jev runs alongside `verify::run` per claim, inside the claim's permit. Once verify finishes, an unfinished Jev call is abandoned and recorded as `timeout`, so a claim never takes longer with Jev (2026-11-24).
- **Why:**
  - A detached task dies when the process exits (`src/main.rs:132-137`).
  - It also finishes after the checkpoint row and span are written.
  - Its mock assertions can fail without failing the test (Integration B1–B2; Test B1–B2; Architecture B1; Security M9).
  - The review takes seconds, so joining adds about `max(0, jev − review)` ≈ 0.
- **Rejected:**
  - Detached shadow tasks.
  - Running Jev before verify in 064: that adds serial time against the run deadline (Integration I8).

## D7. Records

- **Chosen:**
  - One `jev_records` row per request, with per-question answers in `questions_json`.
  - Labels are stored as enumerated fields only, plus their provenance (model and prompt version).
  - Research rows also store their redacted state for replay. Turn rows store only a hash and the transcript path.
  - Two session columns: the process UUID and the Claude Code session id.
- **Why:**
  - Cost, latency and request id exist per request (Architecture I2).
  - Shadow data must be replayable after wording changes (Architecture I8).
  - Audit data must not hold sensitive text (Security M11).
  - `session_id` means different things in the two existing record types (Integration M1).

## D8. Ids and the research accumulator

- **Chosen:**
  - `run_turn` creates the evaluation id at its start.
  - The invocation id is created in `RecordGuard::new` and passed to `InvocationRecord::create`.
  - Research Jev results are pushed into an `Arc<JevShadow>` accumulator, the `SearchCost` / `NonModelUsage` / `PageCounts` pattern.
  - `RecordGuard::finish` and `Drop` write the rows in one transaction.
- **Why:**
  - Today both ids are created at exit (`run.rs:549-550`; `telemetry.rs:465`, `record.rs:114`, `:155`), so nothing can link to them earlier (Integration I1; Architecture I3; Test I14).
  - One transaction per run reduces SQLite contention (Integration I4).

## D9. Claim exclusion

- **Chosen:**
  - A claim is never sent to Jev, but is recorded as `excluded`, if it contains any digit, a month or weekday name, a date word ("yesterday", "Q3" and similar) or a version string, or if any of its sources was flagged by the 063 screen.
  - The rule is a tested table.
- **Why:**
  - Jev is weak at math, counting and dates; that belongs to code (principle 2).
  - The excluded share is needed to estimate 065's savings (Architecture I12; Test I12).

## D10. Option order

- **Chosen:** the `relation` options are fixed in the order `says_nothing`, `contradicts`, `supports`. Option order is recorded, never rotated at runtime.
- **Why:**
  - Jev leans toward the first option, so "supports" goes last; the bias then leans toward escalating to the ensemble.
  - Runtime rotation cancels the safe ordering and invalidates tuned thresholds (Design D8; Design-re minor; Calibration I10).

## D11. No memories sent

- **Chosen:**
  - `Candidate` gains an origin field, and only transcript-derived pairs are sent.
  - A turn's `contradicts` label is used only when the turn had zero memory-derived candidates.
- **Why:** memory-derived pairs carry stored memory text (`review.rs:192-203`), which the plan keeps behind a later opt-in (Security H1; Design-re B1).

## D12. Stop hook

- **Chosen:** FR-010 installs the project's Stop-to-`checkpoint_turn` hook and sets both sites to shadow in the user's setup, with the user's approval of the exact keys.
- **Why:** the project's `.claude/settings.json` has no Stop entry for `checkpoint_turn`, so turn-review shadow would collect nothing (Integration I7).

## D13. Measurement methods (FR-011)

| Measurement | Method |
|---|---|
| Noise | 5 calls on each of 100 held-out items per question; within-item spread on the logit scale; decision flip rate. Sets the middle-band width in 065/066. |
| Self-agreement | Turn review and verify ensemble each re-run twice on about 100 items. This is the ceiling for any agreement figure. |
| Option order | Each choice item under all 6 orders; paired decisions compared with McNemar's test; flip rate reported. |
| Base rate | Share of turn reviews that flagged or captured, from existing `checkpoint_records`. A read-only query, approved first. Sizes the shadow period. |
| Injection | exa-mcp-server's back-test method ported onto `metrics.rs`, run on `jev-1.14.0` with InjecAgent, BIPIA and at least 300 real research pages. Datasets downloaded into an isolated folder; Python run with `-I`. |

- **Why:** the calibration review (B2, I4, I8, I10, I11), and the choice of the measurements that need no labeling time.
- **Deferred:** hand-labeled correctness moves to 065/066.

## D14. Statistics in `metrics.rs`

- **Chosen:**
  - one-sided Clopper-Pearson lower bounds for recall;
  - Wilson intervals for reported rates;
  - cluster bootstrap by research run or session;
  - thresholds chosen on a tune set by a rule stated in advance, then checked once per question version on a sealed confirmation set, with every look logged;
  - the integer threshold grid limited to observed scores;
  - always-no and always-yes baselines;
  - a pure `promotion_decision()` that returns its reasons.
- **Why:** calibration review B1–B4 and I1–I7, plus test review I9. 30 positives can't support realistic targets (30/30 gives a lower bound of about 0.89).

## D15. Left out

| Left out | Why |
|---|---|
| Circuit breaker | Per-call deadline plus fail-open is enough. The design lists the breaker as removed (`NEW_SERVER_DESIGN.md:328`). Skipped calls would need rows (Design-re I13). |
| Compression | Bodies are a few KB; it needs a new crate (Design D12). |
| Jitter | Shadow doesn't retry, and there is no randomness dependency. |
| Base-URL environment variable | Security M6. |
| `TYPESAFE_API_KEY_FILE` and a daily token cap | Gaps review Y3. |
| `warn` mode | Gaps review Y1. |

## D16. Promotion rule (shadow to on)

Copied verbatim from the approved plan's §7 (`~/.claude/plans/warm-kindling-shell.md`, revision 3), so FR-008, SC-005 and D14 cite text in the repository. Implemented per question in `metrics.rs`.

1. **Positive means the class whose miss causes harm:**
   - for U2: the review flagged or captured;
   - for U4: the ensemble refuted or contested the claim.
2. **Skip gates (U2, U4):**
   - the one-sided 95% Clopper-Pearson lower bound on recall over harmful positives must clear the target;
   - for U4, the rate of Jev-settled claims the ensemble did not support must stay below a stated cap;
   - the skip share's lower bound must be at least a stated minimum.
3. **Flags (U1, U3, U5):**
   - precision is estimated at natural prevalence;
   - the false-positive rate's upper bound is tight, e.g. 0 of 300 negatives flagged;
   - at least 60 flags;
   - a recall floor.
4. **Sample sizes follow from the target.** For a 95% recall target, ≥ 59 positives with 0 misses or ≥ 93 with 1 miss, on the sealed confirmation set. The tune set is separate.
5. **Baselines.** Results must beat always-no and always-yes, with the paired targets above, so no single number can be gamed.
6. **Clustering.** Intervals are bootstrapped by research run or session, and the number of clusters is reported.
7. **Pinning.** Thresholds are pinned to Jev version, big-model id and prompt versions. Any change invalidates promotion and is a startup error for `on`.
8. **After promotion.**
   - **Audit sample:** a random 5–10% of items Jev would skip still go to the big model, weighted.
   - **Drift check:** a rolling Wilson lower bound on recall over the last 60 audited positives. When it falls below the target, the site reverts to shadow, the revert is recorded, and the operator is told.
