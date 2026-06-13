# Tasks: Source-Grounded Verification (`grounded-verify`)

**Feature**: `008-grounded-verify` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

Tests are included — the constitution (Principle IV) requires mockable seams
with tests, and every prior layer shipped them.

## Phase 1: Setup

- [ ] T001 Add `grounded_verify_root: Option<String>`, `grounded_verify_max_bytes: usize` (default `262144`), and `grounded_verify_max_locators: usize` (default `64`) to `Config` and `Config::from_env` in `src/config.rs`, with the loud-malformed convention (present-but-unparseable ⇒ `ConfigError::Invalid`) and a config test.
- [ ] T002 [P] Declare the `SourceReader` seam (8th trait) in `src/traits/source.rs` — one operation resolving a locator to verbatim text + byte length or a typed error — and export it from `src/traits/mod.rs`.
- [ ] T003 [P] Scaffold the `grounded` module (`src/grounded/mod.rs`) and register `pub mod grounded;` in `src/lib.rs`.

## Phase 2: Foundational (blocking prerequisites for all stories)

**Goal**: the deterministic substrate — confined reading, all-or-nothing assembly, and a reusable ensemble — that every user story builds on.

- [ ] T004 Define `SourceLocator` (`path`, optional `start_line`/`end_line`) and the locator/assembly error mapping to `AppError::InvalidInput` (missing/empty/out-of-range/non-text/out-of-root/over-ceiling, each naming the locator) in `src/grounded/mod.rs`.
- [ ] T005 Implement `SystemSourceReader` in `src/grounded/reader.rs`: canonicalize the configured root once; per locator, join → canonicalize → assert the canonical path is prefixed by the canonical root (rejects `../` and symlink escape before any read); enforce UTF-8 text-only; support whole-file and 1-based inclusive line-range reads; return text + byte length.
- [ ] T006 [P] Unit tests in `src/grounded/reader.rs` (tempdir-backed): traversal (`../`) rejected, symlink-escape rejected, in-root read succeeds, line-range bounds, empty-file and non-text errors — all naming the locator.
- [ ] T007 Implement the assembly stage in `src/grounded/assemble.rs`: resolve **all** locators all-or-nothing over a `&dyn SourceReader`, enforce `max_locators` and `max_bytes` (loud, named), and build `AssembledEvidence` (spans in resolution order with provenance). Pure over the seam.
- [ ] T008 [P] Unit tests in `src/grounded/assemble.rs`: any single failure aborts and names the locator (no partial set), byte/locator ceilings enforced, deterministic span order, success path assembles verbatim — against a mock `SourceReader`.
- [ ] T009 Factor `verify`'s pass-and-aggregate (run K stance-blind passes → majority verdict, agreement-derived confidence, collected findings) into a shared routine in `src/modes/verify.rs` reusable by `grounded_verify`, with **no behavior change** to `verify` (existing verify tests still pass).

**Checkpoint**: confined reads, all-or-nothing assembly, and the shared ensemble exist and are unit-tested — US1 can begin.

## Phase 3: User Story 1 - Verify a claim against verbatim source (P1) 🎯 MVP

**Goal**: a working `grounded_verify` tool that reads named verbatim source and returns a verdict the caller cannot bias by wording.

**Independent test**: a conclusion-laden claim + accurate source ⇒ the verdict tracks the source, not the phrasing.

- [ ] T010 [US1] Implement the `grounded_verify` mode in `src/modes/grounded_verify.rs`: the prompt, the flat + closed pass schema `{ verdict, findings, missing_evidence }` (validated by the local validator), and `run = assemble → shared ensemble → server-assembled result (verdict, findings, confidence)`; register it in `src/modes/mod.rs`.
- [ ] T011 [US1] Wire the `grounded_verify` tool in `src/server.rs`: present in the catalog only when `grounded_verify_root` is set; inject the `SourceReader` dependency; record exactly one invocation via the existing `run_recorded` path (OTLP-exported when telemetry is configured).
- [ ] T012 [P] [US1] Integration tests in `tests/integration.rs` (008 block): catalog gating (no root ⇒ tool absent; root ⇒ present), verbatim-flips-verdict, exactly one invocation record, and an unresolvable locator surfaces a named `invalid_params` error with no verdict.
- [ ] T013 [P] [US1] Schema/contract test: the pass schema is flat + closed and passes the sanitizer/validator; the result shape matches `contracts/grounded-verify.md`.

