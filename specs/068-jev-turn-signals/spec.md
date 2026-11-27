# Feature Specification: Jev turn signals (U1 as the roadmap specifies)

**Feature Branch**: `068-jev-turn-signals`

**Created**: 2026-11-27

**Status**: Draft

**Input**: the Jev roadmap (`warm-kindling-shell.md`): §6.2's U1 text ("U1 turn signals, shadow only in 066"), §4.1's 066 row, §11's 066 test list, §8's U1 row, with the shared rules in §3, §5.1 (config, client, records) and §7. This spec carries that text as written. It corrects 066, which built the two signals as code detectors with no Jev call (066 FR-003) instead of the Jev questions §6.2 specifies. Where the roadmap contradicts itself or is silent, the owner's answers are recorded under Clarifications. Nothing else is added.

## Clarifications

### Session 2026-11-27

- **A.** Q: What happens to 066's code detectors, `CHECKPOINT_TURN_SIGNALS` and `turn_signal_records`? → A: Remove all of them: the detectors, the switch, the table and the report code that reads it. The Jev questions replace them (decide-scored options; chosen 2026-11-27).
- **B.** Q: The wording of the `pushback_reversal` and `unverified_fact` questions (§6.2 names them and points to `WATCHDOG_LAYER.md:149,151`, now lines 156 and 158, with no question text). → A: Yes/no questions, version 1, drafted from those rows; the exact text is in `contracts/questions.md` (drafted during specification; decide-scored, chosen 2026-11-27).
- **C.** Q: What does §6.2's "Cooldown. Identity is session plus signal kind" govern while the signals only record? → A: Every turn is still asked; each row records whether a flag for the same session and signal kind would have been suppressed by the cooldown window, so a later promotion can be measured with it (decide-scored options; chosen 2026-11-27).
- **D.** Q: Which estimates does §6.2's "downstream continuation and verify cost" go into, and where? → A: The back-test report: for each signal, the estimated cost of the continuations and correctives its yes answers would have triggered, computed from the rows and the per-corrective costs in existing invocation records; no new column (decide-scored options; chosen 2026-11-27).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The two turn signals are asked of Jev, recorded only (Priority: P1)

At each non-continuation turn end, Jev is asked `pushback_reversal` and `unverified_fact` (§6.2) about the redacted turn. Answers are recorded in `jev_records`; nothing is flagged and nothing the user sees changes (shadow only, §6.2).

**Independent Test**: with `PARALLAX_JEV_MODE_TURN_SIGNALS=shadow`, fixture turns leave one Jev row per request with both answers, and the checkpoint result and record equal the run with the site off.

**Acceptance Scenarios**:

1. **Given** the site in shadow and a turn end with a final message, **When** the turn ends, **Then** one Jev request carries both questions and one `jev_records` row records the answers.
2. **Given** a claim in the final message containing a date or a number, **When** the turn ends, **Then** that claim is kept out of `unverified_fact` in code (§6.2 "Exclusions in code").
3. **Given** a continuation turn, **When** it ends, **Then** nothing flags (§11).
4. **Given** the site off, **When** a turn ends, **Then** no Jev request is made for these signals and no row is written.

### User Story 2 - User text leaves only with the opt-in (Priority: P1)

`pushback_reversal` sees the user's pushback, which breaks judge blindness to the user's stance; the exception is named and justified in an amendment and limited to user text from that turn (§6.2). User text is sent only with `PARALLAX_JEV_SEND_USER_TEXT=true` (§6.2, §8).

**Independent Test**: wire-level tests assert that no user text is in the request body without the opt-in, and that with it only that turn's user text is present.

### User Story 3 - 066's detectors are removed (Priority: P1)

066's code detectors, `CHECKPOINT_TURN_SIGNALS`, `turn_signal_records` and the report code that reads it are removed (Clarification A). 066's transcript parser, catalog-only recommendations, signal-enum binding and the U2 review skip stay unchanged.

