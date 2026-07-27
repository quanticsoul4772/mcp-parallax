---

description: "Task list for 040 — an unresolvable default fails instead of being skipped"
---

# Tasks: An Unresolvable Default Fails Instead of Being Skipped

**Input**: Design documents from `/specs/040-unresolvable-default-fails/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/failure-modes.md

**Tests**: REQUIRED (Constitution Principle IV).

**A note on what "test" means here.** The deliverable *is* test code — a
resolver and the checks that call it. So a task that "adds a test" for this
feature means **a test of the resolver itself**, driven by a fixture rather than
by the real `config.rs`. Testing the resolver only against real configuration
would mean it passes today and silently stops covering a shape the moment
`config.rs` changes — the exact failure being fixed, rebuilt one level up.

**Organization**: grouped by user story so each is independently testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable — different files, no dependency on an incomplete task
- **[Story]**: US1–US3 from spec.md
- Exact file paths in every description

## Path Conventions

Single Rust project: `src/`, `tests/` at repository root.

---

## Phase 1: Setup

**Purpose**: the module both document checks will call, and the fixture that
lets it be tested independently of real configuration.

- [X] T001 Create `src/config_facts.rs` as a `#[cfg(test)]` module declared from `src/main.rs`, with the `SOURCES` file list (`src/config.rs`, `src/client/anthropic.rs`) and an empty `EXCLUSIONS: &[(&str, &str)]`
- [X] T002 Add a fixture string in `src/config_facts.rs` containing one variable of each of the four default shapes plus one unresolvable shape, so the resolver is tested against known input rather than against whatever `config.rs` happens to contain today

**Checkpoint**: module compiles, suite unchanged.

---

## Phase 2: Foundational (Blocking)

**Purpose**: the resolver itself. **No user story can start until this
completes** — all three consume its output.

- [X] T003 Implement `Resolution` in `src/config_facts.rs` as a three-state enum (`Resolved(value)`, `Excluded(reason)`, `Unresolvable(found)`) with **no fourth variant for skipped**, per data-model.md
- [X] T004 Implement default extraction in `src/config_facts.rs` for `parse_env("X", <expr>)` and `unwrap_or_else(|_| <expr>)`, returning the raw expression text rather than a filtered value, so classification happens in one place
- [X] T005 Implement literal resolution in `src/config_facts.rs` for numeric literals (stripping `_` separators) and string literals
- [X] T006 Implement constant resolution in `src/config_facts.rs` by reading only the files in `SOURCES` and matching `const NAME: type = value;`, returning `Unresolvable` when the name is absent from that set (FR-004a)
- [X] T007 [P] Test in `src/config_facts.rs` that each of the four shapes in the fixture resolves to its expected value
- [X] T008 [P] Test in `src/config_facts.rs` that the fixture's unresolvable shape returns `Unresolvable` carrying the expression text, never a skip
- [X] T009 [P] Test in `src/config_facts.rs` that a constant declared outside `SOURCES` returns `Unresolvable`, and that the message names the constant, the variable, and the searched set but **does not** name a file it did not read (FR-004b)

**Checkpoint**: the resolver is correct against the fixture and independent of
real configuration. Nothing caller-facing yet.

---

## Phase 3: User Story 1 — An unresolvable default stops the build (P1)

**Goal**: a default in a form the resolver does not handle fails, naming the
variable. Skipping is not a reachable outcome.

**Independent Test**: introduce a default in an unrecognised form in the real
`config.rs` and confirm the suite fails naming that variable; add it to
`EXCLUSIONS` and confirm it passes.

### Tests for User Story 1 (REQUIRED) ⚠️

- [X] T010 [P] [US1] Test in `src/config_facts.rs` that `resolve()` over the real `config.rs` produces zero `Unresolvable` entries today — the assertion that fails first when someone writes a new shape
- [X] T011 [P] [US1] Test in `src/config_facts.rs` that an excluded variable is reported as `Excluded` and not compared against any document
- [X] T012 [P] [US1] Test in `src/config_facts.rs` that an `EXCLUSIONS` entry naming a variable which carries no default fails with `EXCLUSION_STALE` (FR-003)

### Implementation for User Story 1

- [X] T013 [US1] Implement `assert_all_resolved()` in `src/config_facts.rs`, failing with the `DEFAULT_UNRESOLVED` message from contracts/failure-modes.md — variable, expression found, shapes handled, both remedies
- [X] T014 [US1] Implement the stale-exclusion check in `src/config_facts.rs`, asserting every `EXCLUSIONS` name still appears as a variable carrying a default

**Checkpoint**: US1 delivers alone. The silent skip is unreachable from here on,
even before any document check is rewired.

---

## Phase 4: User Story 2 — Every documented default matches the code (P1)

**Goal**: both documents state the value the server applies, for every variable
that has one, in both directions.

**Independent Test**: change any default in `config.rs` without touching the
documents and confirm the suite fails naming the variable and both values;
separately, add a default row for a variable that does not exist and confirm it
fails.

### Tests for User Story 2 (REQUIRED) ⚠️

- [X] T015 [P] [US2] Test in `src/config_facts.rs` that the extraction guard fires when a document yields fewer rows than expected, with the `EXTRACTION_EMPTY` message — the 039 failure, where a CRLF boundary search matched nothing and every variable was reported missing
- [X] T016 [P] [US2] Test in `src/main.rs` that a named-constant default drifting in `--help` fails — the mutation that motivated this feature and currently passes (SC-002)
- [X] T017 [P] [US2] Test in `src/main.rs` that a string default drifting in `--help` fails, covering the shape the hand-written list of three used to assert
- [X] T018 [P] [US2] Test in `src/main.rs` that a default stated in the README for a variable with none fails with `DEFAULT_PHANTOM` (FR-009)
- [X] T019 [P] [US2] Test in `src/main.rs` that a number in the README's Purpose column produces no finding, so the reverse direction stays quiet on prose (FR-010, SC-007)

