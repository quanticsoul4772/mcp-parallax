# Implementation Plan: Jev on tool calls

**Branch**: `067-jev-tool-risk` | **Date**: 2026-11-27 (rewritten from the roadmap) | **Spec**: [spec.md](spec.md)

**Input**: `specs/067-jev-tool-risk/spec.md`, which carries the roadmap's §6.3, §11 067 list and §5.5 as written, plus the owner's four answers.

## Summary

After a spike verifies it, the `PreToolUse` matcher widens to MCP tools, excluding Parallax's own (§6.3.1). `run_action` classifies every matched call, including with memory off (§6.3.2): risk patterns, then a whole-command allowlist, then Jev for the remainder, with one choice question, `action_class` (§6.3.3, §6.3.4). The gate's verdict and the memory hold on `Bash`, `Write` and `Edit` are unchanged and never wait for Jev (§6.3.5, Clarifications O and P). Jev sees projections only (§6.3.6). Shadow only (§6.3.8), off in code, turned to `shadow` in the owner's setup last (§4.1 line 186).

## Technical Context

**Language/Version**: Rust, toolchain pinned 1.100.0, MSRV 1.94.

**Primary Dependencies**: no new crates.

**Storage**: SQLite: no schema change; `jev_records.site` gains `tool_risk`.

**Testing**: §11's eight 067 tests with `MockClassifier` and the mock storage; one live example per §5.5.

**Target Platform**: the MCP stdio server on the operator's machine (Windows and Linux CI).

**Project Type**: library + binary (MCP server).

**Performance Goals**: gate latency unchanged (§11; Clarifications P).

**Constraints**: projections only (§6.3.6); off by default (§3.9); no behavior change (§6.3.8).

**Scale/Scope**: one checkpoint record per matched call; one `jev_records` row per Jev request or code exclusion.

## Constitution Check

- **I. Design-Corpus Fidelity**: carries roadmap §6.3. The roadmap's own contradictions are settled by the owner (spec Clarifications O and P); amendments per §6.3.9. Pass.
- **II. Constrained-Output Contract**: typed choice answer, validated by the existing Jev validator. Pass.
- **III. Compiler-Enforced Discipline**: no `unwrap`/`expect` in production; new modules under 500 lines. Pass.
- **IV. Seams, Composition, and Tests**: Jev behind `Classifier`; tests with a classifier that hangs, errors or answers `read_only` at 1.0. Pass.
- **V. Deterministic Over Probabilistic**: code order first (§6.3.3); the hold stays deterministic. Pass.
- **VI. Capabilities Off By Default**: `PARALLAX_JEV_MODE_TOOL_RISK` defaults to `off`; `on` refused. Pass.
- **VII. Simplicity and Scope Discipline**: nothing beyond §6.3 and the four answers. Pass.

## Project Structure

### Documentation (this feature)

```text
specs/067-jev-tool-risk/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── config.md
│   ├── questions.md
│   ├── hooks-matcher.md
│   └── record-additions.md
└── tasks.md             # /speckit-tasks
```

### Source Code (repository root)

```text
integrations/claude-code/hooks.json, integrations/claude-code/README.md, .claude/settings.json   # §6.3.1
src/checkpoint/run.rs                # run_action restructure (§6.3.2, §6.3.5)
src/checkpoint/tool_risk.rs          # new: order, allowlist, projections (§6.3.3, §6.3.6)
src/jev/questions.rs                 # action_class (§6.3.4)
src/jev/shadow/tool.rs               # new: the request and its row (§6.3.5, §5.1 records)
src/server.rs                        # wiring the site into CheckpointDeps
CHANGELOG.md, CLAUDE.md              # touch-points (roadmap §10)
src/traits/classifier.rs             # JevSite::ToolRisk
src/config.rs, src/main.rs, README.md  # PARALLAX_JEV_MODE_TOOL_RISK
examples/acceptance_tool_risk.rs     # §5.5 live example
docs/design/WATCHDOG_LAYER.md, specs/006-checkpoint-layer/, src/checkpoint/gate.rs   # §6.3.9
```

## Process (roadmap line 163)

Test-first implementation, the full gate, an independent review, live acceptance, then the PR.

## Open items

- §11 test 7, "memory questions off without the opt-in": 067 asks Jev no memory question, so what the test checks is to be settled with the owner (tasks.md T015).

## Complexity Tracking

No constitution violations.
