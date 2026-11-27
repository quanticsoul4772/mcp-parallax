# Research: Jev at turn end

## D1. Transcript spike

- **Decision**: the first task reads real Claude Code transcripts on the operator's machine (shapes only) and writes scrubbed fixtures for each line kind 066 reads. Field names used to tell the kinds apart come from these fixtures. Scrubbing removes user names, paths containing an account name, session ids and real prompt text; a test checks the committed fixtures for them.
- **Rationale**: the transcript format belongs to Claude Code and is not documented in the repository.
- **Spike result (T001, 2026-11-26)**: shapes counted across the local transcripts; no content kept. Fixtures: `tests/fixtures/transcripts/` (`turn_string_prompt`, `turn_block_prompt`, `turn_slash_command`, `turn_compacted`).

| Line kind | How it is told apart | Observed |
|---|---|---|
| Real user prompt | `type: "user"`, `origin.kind: "human"`, no `tool_result` block, not `isMeta`, `isCompactSummary` or `isSidechain` | string content; text blocks only with an image block; slash commands typed by the user also carry `origin.kind: "human"` |
| Tool input | `type: "assistant"`, `message.content[].type: "tool_use"`, field `input` | |
| Tool result | `type: "user"` with a `toolUseResult` key and `message.content[].type: "tool_result"` | `content` is a string, a list of `text` blocks, or a list of `tool_reference` blocks; `is_error` absent, false or true |
| Subagent line | `isSidechain: true` | in `subagents/` files, not the main transcript |
| Hook-injected context | `type: "attachment"`, `attachment.type` `hook_additional_context` or `hook_success`, with `hookEvent` | never a `user` line |
| Interruption marker | `type: "user"`, content a `text` block starting `[Request interrupted` | no `origin` |
| Meta line | `isMeta: true` | string or text-block content; `origin.kind` absent or `plugin` |
| Compaction summary | `isCompactSummary: true` (with `isVisibleInTranscriptOnly`); preceded by `type: "system"`, `subtype: "compact_boundary"` | may carry `turnOrigin: "human"` |
| Other user-role lines | `origin.kind` `task-notification` or `peer`, or no `origin`: `turnOrigin` `scheduled` or `sdk`, `<bash-input>`/`<bash-stdout>`, `<command-name>`, `<local-command-stdout>` | excluded by the `origin.kind: "human"` rule |
| Stop-hook block reason | — | not observed: no `stop_hook_summary` with `preventedContinuation: true` and no `Stop hook feedback` line in any local transcript; Stop-hook output appears as `attachment` lines with `hookEvent: "Stop"` |
| `surface` block | — | no fixture: hook output, carried by `attachment` lines, which are never `user` lines; the flag-prefix tests (T005-T007) build it from its template |

## D2. Turn start

- **Decision**: the last line the spike identifies as a real user prompt, skipping every excluded kind and any line containing an exported flag prefix anywhere (FR-014). Not found inside the 2 MiB tail → both signals record `turn start not found`.

## D3. One read

- **Decision**: `TrajectoryReader::read` returns `(TrajectoryWindow, Vec<String>)`, the window and the raw tail lines, from one read with one session check. Batch callers ignore the lines. Mock stubs return an empty list.
- **Rationale**: one snapshot for the window and the turn data; the reader stays the only path to the transcript.

## D4. Previous final message

- **Decision**: a per-session map in `CheckpointDeps`, updated at every turn end including continuation turns, never persisted.

## D5. pushback_reversal detector

- **Decision**: structure check (a real user prompt after the previous final message, no tool call after it), then `review::mine_candidates` over a window holding the previous and final assistant messages, called with an empty recall slice as 064 does (`src/jev/shadow/turn.rs:112`). A candidate pair → the signal fires.

## D6. unverified_fact detector

- **Decision**: `src/jev/entities.rs` extracts file paths (backslashes and JSON escapes normalized to `/`), version strings (`\d+\.\d+(\.\d+)*`), identifiers and hashes of 7 or more characters, and numbers of 3 or more digits on word boundaries. Each is looked up in the turn's tool inputs (raw, before normalization), tool outputs and user prompt, normalized the same way. Unmatched entities are recorded by kind with a hash of their text.
- **Rationale**: the "Hallucination / ungrounded claim" row of `WATCHDOG_LAYER.md` ("What it watches for") names entity diffs; names and dates match unreliably against this turn's evidence.
- **Details (decided at T009)**: an identifier is code-shaped: 7 or more characters from letters, digits, `_` and `-`, with a digit, an underscore, or a capital after the first character, so plain words never count. A path is any token containing `/` or `\` once edge punctuation is stripped. A leading `v` is dropped from versions. A token of digits only is a number, not an identifier. ISO dates (`YYYY-MM-DD`) are skipped. Paths and versions are taken out before numbers, so the `14` in `1.14.0` is not a number. Numbers are looked up on word boundaries; the other kinds as substrings of the normalized evidence. The hash is `fnv1a64` of the normalized text, as 16 hex characters.

## D7. Audit share

- **Decision**: `checkpoint::fnv1a64(eval_id.as_bytes()) % 100 < 10` → audit. The evaluation id is injectable through `CheckpointDeps` for tests.

## D8. Pinned tables

- **Decision**: `TURN_REVIEW_THRESHOLDS: &[ThresholdEntry]` keyed by Jev model, big-model id and the turn-review question version (both questions share one), and `TURN_REVIEW_ORDER_CHECKED` keyed the same way, both empty, in `src/jev/policy/skip.rs`. Config refuses `TURN_REVIEW=on` unconditionally in 066 because the drift check does not exist. Thresholds are injectable through `CheckpointDeps` so the skip path is tested.

## D9. Turn-review questions version 2

- **Decision**: `contradiction` and `capture_worthy` gain declared criteria and become version 2; production sends version 2 from 066 on. Version-1 rows are excluded from promotion data. The order check (both criteria orders) runs on version 2 with the `jev_backtest` harness.

## D10. Skip order of work

- **Decision**: while the skip is off, today's `tokio::join!` of review and Jev stays unchanged. When enabled in tests, Jev runs first under the turn deadline; the review starts only if not skipped; a skipped turn's 064 row sets `applied = 1`.

## D11. Records

- **Decision**: `turn_signal_records` (schema in `contracts/turn-signal-record.schema.json`, bound two ways by a `PRAGMA table_info` test). The checkpoint record's `review_skipped_by` is mirrored on the OTLP span. Jev request counts and cost per turn are unchanged.

## D12. Live example

- **Decision**: `examples/acceptance_turn_signals.rs` runs the scrubbed fixtures through `run_turn` with the signals on and a scratch database, asserts one joined row per covered turn end with the expected detector results and that no sentinel string reaches any request body, and exits non-zero on failure. It makes no Jev call for the signals.
