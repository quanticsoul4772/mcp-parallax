# Implementation Plan: Screen research pages for prompt injection

**Branch**: `063-research-injection-screen` | **Date**: 2026-11-23 | **Spec**: [spec.md](spec.md)

## Summary

A new pure module `src/research/screen.rs` exposes
`screen(text) -> Vec<&'static str>`: the sorted, de-duplicated names of the
patterns found in a page's text. `extract_source` calls it on the exact text
it sends to extraction (after trimming and the character cap), stores the
names on the `SourceRecord`, and the outcome copies them to each result
source as `injection_patterns`. `Stats.pages_flagged` counts sources with at
least one name. The page text itself is untouched, so extraction and
verification are unchanged (warn, not block).

## Technical Context

**Language/Version**: Rust (toolchain 1.100.0, MSRV 1.94)
**Primary Dependencies**: none new; plain string and char matching
**Storage**: none (no invocation-record change)
**Testing**: unit tests on `screen` per pattern family and the edge cases;
pipeline tests that a flagged page is still extracted unchanged and that the
result and stats carry the names and count
**Constraints**: no network or model call (FR-001, SC-003); text unchanged
(FR-005)

## Constitution Check

The constitution is the template placeholder, so there are no ratified gates.
Project conventions apply: test first, no fallbacks, full gate before each
commit. No egress or code execution, so nothing to put behind configuration.

## Project Structure

```text
src/research/screen.rs      # new: pattern families, screen()
src/research/mod.rs         # pub mod screen; SourceRecord.injection_patterns
src/research/pipeline.rs    # extract_source screens the text it extracts
src/research/contract.rs    # SourceRef.injection_patterns, Stats.pages_flagged
src/research/outcome.rs     # copies the names; counts flagged pages
specs/004-research-layer/contracts/research.tool.json   # FR-006
src/research/pipeline_tests.rs  # pipeline tests
```

## Decisions

See [research.md](research.md).
