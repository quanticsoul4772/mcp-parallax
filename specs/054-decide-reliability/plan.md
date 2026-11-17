# Implementation Plan: Decide Stops Claiming Calibrated Confidence

**Branch**: `054-decide-reliability` | **Date**: 2026-11-17 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/054-decide-reliability/spec.md`

## Summary

A words-only change. `decide` computes and returns exactly what it does today.
Its description stops saying the confidence is calibrated and gains three
sentences the owner chose; the design document gets a dated amendment
withdrawing the claim the measurement contradicts; the measurement's scores
and one script are committed with the owner's other-project text removed.

The work is four small groups, in this order, each ending with the full gate:

1. **Tests first** (FR-004, FR-005): each is seen to fail for its stated reason before the text it checks changes.
2. **Live text** corrected until those tests pass (FR-001 to FR-003, FR-007).
3. **The record**: design amendment, dated notes, change log (FR-006, FR-008, FR-009).
4. **Evidence folder** and removal of the probe from `examples/` (FR-010, FR-011).

## Technical Context

**Language/Version**: Rust, toolchain pinned at 1.100.0 (MSRV 1.94). One Python 3 script, standard library only, for the evidence folder; it is not part of the build or the gate.

**Primary Dependencies**: none added.

**Storage**: N/A.

**Testing**: `cargo test`. New tests: three unit tests in `src/modes/decide.rs`, one integration test in `tests/integration.rs`. All through existing seams; no model, network or disk.

**Target Platform**: unchanged (stdio MCP server; CI on Linux, owner's machine Windows). Nothing here is platform-specific.

**Project Type**: single Rust crate.

**Performance Goals**: N/A. No runtime path changes.

**Constraints**: no computed value, result field, model-facing prompt, model output shape or request may change (FR-005). No assertion in an existing test of `decide`'s computed values may be edited.

**Scale/Scope**: 16 existing files touched, plus one new evidence folder; roughly 30 lines of Rust text and comments, four tests, one design-document amendment, five one-line notes, one evidence folder of six files.

## Constitution Check

*GATE: evaluated before research and again after design. Both passes: no violations.*

| Principle | Assessment |
|---|---|
| I. Design-Corpus Fidelity | This feature exists to satisfy it: implementation experience showed a design statement wrong, and the corpus is amended in the same change (FR-006). The amendment is appended and dated; earlier text is not edited, matching how the corpus has been amended before. One named narrowing: the catalog says Decide corrects "miscalibration"; the amendment states a single pass does not. |
| II. Constrained-Output Contract | Untouched. `DecidePass`, its schema and the prompt do not change. |
| III. Compiler-Enforced Discipline | No production logic changes. Text and comments only. |
| IV. Seams, Composition, and Tests | Four new tests, written first, through existing seams. The Python script is evidence tooling outside the crate; its printed output is committed so the table can be compared by eye, and the plan claims no automated check for it. |
| V. Deterministic Over Probabilistic | The checks are exact string comparisons. |
| VI. Capabilities Off By Default | No capability added. |
| VII. Simplicity and Scope Discipline | Smallest change that removes the false claim. `src/modes/decide.rs` is already 796 lines, over the 500-line target; this feature adds three short tests and does not split the file. See Complexity Tracking. |

## Project Structure

### Documentation (this feature)

```text
specs/054-decide-reliability/
├── plan.md              # this file
├── research.md          # decisions and the alternatives rejected
├── data-model.md        # the exact strings and the file lists
├── quickstart.md        # how to check the result by hand
├── contracts/
│   └── description.md   # the full new description, before and after
├── checklists/
│   └── requirements.md
└── tasks.md             # written by /speckit-tasks, not by this plan
```

### Source Code (repository root)

```text
src/modes/decide.rs                         # description constant, field doc, module doc, comments; 3 new unit tests
src/server.rs                               # served description literal and the comment above it
src/memory/push.rs                          # one comment: drop "the stable band"
tests/integration.rs                        # 1 new test; one test renamed and its comment reworded (no assertion edited)
examples/acceptance_decide.rs               # header comment only
examples/probe_decide_sensitivity.rs        # removed from here (owner runs the delete)
CLAUDE.md                                   # the decide sentence only (FR-007); the Active feature block is asked of the owner separately
CHANGELOG.md                                # one entry under Unreleased / Changed
docs/design/NEW_SERVER_DESIGN.md            # new dated amendment, appended
specs/013-decide-methodology/spec.md                    # one dated line
specs/013-decide-methodology/contracts/decide.md        # quoted description (live)
specs/013-decide-methodology/contracts/decide.tool.json # confidence description text only (live)
specs/016-push-memory/spec.md               # one dated line
specs/016-push-memory/research.md           # one dated line
specs/017-memory-consolidation/spec.md      # one dated line
claudedocs/experiments/decide-order-bias/analysis.md    # one dated line
claudedocs/experiments/decide-stability/    # new: README.md, results.jsonl, plan-redacted.json, table.py, table-output.txt, probe.rs.txt
```

**Structure Decision**: no new module, crate or directory in `src/`. The only new directory is the evidence folder beside the earlier experiment.

## Decisions carried from research (summary; detail in research.md)

- **One description, two copies.** The served literal in `src/server.rs` and the constant in `src/modes/decide.rs` are both edited by hand; a new integration test holds them equal, following the test added for `grounded_verify`.
- **The forbidden stem is checked where the text lives.** A unit test checks the registered constant. The wider "live text" list in FR-007 is checked once by a search recorded in the task list, not by a permanent test: a permanent test that scans source files for a word would be the kind of hand-maintained scan this project has repeatedly found unreliable.
- **Every occurrence of the stem in `src/modes/decide.rs` is reworded**, including two comments inside its test module. No assertion is touched. Two hits remain after the work and are expected: the new test's own search string, and the `check` tool's description in `src/server.rs`, which is about another tool.
- **"No behaviour change" has one new guard and otherwise rests on review.** Existing tests pin the confidence values and ranking. Nothing pins the prompt text, the model's output schema, or the request body. A new unit test pins the prompt as built for a fixed input, with and without the retry wording, and the sanitized schema the request carries; the rest of the request body is established by reading the diff, and the pull request says so.
- **The contract file keeps its shape.** Only the confidence property's `description` text changes; a new unit test holds it equal to the derived schema's description and to FR-003.
- **Redaction is a script, and its output is shown to the owner before commit.**
- **`examples/probe_decide_sensitivity.rs` is deleted by the owner's command**, because deletion is blocked for the agent in this environment; its source is first copied to the evidence folder as `probe.rs.txt` with a corrected header.

## Order of work and gates

| Step | Work | Ends with |
|---|---|---|
| 1 | Write the four tests and run them. Two fail on today's text (the description content test and the field description test). Two pass today and are expected to: the prompt-and-schema pin, and the served-equals-registered test | output recorded, with which failed and why |
| 2 | Edit the registered constant first; record the served-equals-registered test failing; then edit the served literal, the field doc (on one source line), the contract file and the contract note's quoted block | all four tests pass; full gate |
| 3 | Reword the remaining live-text locations, including the `decide` sentence in `CLAUDE.md`; rename the one integration test; run the one-time stem search | search output recorded, showing only the two expected hits; full gate |
| 4 | Design amendment, five dated lines, change-log entry | the markdown check, run as its own command (the gate does not include it) |
| 5 | Evidence folder: redaction script run (it stops on any non-billing failure and on any unexpected case id), output shown to the owner, table script run and compared with the specification's table | owner's go-ahead on the redacted files |
| 0 | Before anything else: copy the probe's source into the evidence folder and have the owner delete it from `examples/`, so no gate run depends on it compiling | gate output as the starting state |
| 7 | Independent review of the whole diff; then the owner's decision on a live call; then commit, push, pull request, each asked | — |

## What this plan does not decide

- Whether to make one live `decide` call before the pull request. The project's note says such changes must pass a live example and none exists for `decide`; the owner is asked with that stated and with the price.
- Whether to revisit the memory-feature choices made with `decide` (named open question in the specification).

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| `src/modes/decide.rs` stays over the 500-line target (796 lines, plus three short tests) | The tests must sit beside the constant and schema they check | Splitting the file is a refactor; in a words-only change it would make "nothing else moved" impossible to see in the diff |
