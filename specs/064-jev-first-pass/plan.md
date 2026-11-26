# Implementation Plan: Jev foundation and shadow answers

**Branch**: `064-jev-first-pass` | **Date**: 2026-11-24 | **Spec**: [spec.md](spec.md)

**Input**: `specs/064-jev-first-pass/spec.md` (commits 6b9a5b2, dd2cf6c) and the Jev roadmap `~/.claude/plans/warm-kindling-shell.md` (revision 3, after two review rounds).

## Summary

064 adds TypeSafe's Jev to Parallax as a cheap typed-judgment tier and runs it in **shadow** at two places:

- the end-of-turn watchdog review;
- research claim verification.

Jev's answers are recorded beside the big model's answer for the same item, and nothing Jev says changes any output. The point is measurement: the records and the FR-011 measurements are the evidence that 065 and 066 need before any Jev answer may change behavior.

The feature also:

- builds the client, the mandatory redactor, the records, label storage, and a tested metrics module with a back-test harness;
- ends by turning both sites on (shadow) in the operator's setup.

## Technical Context

**Language/Version**: Rust, toolchain pinned at 1.100.0; MSRV 1.94.

**Primary Dependencies**: existing crates only: `reqwest`, `tokio`, `serde`, `sqlx` (SQLite), `mockall`, `wiremock`, `tracing`, `opentelemetry`. No new dependencies; in particular no randomness crate and no compression crate.

**Storage**: the existing SQLite database, plus one new table, `jev_records`.

**Testing**: `cargo test --all-features --tests`; mock seams (mockall); wiremock for the client; live examples under `examples/` that assert and exit non-zero on failure.

**Target Platform**: the stdio MCP server, run by Claude Code on Windows and Linux.

**Performance Goals**: Jev adds no wait to a turn review beyond `max(0, jev − review)`. Per-claim deadlines keep research within its existing budget. Live, Jev time on the critical path is reported per run.

**Constraints**:

- **Deadlines:** turn 1000 ms; claim 2000 ms, capped by the run's remaining time and never longer than the claim's verify.
- **Concurrency:** at most 4 Jev requests in flight per process.
- **Retries:** none in shadow.
- **Data:** nothing sent unredacted; no stored memories sent.

**Scale/Scope**:

- About 150 turn reviews and about 5 research runs on a heavy day.
- Each research run carries up to about 420 claims at deep depth, a part of which is excluded in code.

## Constitution Check

Checked against `.specify/memory/constitution.md`:

| Principle | How 064 complies |
|---|---|
| Outbound features are gated and off by default; enabling is a deliberate operator decision (`:92-94`) | Every `PARALLAX_JEV_MODE_*` defaults to `off`. A shared `TYPESAFE_API_KEY` alone enables nothing. Turning sites on in the operator's setup is the operator's decision, made as a closing step (FR-010). |
| Every external dependency behind a mockable seam | New `Classifier` trait with `MockClassifier`; storage through the `Storage` trait. |
| Constrained, validated output (II) | Jev's typed answers plus the local validator (FR-002) are this site's output contract. Rejected answers never pass. |
| Loud configuration; no silent fallback | Unknown suffix, bad value, or a mode without a key is a startup error naming the variable. `on` doesn't exist in 064. |
| No hidden failures (III) | Every attempted call and every exclusion is recorded or counted. A 401 is logged once. A missing key is logged at startup. |
| Tests first | Each task writes its failing test first (spec SC-001..SC-006; plan §11). |
| One record per call | The checkpoint record and the Jev row are written by the same record step (`record_with_jev()`), the Jev row second; a failed Jev write is logged and never affects the turn. Research rows are written by `RecordGuard` with the invocation record. |

