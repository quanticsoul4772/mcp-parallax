# Implementation Plan: Jev injection warning on research sources

**Branch**: `065-jev-injection-warning` | **Date**: 2026-11-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/065-jev-injection-warning/spec.md`; Jev roadmap `~/.claude/plans/warm-kindling-shell.md` §6.1 U5 (amended 2026-11-25).

## Summary

Research asks Jev the `directs_ai` question about each page it keeps for extraction, beside that page's extraction call, and with the new site `on` marks pages scoring at or above the pinned threshold (0.35 for `jev-1.14.0`) with an advisory warning, separate from 063's pattern screen. Shadow records the same calls and changes nothing. It reuses 064's client, redactor, validator, records table and back-test harness; it adds the first `on` mode (this site only), a pinned threshold table, retries for `on` calls within a 2000 ms deadline, and new result fields that appear only when the site is on.

## Technical Context

**Language/Version**: Rust, toolchain pinned 1.100.0, MSRV 1.94.

**Primary Dependencies**: no new crates. Existing: rmcp, reqwest, tokio, serde/serde_json, sqlx (SQLite), schemars, tracing, mockall, wiremock (tests).

**Storage**: SQLite `jev_records` (064), one new site value `research_injection`; no new column.

**Testing**: `cargo test --all-features --tests` with `MockClassifier` and wiremock; one live example (`examples/acceptance_jev_injection.rs`) that exits non-zero on failure.

**Target Platform**: the MCP stdio server on the operator's machine (Windows and Linux CI).

**Project Type**: library + binary (MCP server).

**Performance Goals**: no added time on a research run when Jev answers before the page's extraction call (live Jev calls measured 94-159 ms; extraction takes seconds).

**Constraints**: 2000 ms per Jev call including retries, capped by the run's remaining time; at most 4 Jev requests in flight per process (064); nothing leaves unredacted; off in code.

**Scale/Scope**: 5 / 15 / 35 kept pages per run by depth (061), so up to 35 Jev calls per run, about $0.002 each at most.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Design-Corpus Fidelity**: traces to plan §6.1 U5 and the design corpus's corrective-selection signals table (Jev as a version-pinned, back-tested classifier). The plan amendment (claim settling split out, 2026-11-25) is named in the spec's Input. `RESEARCH_PRIMITIVE.md` gets a short amendment for the advisory field. Pass.
- **II. Constrained-Output Contract**: Jev answers are typed (noul) and validated by 064's validator; no model output is parsed as free text. The research result schema gains optional fields only. Pass.
- **III. Compiler-Enforced Discipline**: no `unwrap`/`expect` in production; pedantic clippy; files ≤500 lines for new modules (`src/jev/shadow/injection.rs`). Pass.
- **IV. Seams, Composition, and Tests**: Jev stays behind the `Classifier` seam; every behavior is test-first with `MockClassifier`/wiremock; a live example asserts SC-005. Pass.
- **V. Deterministic Over Probabilistic**: 063's deterministic screen is untouched and runs first; Jev only adds an advisory field and never clears or weakens a deterministic finding. Pass.
- **VI. Capabilities Off By Default**: `PARALLAX_JEV_MODE_RESEARCH_INJECTION` defaults to `off`; the key alone enables nothing; egress is named in a startup line. Turning it on in the operator's setup is a separate operator decision (FR-010). Pass.
- **VII. Simplicity and Scope Discipline**: one question, one call per page, no splitting, no breaker, no claim settling. Pass.

Re-check after Phase 1: unchanged, pass.

## Project Structure

### Documentation (this feature)

```text
specs/065-jev-injection-warning/
├── spec.md
├── plan.md              # this file
├── research.md          # Phase 0
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/           # Phase 1
│   ├── config.md
│   ├── research-result-additions.md
│   └── record-additions.md
└── checklists/requirements.md
```

### Source Code (repository root)

```text
src/
├── traits/classifier.rs      # JevSite::ResearchInjection, JevMode::On
├── config.rs                 # PARALLAX_JEV_MODE_RESEARCH_INJECTION; `on` rules
├── jev/
│   ├── questions.rs          # directs_ai question (criteria), version 1
│   ├── policy.rs             # pinned threshold table, injection decision
│   ├── startup.rs            # startup line for the new site
│   └── shadow/injection.rs   # NEW: prepare, ask (with on-site retries), row
├── client/typesafe.rs        # Retry-After surfaced on 429/529 errors
├── research/
│   ├── pipeline.rs           # ResearchDeps.jev_injection; call beside extract_source
│   ├── contract.rs           # optional per-source and stats fields
│   └── outcome.rs            # fields filled only when the site is on
├── server.rs                 # wiring
└── storage/jev.rs            # site value bound to JevSite::ALL
examples/
├── jev_backtest/             # injection-site report from stored records
└── acceptance_jev_injection.rs   # NEW live example (SC-005)
```

**Structure Decision**: single project, existing layout; the new code lives in one new module beside 064's turn and claim shadow modules, and research gains one optional dependency.

## Complexity Tracking

No constitution violations to justify.
