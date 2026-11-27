# Implementation Plan: Jev turn signals (068)

**Branch**: `068-jev-turn-signals` | **Spec**: [spec.md](spec.md)

## 1. Summary

`pushback_reversal` and `unverified_fact` become Jev questions asked at each non-continuation turn end, shadow only, recorded in `jev_records` (roadmap §6.2). 066's code detectors, `CHECKPOINT_TURN_SIGNALS`, `turn_signal_records` and its report code are removed (spec Clarification A). 066's transcript parser, catalog-only recommendations, signal-enum binding and the U2 review skip stay.

## 2. Technical context

- Rust, the pinned toolchain; the full gate as CLAUDE.md states it.
- Seams used: `Classifier` (`src/traits/classifier.rs`), `Redactor` (`src/jev/redact.rs`), `Storage` (`src/traits/storage.rs`), `TrajectoryReader`.
- Kept from 066: `src/checkpoint/turn_view.rs` (`parse_turn`, `TurnData`; it imports only the flag prefixes and depends on nothing D5 removes; its one caller today, `src/checkpoint/signals.rs`, is removed, and the new turn-signal shadow becomes its caller; checked 2026-11-27, analyze finding C3), the recommendation rule, the enum binding (`src/checkpoint/contract.rs`), the review skip (`src/jev/policy/skip.rs`).
- Reused from 064: `claim_exclusion` (`src/jev/exclusion.rs`: any digit, a number word, a month, a weekday or a relative date word).

## 3. Decisions

### D1. State sent to Jev

Two requests, each with its own state (decide-scored options; chosen 2026-11-27, replacing the earlier one-shared-state choice after analyze finding E1 showed the stripping hid number reversals from `pushback_reversal`):

The data is the roadmap's §8 U1 row: "the above plus same-turn user text", where "the above" is the U2 row, "turn activity summary, transcript-only pairs, final assistant text" (decide-scored options; chosen 2026-11-27, replacing tool inputs and outputs after the roadmap check, items 28 and 37):

- **`pushback_reversal`**: the turn activity summary (`review::turn_activity`), the transcript-only statement pairs, the final assistant text, and the same-turn user text only with `PARALLAX_JEV_SEND_USER_TEXT=true`. Without the opt-in it sees no pushback text.
- **`unverified_fact`**: the turn activity summary, the transcript-only statement pairs, and the final assistant text with its date- and number-bearing claims (D2) taken out; never user text (D9). FR-004 and SC-003 hold as written.

One `jev_records` row per request, so two rows per covered turn end. This departs from §3.3 ("A site's questions go in one request"): one shared state would put the user text and the date and number claims in `unverified_fact`'s input, kept out only by instructions, against §6.2 and D9 (decide-scored options with every fact in the input; chosen 2026-11-27; roadmap check item 2). Cap: the final assistant text is cut to 2000 characters, as 064's turn state does (`FINAL_MESSAGE_EXCERPT_CHARS`); the row's `truncated` flag records a cut. (Analyze finding U3 chose 2000 characters per tool entry; tool entries are no longer sent, so the same length applies to the final text.)

### D0. Question wording

The roadmap names the two questions and gives no text or criteria (§6.2). The wording in `contracts/questions.md` was drafted from `WATCHDOG_LAYER.md`'s cited rows (decide-scored options; chosen 2026-11-27; roadmap check item 21).

### D1a. When the request runs

Off the Stop hook's response path, never awaited, as 067's tool-risk request: spawned by the long-running server, deadline `JEV_TURN_DEADLINE_MS` (1000 ms), one attempt; its row is written when it ends. No time is added to the hook (decide-scored options; chosen 2026-11-27; analyze finding U1).

### D2. Claims with dates or numbers

The final message is split into claims (sentences); each claim that `claim_exclusion` excludes as `Number` or `Date` is kept out of `unverified_fact` in code and counted (§6.2 "Exclusions in code"). A tested rule table, as 064's.

### D2a. Number and date differences in code (§3.2)

Before the `pushback_reversal` request, code compares the numbers and dates in the final text with those in the earlier statement of each transcript pair and records any difference; Jev is still asked (decide-scored options; chosen 2026-11-27; roadmap check item 1). Recorded as an optional `number_diff` field (true or false) on the `pushback_reversal` `questions_json` entry, with a contract update and no migration (decide-scored options; chosen 2026-11-27).

