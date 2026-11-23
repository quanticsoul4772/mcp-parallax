# Implementation Plan: Research deadlines that fit the claude_cli backend

**Branch**: `062-claude-cli-research-deadlines` | **Date**: 2026-11-23 | **Spec**: [spec.md](spec.md)

## Summary

Research picks its default deadline from the depth tier
(`DepthTier.default_deadline_ms`: 120 / 240 / 480 s) in
`research/settings.rs`, unless the caller passes `constraints.deadline_ms`.
`ResearchDeps` gains the selected backend; when it is `claude_cli`, the
default comes from a second per-depth table (120 / 360 / 720 s). The caller
override, its 5000 ms minimum, and every other tier value are untouched, so
other backends stay byte-identical.

## Technical Context

**Language/Version**: Rust (toolchain 1.100.0, MSRV 1.94)
**Primary Dependencies**: none new
**Storage**: none
**Testing**: `cargo test`; unit tests on the settings resolution per backend
and depth, plus one pipeline test that a `claude_cli` standard run is still
running past 240 s on a test clock
**Constraints**: other backends byte-identical (FR-002, SC-003)
**Scale/Scope**: one field, one table, four construction sites

## Constitution Check

The constitution (`.specify/memory/constitution.md`) is the template
placeholder, so there are no ratified gates. The project conventions apply:
test first, no fallbacks, full gate before each commit. No new egress or
capability, so nothing to gate behind configuration.

## Project Structure

```text
src/
├── research/mod.rs        # Depth::claude_cli_deadline_ms (120/360/720 s)
├── research/settings.rs   # default deadline chosen by backend (FR-001/002)
├── research/pipeline.rs   # ResearchDeps.backend
├── server.rs              # passes config.backend into ResearchDeps
examples/acceptance_research.rs  # passes config.backend (SC-001/002 live run)
src/research/pipeline_tests.rs   # deps builder: backend field
CHANGELOG.md               # FR-004
```

## Decisions

See [research.md](research.md).