- [X] T019a [US2] Test in `src/main.rs` that after rewiring, `VOYAGE_API_KEY`, `BRAVE_API_KEY`, `GROUNDED_VERIFY_ROOT` and `CHECKPOINT_GATE_PATTERNS` are still required to appear in both `--help` and the README table (FR-008). These carry no default, so the resolver never sees them — deleting two scans in T022 and T024 is exactly how their presence checks would be lost without anything failing

### Implementation for User Story 2

- [X] T020 [US2] Implement `assert_document_agrees()` in `src/config_facts.rs`: forward comparison for every resolved variable, naming every document in a mismatch so a fix landing in one place and not another is visible
- [X] T021 [US2] Implement the reverse direction in `src/config_facts.rs`, reading the README table's Default column and `--help`'s `(default: X)` marker only, never surrounding prose
- [X] T022 [US2] Replace the `--help` numeric scan in `src/main.rs` with a call to the resolver, deleting the digit-filter and its silent `continue`
- [X] T023 [US2] Replace the `--help` hand-written list of three string defaults in `src/main.rs` with the resolver's string coverage
- [X] T024 [US2] Replace the README scan in `src/main.rs` with a call to the resolver, deleting the second copy of the extraction along with the guards it grew that its sibling never did

**Checkpoint**: both documents checked by one resolution. The two copies are
gone rather than both patched.

---

## Phase 5: User Story 3 — Coverage is stated, not implied (P2)

**Goal**: the check reports how many variables it examined, as a count that must
balance rather than a floor that can be cleared.

**Independent Test**: confirm the reported count equals variables-with-a-default
minus exclusions, and that removing a variable from resolution fails the balance.

### Tests for User Story 3 (REQUIRED) ⚠️

- [X] T025 [P] [US3] Test in `src/config_facts.rs` that `resolved + excluded == variables carrying a default`, and that an artificially dropped variable fails with `COVERAGE_UNBALANCED`
- [X] T026 [P] [US3] Test in `src/config_facts.rs` that the reported figure is the count examined, not a threshold — pinning that the `checked >= 8` floor is gone rather than renamed

### Implementation for User Story 3

- [X] T027 [US3] Implement the coverage equation in `src/config_facts.rs` and remove the `checked >= 8` floor from `src/main.rs`

**Checkpoint**: partial coverage can no longer report success.

---

## Phase 6: Polish & Cross-Cutting

- [X] T028 Run the motivating mutation end to end — set `GROUNDED_VERIFY_MAX_BYTES` to `999999` in both `README.md` and the help body in `src/main.rs`, confirm the suite fails, then restore. This passed before the feature (SC-002)
- [X] T029 [P] Mutation-verify each new failure mode in contracts/failure-modes.md fires, and record which mutation produced which message — a failure surface nobody has seen fire is a claim, not a check
- [X] T030 [P] Add a `CHANGELOG.md` `[Unreleased]` entry covering the resolver, the four shapes, both directions, and the coverage equation
- [X] T031 [P] Amend `specs/036-*` and `specs/039-*` references in `CHANGELOG.md` to record that their derivation was partial, so the released history does not read as though the loop was closed
- [X] T032 Run the full gate: `cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test`
- [X] T033 Constitution-required pre-merge review of `src/config_facts.rs` and `src/main.rs` by `code-reviewer`, with the resolver's text-parsing assumptions named as the thing to attack

---

## Dependencies

```text
Phase 1 (Setup) ──► Phase 2 (Resolver) ──┬──► Phase 3 (US1, P1)
                                          ├──► Phase 4 (US2, P1)
                                          └──► Phase 5 (US3, P2)
                                                      │
                          Phases 3-5 ─────────────────┴──► Phase 6 (Polish)
```

- **Phase 2 blocks everything.** All three stories consume the resolver.
- **US1, US2 and US3 are independent of each other** once the resolver exists.
  US1 is the requirement; US2 is the coverage that follows; US3 is the report.
- **T031 depends on nothing in this feature** — it is a correction to released
  notes and could be done first.

## Parallel Opportunities

- **Phase 2**: T003→T004→T005→T006 sequential (same file, building on each
  other); T007, T008, T009 parallel once T006 lands
- **Phase 3**: T010–T012 parallel; T013→T014 sequential
- **Phase 4**: T015–T019a parallel; T020→T021 sequential, then T022, T023, T024
  parallel (distinct call sites)
- **Phase 5**: T025, T026 parallel
- **Phase 6**: T029, T030, T031 parallel; T028 then T032→T033 last
- **Across phases**: Phases 3, 4 and 5 can run concurrently once Phase 2 lands

## Implementation Strategy

**MVP = Phase 1 + Phase 2 + Phase 3 (US1).** That alone makes the silent skip
unreachable, which is the feature. The document coverage in US2 is what the
requirement buys, and it is worth having, but a build that cannot skip silently
is already strictly better than what exists.

**Recommended order**: US1 → US2 → US3. US3 last because a coverage figure is
only meaningful once the thing it counts is complete.

**Stop-and-review point**: after Phase 2. If the text-based resolver turns out
to need more shape-handling than expected — the risk plan.md names — that is
where it shows, and the fallback is `EXCLUSIONS` rather than a redesign.