### D3. Would-be cooldown suppression

A new nullable `jev_records` column, `cooldown_suppressed`: the signal kinds whose yes answer would have been suppressed by the cooldown, as a JSON array; null on other sites. A migration and a contract update (decide-scored options; chosen 2026-11-27). The identity is session plus signal kind; the window is the checkpoint layer's `COOLDOWN_WINDOW_MS` (spec Clarification C). For this record only, a yes is a probability of 0.5 or more; promotion's threshold (D11) replaces it (decide-scored options; chosen 2026-11-27).

### D4. Downstream-cost estimate

The back-test report computes, for each signal, the cost of the continuations and correctives its yes answers would have triggered, from the rows and the per-corrective costs in existing invocation records; no new column (spec Clarification D). The estimate is reported at three cutoffs, p ≥ 0.5, 0.7 and 0.9, so no yes is fixed before promotion. At each cutoff, a yes is left out when the same session had a yes for the same kind at that cutoff within `COOLDOWN_WINDOW_MS` before it, recomputed from the rows rather than read from `cooldown_suppressed`. Correctives are priced at the mean `cost_usd` of that tool's invocation records; continuations are counted with no dollar figure, because Parallax records no cost for the harness model's turn (decide-scored options; chosen 2026-11-27).

### D5. Removal list

Code: `src/checkpoint/signals.rs` (the detectors; the recommendation rule moves to the new site), `src/jev/entities.rs` (the `unverified_fact` entity lookup), `src/storage/turn_signals.rs`, the `turn_signal_records` migration and methods in `src/storage/sqlite.rs`, `src/storage/mod.rs` and `src/traits/storage.rs`, `src/jev/report/signals.rs`, the `CHECKPOINT_TURN_SIGNALS` parsing in `src/config.rs`, its `--help` line in `src/main.rs`, the wiring in `src/server.rs` and `src/checkpoint/run.rs`, `src/checkpoint/run/tests/turn_signals.rs`, `examples/acceptance_turn_signals.rs` and its `Cargo.toml` entry, the turn-signal parts of `examples/jev_backtest/offline.rs`, and the fixtures' `turn_signals: None` fields (`tests/common/mod.rs`, `examples/common/mod.rs`, the acceptance examples). Docs: `README.md`, `CLAUDE.md`, `CHANGELOG.md` (a removal entry), `docs/design/WATCHDOG_LAYER.md` (the 066 amendment), `docs/design/NEW_SERVER_DESIGN.md`. 066's own spec files stay as the record of what 066 built.

### D6. Configuration

`PARALLAX_JEV_MODE_TURN_SIGNALS` (`off` | `shadow`), off in code, `shadow` needs `TYPESAFE_API_KEY`; `PARALLAX_JEV_SEND_USER_TEXT` (`true` | `false`), false unless set, moved from the refused-names list to the known set. `CHECKPOINT_TURN_SIGNALS` is removed.

### D7. Records

One `jev_records` row per request, site `turn_signals`, linked by `eval_id` to the checkpoint record; `state_json` null, `state_hash` plus transcript coordinates (§5.1); excluded claims counted. Each `questions_json` entry gains an optional `recommends` field: the corrective 066's rule names for that signal given the catalog at the time (`verify`; `grounded_verify`, else `research`, else none), with a contract update and no migration (decide-scored options; chosen 2026-11-27; analyze finding U2). `label_json` is null on turn-signal rows: no label call is made in 068, so §8's destinations and §4.1's "fewer end-of-turn big-model calls" hold; labels come later, by hand (§5.4) (decide-scored options; chosen 2026-11-27, replacing analyze finding U4's big-model label after roadmap check items 6, 12 and 35).

Additions to §5.1's record shape: `recommends` and `number_diff` on `questions_json` entries, and the `cooldown_suppressed` column; `truncated` (064) and `label_cost_usd` (067, null here) already exist (decide-scored options; chosen 2026-11-27; roadmap check item 16).

### D7a. Checkpoint cost and telemetry

