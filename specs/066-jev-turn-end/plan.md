# Implementation Plan: Jev at turn end

**Branch**: `066-jev-turn-end` | **Date**: 2026-11-26 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/066-jev-turn-end/spec.md`; the Jev roadmap §6.2 and §7 (quoted in the spec).

## Summary

At every non-continuation turn end, with `CHECKPOINT_TURN_SIGNALS=on`, two turn-level failures are checked by the design corpus's cheap detectors, in code: the existing pair miner over the previous and final assistant messages after a real user prompt with no tool call (`pushback_reversal`), and a normalized verbatim lookup of the final message's entities in the turn's own evidence (`unverified_fact`). Results go to a new `turn_signal_records` table with anchors back to the transcript; no Jev call is made and nothing leaves the machine. The end-of-turn review skip is built and tested behind injected thresholds and stays off; `on` is refused until promotion data, an order check and the drift check exist.

## Technical Context

**Language/Version**: Rust, toolchain pinned 1.100.0, MSRV 1.94.

**Primary Dependencies**: no new crates.

**Storage**: SQLite: new table `turn_signal_records`; `checkpoint_records` gains `review_skipped_by` (migration).

**Testing**: `cargo test --all-features --tests` with the trajectory reader mock, `MockClassifier` for the skip, and scrubbed transcript fixtures from the spike; one live example that exits non-zero on failure.

**Target Platform**: the MCP stdio server on the operator's machine (Windows and Linux CI).

**Project Type**: library + binary (MCP server).

**Performance Goals**: the detectors run in code on at most the 2 MiB transcript tail already read; no network call.

**Constraints**: nothing new leaves the machine; the turn-review request carries only assistant-written text; off by default.

**Scale/Scope**: one row per covered turn end.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Design-Corpus Fidelity**: traces to roadmap §6.2 and `WATCHDOG_LAYER.md:151,153`. Named departures, each in the spec: the signals record the cheap detectors without a Jev question (Jev questions deferred to a later feature, because a question that works with the evidence withheld needs its own design and back-test); dates are left out of the entity lookup; the signals run outside the review's gate; the skip lets Jev stop the review; the 064 turn-review questions move to version 2. Amendments in FR-013. Pass.
- **II. Constrained-Output Contract**: no new model output; the turn-review questions stay typed yes/no. Pass.
- **III. Compiler-Enforced Discipline**: no `unwrap`/`expect` in production; new modules stay under 500 lines; orchestration goes in `src/checkpoint/signals.rs`, not `run.rs`. Pass.
- **IV. Seams, Composition, and Tests**: transcript reads stay behind `TrajectoryReader`; the skip is tested through injected thresholds and evaluation ids. Pass.
- **V. Deterministic Over Probabilistic**: both signals are deterministic detectors. A verbatim match shows an entity appears in the turn's evidence, not that the claim is true; the record says "unmatched", not "unsupported". Pass.
- **VI. Capabilities Off By Default**: `CHECKPOINT_TURN_SIGNALS` defaults to `off`; the skip's `on` is refused. Pass.
- **VII. Simplicity and Scope Discipline**: no Jev call for the signals, no cooldown, no promotion, no drift check. Pass.

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
│   ├── turn-signal-record.schema.json
│   ├── questions.md
│   └── record-additions.md
└── checklists/requirements.md
```

### Source Code (repository root)

```text
src/
├── traits/trajectory.rs      # read returns (window, tail lines); mock stubs updated
├── checkpoint/
│   ├── turn_view.rs          # NEW: parse_turn
│   ├── signals.rs            # NEW: detectors, recommendation mapping, row building, previous-final map
│   ├── run.rs                # call signals; skip path when enabled; audit share; review_skipped_by; flag prefixes for batch and hold
│   ├── mod.rs                # CheckpointRecord.review_skipped_by; module list
│   ├── review.rs             # flag prefix constant (shared with preference.rs)
│   ├── preference.rs         # uses the shared prefix
│   └── contract.rs           # signal-enum binding test
├── jev/
│   ├── entities.rs           # NEW: entity extraction, normalization, verbatim lookup
│   ├── questions.rs          # turn-review questions version 2 with criteria
│   ├── policy.rs             # pinned threshold table and order-check table (empty), skip decision
│   ├── mod.rs                # module list
│   └── startup.rs            # turn-review `on` wording
├── memory/push.rs            # surface prefix exported
├── config.rs                 # CHECKPOINT_TURN_SIGNALS; PARALLAX_JEV_* known-set check; TURN_REVIEW=on refused
├── main.rs                   # --help: CHECKPOINT_TURN_SIGNALS
├── storage/sqlite.rs         # turn_signal_records table; review_skipped_by migration
├── storage/turn_signals.rs   # NEW: record and read rows; schema binding test
├── traits/storage.rs         # record_turn_signals on Storage
└── observability.rs          # review_skipped_by on the span
examples/
├── acceptance_turn_signals.rs  # NEW live example (SC-004)
└── jev_backtest/             # report covers turn_signal_records; order check for version-2 questions
tests/fixtures/transcripts/   # NEW: scrubbed spike fixtures
docs/design/                  # MEMORY_LAYER.md, WATCHDOG_LAYER.md, CORRECTIVE_SELECTION.md, NEW_SERVER_DESIGN.md
specs/017-memory-consolidation/spec.md   # note
specs/006-checkpoint-layer/contracts/    # checkpoint record: review_skipped_by
specs/007-observability-layer/contracts/telemetry.md
README.md                     # CHECKPOINT_TURN_SIGNALS row
CHANGELOG.md
```

**Structure Decision**: single project, existing layout. Turn data comes from a separate function over the lines the reader already read; the window and its consumers are untouched.

## Complexity Tracking

No constitution violations to justify.
