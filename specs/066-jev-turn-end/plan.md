# Implementation Plan: Jev at turn end

**Branch**: `066-jev-turn-end` | **Date**: 2026-11-26 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/066-jev-turn-end/spec.md`; the Jev roadmap §6.2 and §7 (quoted in the spec).

## Summary

At every non-continuation turn end, two turn-level failures are checked in shadow. Code runs the design corpus's cheap detector for each first: the existing pair miner over the previous and final assistant messages for `pushback_reversal`, and a verbatim entity lookup against the turn's own evidence for `unverified_fact`. Jev is asked only about what the detector leaves open, and sees only assistant text. Results are recorded in `jev_records` as `turn_signals` rows; nothing changes in the turn. The end-of-turn review skip is built and tested but stays off: `on` needs a pinned threshold table entry and a recorded option-order check.

## Technical Context

**Language/Version**: Rust, toolchain pinned 1.100.0, MSRV 1.94.

**Primary Dependencies**: no new crates.

**Storage**: SQLite `jev_records` (064), one new site value `turn_signals`; `checkpoint_records` gains `review_skipped_by` (migration).

**Testing**: `cargo test --all-features --tests` with `MockClassifier`, the trajectory reader mock, and transcript fixtures from the spike; one live example that exits non-zero on failure.

**Target Platform**: the MCP stdio server on the operator's machine (Windows and Linux CI).

**Project Type**: library + binary (MCP server).

**Performance Goals**: no added time on a turn end while the signals run beside the review; `parse_turn` runs only at turn end, never on the batch path.

**Constraints**: nothing the user wrote and nothing a tool returned leaves the machine; turn deadline 1000 ms; at most 4 Jev requests in flight per process; off in code.

**Scale/Scope**: one turn end at a time per session; at most one Jev request per turn end.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Design-Corpus Fidelity**: traces to roadmap §6.2 and `WATCHDOG_LAYER.md:151,153`. Departures are named in the spec: the date/number exclusion is dropped (FR-006), the signals run outside the review's gate (FR-013 amends `WATCHDOG_LAYER.md:59`), and the review skip lets Jev stop the review (same amendment). Pass.
- **II. Constrained-Output Contract**: Jev answers are typed yes/no, validated by 064's validator. Pass.
- **III. Compiler-Enforced Discipline**: no `unwrap`/`expect` in production; new modules (`src/checkpoint/turn_view.rs`, `src/jev/shadow/signals.rs`, `src/jev/entities.rs`) stay under 500 lines. Pass.
- **IV. Seams, Composition, and Tests**: Jev stays behind `Classifier`; transcript reads stay behind `TrajectoryReader`; every behavior is test-first. Pass.
- **V. Deterministic Over Probabilistic**: each signal's cheap detector runs first, in code; Jev judges only what it leaves open; dates and numbers are settled by verbatim match. Pass.
- **VI. Capabilities Off By Default**: `PARALLAX_JEV_MODE_TURN_SIGNALS` defaults to `off`; the skip's `on` is refused without a pinned table entry and order check; the startup line names destination and data class. Pass.
- **VII. Simplicity and Scope Discipline**: two questions, one request per turn end, no cooldown in shadow, no promotion. Pass.

Re-check after Phase 1: unchanged, pass.

## Project Structure

### Documentation (this feature)

```text
specs/066-jev-turn-end/
├── spec.md
├── plan.md              # this file
├── research.md          # Phase 0
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/           # Phase 1
│   ├── config.md
│   ├── questions.md
│   └── record-additions.md
└── checklists/requirements.md
```

### Source Code (repository root)

```text
src/
├── checkpoint/
│   ├── turn_view.rs          # NEW: parse_turn (turn start, user prompt, tool inputs and outputs)
│   ├── run.rs                # signals beside the review; skip path when on; audit share; review_skipped_by
│   ├── mod.rs                # CheckpointRecord.review_skipped_by; flag prefixes exported
│   ├── review.rs             # flag prefix constant; pair miner reused for pushback_reversal
│   └── preference.rs         # flag prefix constant
├── jev/
│   ├── entities.rs           # NEW: entity extraction and verbatim lookup
│   ├── questions.rs          # pushback_reversal, unverified_fact; criteria for the turn-review questions
│   ├── policy.rs             # TURN_REVIEW_NO_BAND table (empty), order-check table, skip decision
│   ├── startup.rs            # startup line for the signals site
│   └── shadow/signals.rs     # NEW: detectors → request → turn_signals row; recommendation mapping
├── memory/push.rs            # surface block prefix exported (turn-start exclusion)
├── traits/classifier.rs      # JevSite::TurnSignals
├── config.rs                 # PARALLAX_JEV_MODE_TURN_SIGNALS; closed PARALLAX_JEV_* names; TURN_REVIEW=on gate
├── server.rs                 # wiring; last final message per session
├── storage/sqlite.rs         # review_skipped_by migration
├── storage/jev.rs            # site value
└── observability.rs          # review_skipped_by and request count on the span
examples/
├── acceptance_jev_signals.rs # NEW live example (SC-004)
└── jev_backtest/             # order check for the turn-review questions
tests/fixtures/transcripts/   # NEW: spike fixtures
```

**Structure Decision**: single project, existing layout. Turn data comes from a separate function on the same transcript lines, so the trajectory window and its consumers are untouched.

## Complexity Tracking

No constitution violations to justify.
