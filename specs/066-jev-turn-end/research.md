# Research: Jev at turn end

## D1. Transcript spike

- **Decision**: the first task reads real Claude Code transcripts from the operator's machine (shapes only) and writes scrubbed fixtures for each line kind 066 reads: user prompts (string and text blocks), subagent lines, hook-injected context, interruption markers, meta lines, compaction summaries, tool inputs and results, Stop-hook block reasons. Field names used to tell them apart come from the fixtures, not from memory.
- **Rationale**: the format belongs to Claude Code and is not documented in the repository; three reviewers flagged guessed field names.
- **Alternatives**: hard-code expected field names (rejected: unverified).

## D2. Turn start

- **Decision**: the last line the spike identifies as a real user prompt, skipping every excluded kind and any line containing an exported flag prefix (FR-014). Not found inside the 2 MiB transcript tail → `excluded`, reason `turn start not found`.
- **Rationale**: a wrong start moves which tool output counts as the turn's evidence.

## D3. Turn data module

- **Decision**: `parse_turn(lines) -> TurnData` in `src/checkpoint/turn_view.rs`, called from `run_turn` on the lines the reader already read. `parse_lines`, `TrajectoryEntry` and `TrajectoryWindow` are unchanged. To get the lines, the reader trait gains a method returning the raw tail lines, used only by `run_turn`.
- **Rationale**: no change to the batch path, the 200-entry window, the screens, the review miner or 064's fingerprint.
- **Alternatives**: a field on the window (21 construction sites); a `User` entry (changes every consumer).

## D4. Previous final message

- **Decision**: a per-session map in the server process holding the last `final_message` received at a turn end. Not persisted. Missing → `pushback_reversal` excluded with reason `no previous final message`.
- **Rationale**: exact text from the harness; no stored conversation text.

## D5. pushback_reversal detector

- **Decision**: structure check (a real user prompt between the previous and final messages, no tool call after it), then `review::mine_candidates`-style pairing over the two assistant messages (lexical overlap plus opposing polarity). A candidate pair → Jev is asked, with the two messages only.
- **Rationale**: `WATCHDOG_LAYER.md:151`; judge blindness (`NEW_SERVER_DESIGN.md:81,193,198-200`).

## D6. unverified_fact detector

- **Decision**: `src/jev/entities.rs` extracts entities from the final message (numbers with units, dates, file paths, identifiers such as commit hashes and version strings, capitalized names) and looks each up verbatim in the turn's tool inputs, tool outputs and user prompt, unredacted, on the machine. Claims (sentences) holding an unmatched entity go to Jev as claim text only.
- **Rationale**: `WATCHDOG_LAYER.md:153` names entity diffs; Constitution V.
- **Alternatives**: send tool output to Jev (rejected: new data class off the machine).

## D7. Redaction and caps

- **Decision**: assistant text is redacted whole by 064's redactor, then cut: previous and final messages at 2000 characters each (064's excerpt size), claims at 500 characters each and 4000 in total. `truncated` is set on the row when anything is cut.

## D8. Audit share

- **Decision**: `checkpoint::fnv1a64(eval_id) % 100 < 10` → audit. Tests pick ids on each side.
- **Rationale**: the same function 064 uses for its splits; deterministic and testable.

## D9. Pinned tables

- **Decision**: `TURN_REVIEW_NO_BAND: &[(&str, f64, f64)]` (model, contradiction, capture_worthy) and `TURN_REVIEW_ORDER_CHECKED: &[&str]` (models with a recorded order check) in `src/jev/policy.rs`, both empty. Config refuses `TURN_REVIEW=on` unless the configured model is in both.

## D10. Skip order of work

- **Decision**: while the skip is off, today's `tokio::join!` of review and Jev stays. When on, Jev runs first under the turn deadline; the review starts only if not skipped.

## D11. Records

- **Decision**: one `turn_signals` row per covered turn end: detector results in `label_json` (entities matched/unmatched counts, pair found, recommendations), questions and answers in `questions_json`, `state_hash` of what was sent, `state_json` null.
- **Rationale**: no conversation text stored.

## D12. Live example

- **Decision**: `examples/acceptance_jev_signals.rs` feeds fixture transcripts through `run_turn` with the real TypeSafe client and the site in shadow, asserts one joined row per turn end and no user text or tool output in any captured request, and exits non-zero on failure.