There are no violations. Two items are noted under Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/064-jev-first-pass/
├── spec.md
├── plan.md              # this file
├── research.md          # decisions D1–D15
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── jev-record.schema.json
│   ├── record-additions.md
│   ├── telemetry-additions.md
│   └── config.md
├── checklists/requirements.md
└── tasks.md             # /speckit-tasks
```

### Source code

**New:**

```text
src/traits/classifier.rs          # Classifier seam, request/answer types, MockClassifier
src/client/typesafe.rs            # TypeSafe client
src/client/typesafe/tests.rs      # client tests (claude_cli precedent)
src/client/retry.rs               # retry helpers moved from exa.rs (no behavior change)
src/jev/mod.rs
src/jev/redact.rs                 # RedactedState + redactor
src/jev/questions.rs              # versioned questions, option orders
src/jev/policy.rs                 # validator, band functions (Option in, both directions)
src/jev/shadow.rs                 # shadow_turn, shadow_claim (keeps call-site changes small)
src/jev/metrics.rs                # §7 metrics, bounds, promotion_decision()
src/storage/jev.rs                # jev_records table and writes
examples/jev_backtest.rs          # harness and FR-011 measurements
examples/acceptance_jev.rs        # live shadow check (asserts)
tests/fixtures/jev/               # synthetic or scrubbed fixtures, with provenance markers
claudedocs/jev-research-2026-11-24-agents.md   # the seven agent reports
```

**Changed:**

| File | Change |
|---|---|
| `src/checkpoint/run.rs` | Evaluation id created at the start of `run_turn` (`:256-378`; today in `record()` at `:549-550`); Jev joined with `review_once`; `record()` also writes the Jev row and `jev_cost_usd`. `CheckpointDeps` (`:30-48`) gains the classifier and mode. |
| `src/checkpoint/review.rs` | `review_once` (`:348`) also returns its parsed raw answers; `Candidate` gains an origin field (memory pairs built at `:192-203`); `truncate()` (`:448`) returns a truncated flag. |
| `src/research/pipeline.rs` | Jev joined with `verify::run` per claim inside its permit (`:499-520`); exclusion in code; accumulator push. `ResearchDeps` (`:45-75`) gains the classifier and mode. |
| `src/server/record.rs` | Invocation id created in `RecordGuard::new`; `finish` and `Drop` write accumulated Jev rows in one transaction. |
| `src/telemetry.rs` | Id passed into `InvocationRecord::create` (`:465`); `jev_cost_usd` field; price row `jev-1.14.0`; `every_default_model_has_a_price_row` updated. |
| `src/observability.rs` | `parallax.jev.*` attributes (numeric/enum) on checkpoint and invocation spans. |
| `src/error.rs` | Outcome `classifier_provider`; binding test (`:527-565`) updated. |
| `src/config.rs` | Key, modes, model; prefix-scan validation modeled on `src/routing.rs:471-483`. |
| `src/server.rs` | Builds the classifier when the key is set and a site is on; startup log lines. |
| `src/client/claude_cli/invocation.rs` | `TYPESAFE_API_KEY` added to `ENV_REMOVE` (`:17-24`). |
| `src/client/exa.rs` | Uses `src/client/retry.rs`; no behavior change. |
| Config fixtures | `src/config.rs:652` (`test_config`), `tests/common/mod.rs:20`, `examples/common/mod.rs:55`. |
| Contracts | `specs/001-core-layer/contracts/invocation-record.schema.json`; `specs/006-checkpoint-layer/contracts/` checkpoint record; `specs/007-observability-layer/contracts/telemetry.md`. |
| Docs | `README.md` env table, `--help`, `CHANGELOG.md`, CLAUDE.md (pointer and runtime config), `docs/design/SDK_LANDSCAPE.md` (TypeSafe row), `docs/design/CORRECTIVE_SELECTION.md` (signals-table row). Amendment text is shown before it is written. |

## Phases

- **Phase 0 (research):** `research.md`, decisions D1–D15, each with what was chosen, why, and what was rejected.
- **Phase 1 (design):** `data-model.md`, `contracts/`, `quickstart.md`; CLAUDE.md pointer update.
- **Phase 2:** `/speckit-tasks` builds `tasks.md`, test-first, ordered as follows:
  1. config, validator, bands;
  2. client;
  3. redactor;
  4. records and contracts;
  5. U2 shadow;
  6. U4 shadow;
  7. cost and telemetry;
  8. metrics and harness;
  9. live example;
  10. FR-011 measurements;
  11. docs;
  12. setup (FR-010).

## Complexity Tracking

| Item | Why | Mitigation |
|---|---|---|
| `src/checkpoint/run.rs` (1,683 lines) and `src/research/pipeline.rs` (1,069) exceed the 500-line target | Both are existing files and the shadow sites live there | All new logic goes in `src/jev/shadow.rs`; each call site gains about 5 lines |
| `src/storage/sqlite.rs` is 1,730 lines | Existing | The new table goes in its own module, `src/storage/jev.rs` |
