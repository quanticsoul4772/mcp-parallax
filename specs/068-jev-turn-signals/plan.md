# Implementation Plan: Jev turn signals (068)

**Branch**: `068-jev-turn-signals` | **Spec**: [spec.md](spec.md)

## 1. Summary

`pushback_reversal` and `unverified_fact` become Jev questions asked at each non-continuation turn end, shadow only, recorded in `jev_records` (roadmap §6.2). 066's code detectors, `CHECKPOINT_TURN_SIGNALS`, `turn_signal_records` and its report code are removed (spec Clarification A). 066's transcript parser, catalog-only recommendations, signal-enum binding and the U2 review skip stay.

## 2. Technical context

- Rust, the pinned toolchain; the full gate as CLAUDE.md states it.
- Seams used: `Classifier` (`src/traits/classifier.rs`), `Redactor` (`src/jev/redact.rs`), `Storage` (`src/traits/storage.rs`), `TrajectoryReader`.
- Kept from 066: `src/checkpoint/turn_view.rs` (`parse_turn`, `TurnData`), the recommendation rule, the enum binding (`src/checkpoint/contract.rs`), the review skip (`src/jev/policy/skip.rs`).
- Reused from 064: `claim_exclusion` (`src/jev/exclusion.rs`: any digit, a number word, a month, a weekday or a relative date word).

## 3. Decisions

### D1. State sent to Jev

One request with both questions and one state: the previous final message, the final message, the turn's tool inputs and outputs (redacted, capped), and the turn's user prompt text only with `PARALLAX_JEV_SEND_USER_TEXT=true` (decide-scored options; chosen 2026-11-27). Without the opt-in, `pushback_reversal` sees no pushback text. Before sending, the final message's date- and number-bearing claims (D2) are taken out of the shared state, so FR-004 and SC-003 hold as written (decide-scored options; chosen 2026-11-27).

### D2. Claims with dates or numbers

The final message is split into claims (sentences); each claim that `claim_exclusion` excludes as `Number` or `Date` is kept out of `unverified_fact` in code and counted (§6.2 "Exclusions in code"). A tested rule table, as 064's.

### D3. Would-be cooldown suppression

A new nullable `jev_records` column, `cooldown_suppressed`: the signal kinds whose yes answer would have been suppressed by the cooldown, as a JSON array; null on other sites. A migration and a contract update (decide-scored options; chosen 2026-11-27). The identity is session plus signal kind; the window is the checkpoint layer's `COOLDOWN_WINDOW_MS` (spec Clarification C).

### D4. Downstream-cost estimate

The back-test report computes, for each signal, the cost of the continuations and correctives its yes answers would have triggered, from the rows and the per-corrective costs in existing invocation records; no new column (spec Clarification D).

### D5. Removal list

Code: `src/checkpoint/signals.rs` (the detectors; the recommendation rule moves to the new site), `src/jev/entities.rs` (the `unverified_fact` entity lookup), `src/storage/turn_signals.rs`, the `turn_signal_records` migration and methods in `src/storage/sqlite.rs`, `src/storage/mod.rs` and `src/traits/storage.rs`, `src/jev/report/signals.rs`, the `CHECKPOINT_TURN_SIGNALS` parsing in `src/config.rs`, its `--help` line in `src/main.rs`, the wiring in `src/server.rs` and `src/checkpoint/run.rs`, `src/checkpoint/run/tests/turn_signals.rs`, `examples/acceptance_turn_signals.rs` and its `Cargo.toml` entry, the turn-signal parts of `examples/jev_backtest/offline.rs`, and the fixtures' `turn_signals: None` fields (`tests/common/mod.rs`, `examples/common/mod.rs`, the acceptance examples). Docs: `README.md`, `CLAUDE.md`, `CHANGELOG.md` (a removal entry), `docs/design/WATCHDOG_LAYER.md` (the 066 amendment), `docs/design/NEW_SERVER_DESIGN.md`. 066's own spec files stay as the record of what 066 built.

### D6. Configuration

`PARALLAX_JEV_MODE_TURN_SIGNALS` (`off` | `shadow`), off in code, `shadow` needs `TYPESAFE_API_KEY`; `PARALLAX_JEV_SEND_USER_TEXT` (`true` | `false`), false unless set, moved from the refused-names list to the known set. `CHECKPOINT_TURN_SIGNALS` is removed.

### D7. Records

One `jev_records` row per request, site `turn_signals`, linked by `eval_id` to the checkpoint record; `state_json` null, `state_hash` plus transcript coordinates (§5.1); excluded claims counted.

### D8. What leaves the machine

| Destination | Data | When |
|---|---|---|
| TypeSafe (Jev) | the D1 state, redacted | site in shadow |
| TypeSafe (Jev) | that turn's user text, redacted | site in shadow and `PARALLAX_JEV_SEND_USER_TEXT=true` |

### D9. Stance-exception amendment

In both `docs/design/WATCHDOG_LAYER.md` (replacing the 066 amendment's turn-signal sentences) and `docs/design/NEW_SERVER_DESIGN.md` beside the judge-bias contract (decide-scored options; chosen 2026-11-27), this text: "2026-11-27 amendment (068). Stance exception: the turn-signal question `pushback_reversal` sees the user's prompt from that turn, which breaks the rule that judges stay blind to the user's stance. A reversal after pushback cannot be judged without the pushback. The exception covers that one question and that turn's user text only, redacted, and sent only with `PARALLAX_JEV_SEND_USER_TEXT=true`; no other Jev question sees user text."

### D10. Tests (§11's 066 list, U1 part)

Parser fixtures, template-derived exclusions and the window-eviction regression (kept from 066); U1 middle band records only; catalog check; session-plus-kind cooldown record; continuation turns never flag; no user text on the wire without the opt-in; signal-enum binding; the D2 rule table; live example.

## 4. Project structure

- **Added**: `src/jev/shadow/signals.rs` (the turn-signal shadow), its tests, `examples/acceptance_jev_signals.rs`.
- **Changed**: `src/traits/classifier.rs` (`JevSite::TurnSignals`), `src/config.rs`, `src/main.rs`, `src/jev/questions.rs`, `src/checkpoint/run.rs`, `src/server.rs`, `src/jev/report/`, the record contract, README, CLAUDE.md, CHANGELOG, the amendments.
- **Removed**: the D5 list.

## 5. Process

Test-first for every change; the full gate before each commit; an independent review of the diff; the live example; the PR; after merge, the §4.1 setup change with the exact keys shown first.
