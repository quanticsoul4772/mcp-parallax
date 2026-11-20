# Implementation Plan: Provider rejections recorded as their own outcome

**Branch**: `057-provider-rejected-outcome` | **Date**: 2026-11-20 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/057-provider-rejected-outcome/spec.md`

## Summary

Both model clients turn every unsuccessful provider response that is neither
429 nor 5xx into `AppError::Client`, which the outcome taxonomy records as
`validation_failure`, the class for malformed model output. Add one error
variant, `ProviderRejected { status, detail }`, built at exactly those two
sites, and one outcome, `provider_rejected`. Make the outcome list enumerable
and bind it by test to both published lists (the invocation-record schema and
the telemetry contract), replacing a hand-pinned string test that never opens
the contract file. Retry behaviour, the OpenAI-compatible strategy ladder, and
the classification of out-of-contract successful responses are unchanged.

## Technical Context

**Language/Version**: Rust 1.100.0 (pinned), MSRV 1.94

**Primary Dependencies**: existing only — `thiserror`, `reqwest`, `serde_json`, `wiremock` (tests)

**Storage**: SQLite `invocation_records.outcome` is text; no migration. `Outcome::parse` learns the new string

**Testing**: `cargo test`; `wiremock` doubles for both clients; a contract test reading the two published lists

**Target Platform**: the MCP stdio server, both model backends (Anthropic, OpenAI-compatible)

**Project Type**: MCP server (library + binary)

**Performance Goals**: none affected

**Constraints**: loud over silent — a rejection stays terminal and named; no change to retry policy; the error surface and the record share one taxonomy (`src/error.rs` module doc)

**Scale/Scope**: `src/error.rs`, two client construction sites, two published contracts, one design-doc amendment, CHANGELOG

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Note |
|---|---|---|
| I. Design-Corpus Fidelity | Pass, with amendment | The taxonomy's home, `specs/001-core-layer/data-model.md` §6, gets a dated amendment in the same change (FR-008). |
| II. Constrained-Output Contract | N/A | No mode schema or prompt changes. |
| III. Compiler-Enforced Discipline | Pass | Exhaustive `match` on `AppError` and `Outcome` makes every mapping site update or fail to compile; no new lint exceptions. |
| IV. Seams, Composition, Tests | Pass | Tests first through `wiremock` doubles; no new dependency. |
| V. Deterministic Over Probabilistic | N/A | Classification only. |
| VI. Capabilities Off By Default | N/A | No new capability. |
| VII. Simplicity and Scope Discipline | Pass | One variant, one outcome, two construction sites. Transport-error classification stays out of scope. |

Re-check after Phase 1: unchanged; see research D1–D6.

## Project Structure

### Documentation (this feature)

```text
specs/057-provider-rejected-outcome/
├── plan.md              # This file
├── research.md          # Phase 0 decisions D1–D6
├── data-model.md        # The variant, the outcome, what stays the same
├── quickstart.md        # How to verify
├── contracts/
│   └── outcome.md       # The exact recorded outcome list after this change
├── checklists/
│   └── requirements.md
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
src/error.rs                     # ProviderRejected variant, Outcome::ProviderRejected, Outcome::ALL, parse/as_str, contract-binding test
src/client/anthropic.rs          # non-success, non-429/5xx response → ProviderRejected (effort hint kept)
src/client/openai_compat.rs      # terminal 4xx → ProviderRejected (ladder and 200-malformed unchanged)
specs/001-core-layer/contracts/invocation-record.schema.json   # outcome enum gains provider_rejected
specs/007-observability-layer/contracts/telemetry.md           # parallax.outcome list gains provider_rejected
specs/001-core-layer/data-model.md                             # dated amendment to §6
CHANGELOG.md                     # Changed entry
```

**Structure Decision**: Keep the change inside the error taxonomy and the two places that build the misclassified error. Callers of `AppError` that match on outcome get a compile error until they handle the new value.

## Complexity Tracking

None: no principle is violated.