**Checkpoint**: US1 is independently shippable — the MVP corrective works end to end.

## Phase 4: User Story 2 - Audit the evidence (P2)

**Goal**: the verdict carries an inspectable evidence manifest.

**Independent test**: the returned manifest exactly matches the resolved locators (files, ranges, sizes) and reconstructs the evidence set.

- [ ] T014 [US2] Build the `EvidenceManifest` (per-span `path`, `start_line?`, `end_line?`, `bytes`, in resolution order) in `src/grounded/assemble.rs` and surface it server-assembled in the `GroundedVerdict` result in `src/modes/grounded_verify.rs` (never model-authored — FR-012).
- [ ] T015 [P] [US2] Integration tests in `tests/integration.rs`: manifest matches a mixed locator set including a line-range entry; entries carry exact spans and byte sizes; reconstructable from the manifest alone.

**Checkpoint**: US1 + US2 — verdicts are auditable.

## Phase 5: User Story 3 - Surface omitted evidence (P3)

**Goal**: the result names evidence the caller didn't provide.

**Independent test**: a seeded omission ⇒ the completeness signal names the missing source class; complete evidence ⇒ empty.

- [ ] T016 [US3] Aggregate the per-pass `missing_evidence` (union + dedup across passes) into the result's completeness signal in `src/modes/grounded_verify.rs`; empty when the evidence suffices.
- [ ] T017 [P] [US3] Integration tests in `tests/integration.rs`: a claim depending on an omitted source ⇒ completeness names the missing class; fully-covered claim ⇒ empty signal.

**Checkpoint**: all three stories complete.

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T018 [P] **Corpus amendment (Principle I)**: register `grounded-verify` in `docs/design/NEW_SERVER_DESIGN.md` (failure-mode catalog: "context-curation trust gap"; primitives: a Verify-family corrective) and add a routing note in `docs/design/CORRECTIVE_SELECTION.md`, tracing to this spec.
- [ ] T019 [P] Acceptance example `examples/acceptance_grounded_verify.rs` exercising SC-001..006 (verbatim-flips-verdict, manifest fidelity, all-or-nothing named errors, root confinement incl. symlink, catalog gating when unset, completeness over seeded omissions).
- [ ] T020 [P] Docs sync: add the Tools-table row and the three Configuration rows to `README.md`, and update `CLAUDE.md` (status line, config section, repo layout for `src/grounded/` and the 8th seam).
- [ ] T021 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.

## Dependencies & Execution Order

- **Setup (T001–T003)** → **Foundational (T004–T009)** → **US1 (T010–T013)** → **US2 (T014–T015)** → **US3 (T016–T017)** → **Polish (T018–T021)**.
- US2 and US3 each depend only on US1 (the mode + result exist); they are additive and could be reordered, but priority order is P1→P2→P3.
- T018 (corpus amendment) is required by the constitution and gates merge, but is independent of the code and can be done any time in parallel.

## Parallel Execution Examples

- **Setup**: T002 and T003 run together (distinct files), after/with T001.
- **Foundational**: T006 and T008 (unit tests, distinct files) parallel once T005/T007 exist.
- **US1**: T012 and T013 parallel after T010–T011.
- **Polish**: T018, T019, T020 all parallel (docs/example, distinct files), then T021 last.

## Implementation Strategy

MVP = **Phase 1 → Phase 2 → Phase 3 (US1)**: a gated, root-confined,
all-or-nothing `grounded_verify` that returns a verbatim-grounded verdict. Ship
or demo there. US2 (manifest) and US3 (completeness) are independent increments
on top. The corpus amendment (T018) and full gate (T021) are merge gates.