As 067: when a turn-signal row is written, its Jev cost is added to the turn's checkpoint `jev_cost_usd` (which may already hold the turn-review Jev cost), and a `parallax.jev.turn_signals` span is exported per row. The telemetry contract gains the span and notes that the turn span's `parallax.jev.cost_usd` covers the turn-review request only, since the turn-signal cost is added after that span is exported (decide-scored options; chosen 2026-11-27; analyze finding C1). This and D1a depart from §5.1 ("attributes … on the existing checkpoint and invocation spans. Possible because Jev now runs inline"): running inline would add up to 1 s on turn ends without a review (decide-scored options; chosen 2026-11-27; roadmap check items 17-19).

### D8. What leaves the machine

| Destination | Data | When |
|---|---|---|
| TypeSafe (Jev) | the D1 state, redacted | site in shadow |
| TypeSafe (Jev) | that turn's user text, redacted | site in shadow and `PARALLAX_JEV_SEND_USER_TEXT=true` |

### D9. Stance-exception amendment

In both `docs/design/WATCHDOG_LAYER.md` (replacing the 066 amendment's turn-signal sentences) and `docs/design/NEW_SERVER_DESIGN.md` beside the judge-bias contract (decide-scored options; chosen 2026-11-27), this text: "2026-11-27 amendment (068). Stance exception: the turn-signal question `pushback_reversal` sees the user's prompt from that turn, which breaks the rule that judges stay blind to the user's stance. A reversal after pushback cannot be judged without the pushback. The exception covers that one question and that turn's user text only, redacted, and sent only with `PARALLAX_JEV_SEND_USER_TEXT=true`; no other Jev question sees user text." The roadmap says "in the 066 amendment"; 068 writes a new dated amendment because the 066 amendment's turn-signal sentences describe the removed detectors (decide-scored options; chosen 2026-11-27; roadmap check item 22).

### D10. Tests (§11's 066 list, U1 part)

Parser fixtures, template-derived exclusions and the window-eviction regression (kept from 066; the window tests are `read_returns_the_raw_tail_lines_beside_an_unchanged_window` and `oversize_transcript_truncates_to_the_tail_window` in `src/traits/trajectory.rs`, which the D5 removal does not touch; checked 2026-11-27, analyze finding C2). Neither of those checks signals, so 068 adds the roadmap's regression test: window eviction (`WINDOW_ENTRIES` = 200) doesn't change existing signals. The parser stays in `turn_view.rs`, not the roadmap's `trajectory.rs` (decide-scored options; chosen 2026-11-27; roadmap check item 24); catalog check (the "U1 middle band records only" test is deferred to promotion: no bands without a noise measurement; roadmap check item 3); session-plus-kind cooldown record; continuation turns never flag; no user text on the wire without the opt-in; signal-enum binding; the D2 rule table; live example.

### D10a. Additions to the roadmap's text

The yes/no questions declare their criteria `true` then `false` (as 066's version-2 questions; §3.7 covers Choice questions only); continuation turns make no request as well as never flagging (§11 says "never flag"); and tests the roadmap doesn't list: the D2 rule table, the two wire states with the cut and `truncated`, checkpoint cost and the span per row, the `cooldown_suppressed` migration and binding, the cost estimate (decide-scored options; chosen 2026-11-27; roadmap check items 4, 40, 41).

### D11. Deferred to promotion

U1's positive class (§7.1 defines none for U1), hand labels (§3.8, §5.4: a stratified sample of at least 60 the owner labels or approves), the noise measurement and band edges (§3.4), and thresholds (§7.7) come with promotion; 068's rows are the sampling pool (decide-scored options; chosen 2026-11-27; roadmap check items 3, 5, 33, 34).

## 4. Project structure

- **Added**: `src/jev/shadow/signals.rs` (the turn-signal shadow), its tests, `examples/acceptance_jev_signals.rs`.
- **Changed**: `src/traits/classifier.rs` (`JevSite::TurnSignals`), `src/config.rs`, `src/main.rs`, `src/jev/questions.rs`, `src/checkpoint/run.rs`, `src/server.rs`, `src/jev/report/`, the record contract, README, CLAUDE.md, CHANGELOG, the amendments.
- **Removed**: the D5 list.

## 5. Process

Test-first for every change; the full gate before each commit; an independent review of the diff; the live example; the PR; after merge, the §4.1 setup change with the exact keys shown first.