**Independent Test**: the build and full gate pass with the detectors, switch, table and report code gone; a repository search finds none of them.

### User Story 4 - On in the owner's setup (Priority: P2)

After merge and rebuild, the owner's setup gets `PARALLAX_JEV_MODE_TURN_SIGNALS=shadow`, and `PARALLAX_JEV_SEND_USER_TEXT=true` only if the owner chooses (§4.1). `CHECKPOINT_TURN_SIGNALS` is removed from the setup.

### Edge Cases

- Redaction fails: nothing is sent; the row records the failure (§3.10, §5.1).
- Jev errors, times out or answers from another model: recorded, never acted on (§3.6).
- The transcript has no real user prompt in the turn: `pushback_reversal` has no pushback to see.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001** (§6.2 Questions): `pushback_reversal` and `unverified_fact` are Jev questions, shadow only, wording per Clarification B, versioned (§3.12).
- **FR-002** (§6.2 Not built): the three signals the model can see itself are not built.
- **FR-003** (§6.2 Stance exception): the exception is named and justified in an amendment, limited to user text from that turn.
- **FR-004** (§6.2 Exclusions in code): claims with dates or numbers are kept out of `unverified_fact`.
- **FR-005** (§6.2 Transcript parser): 066's parser is kept: real user prompts read from string content and user text blocks, excluding tool results, meta lines, compaction summaries and Parallax's own flag text derived from the template functions.
- **FR-006** (§6.2 Recommendations): only correctives in the catalog (`research`, `grounded_verify`) are recommended.
- **FR-007** (§6.2 Cooldown): identity is session plus signal kind (Clarification C).
- **FR-008** (§6.2 Cost): the downstream continuation and verify cost goes into the estimates (Clarification D).
- **FR-009** (§6.2 User text, §8): sent only with `PARALLAX_JEV_SEND_USER_TEXT=true`.
- **FR-010** (§6.2 Contracts): signal-enum binding test for `checkpoint_turn.tool.json`; the stale list at `telemetry.md:89` is checked and fixed if stale.
- **FR-011** (§5.1 config): `PARALLAX_JEV_MODE_TURN_SIGNALS` (`off` | `shadow`), off in code; `shadow` needs the key; unknown suffixes and unparseable values are startup errors; `PARALLAX_JEV_SEND_USER_TEXT` (`true` | `false`), false unless set.
- **FR-012** (§5.1 records): one `jev_records` row per request; enumerated labels only.
- **FR-013** (Clarification A): remove 066's detectors, `CHECKPOINT_TURN_SIGNALS`, `turn_signal_records` and its report code; amend the documents that describe them.
- **FR-014** (§4.1): the last task sets the owner's setup per User Story 4, exact keys shown first.

### Tests required (§11's 066 list, U1 part)

1. Parser fixtures from 066's spike; template-derived exclusions; window-eviction regression.
2. U1 middle band records only.
3. Catalog check.
4. Session-plus-kind cooldown.
5. Continuation turns never flag.
6. No user text on the wire without the opt-in.
7. Signal-enum binding.
8. Live example.

### Key Entities

- **Jev row** (`jev_records`, §5.1): site `turn_signals`, both answers, the redacted state's hash and transcript coordinates, no user text stored.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With the site in shadow, every non-continuation turn end with a final message leaves one Jev row with both answers, and checkpoint results and records equal the site-off run.
- **SC-002**: No request body contains user text without `PARALLAX_JEV_SEND_USER_TEXT=true`.
- **SC-003**: No date- or number-bearing claim reaches `unverified_fact`.
- **SC-004**: 066's detectors, `CHECKPOINT_TURN_SIGNALS`, `turn_signal_records` and its report code are gone, and the full gate passes.
- **SC-005**: The live example asserts SC-001 to SC-003 and exits non-zero on failure.

## Assumptions

- Promotion of U1 to `on` is a later feature under §7; 068 is shadow only (§4, §6.2).
- The U2 review skip from 066 is unchanged.
