# Tasks: Glob Locators for grounded-verify

**Feature**: `009-glob-locators` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

Tests included (Constitution Principle IV). The pattern→regex translator is the
correctness-critical piece and gets an exhaustive ground-truth table.

## Phase 1: Setup

- [x] T001 Add `regex` and `walkdir` to `Cargo.toml` `[dependencies]`; `cargo build` to confirm they resolve and the lockfile updates.
- [x] T002 [P] Extend `SourceLocator` in `src/grounded/mod.rs` to the path-XOR-glob shape (`path: Option<String>`, `glob: Option<String>`, `start_line`, `end_line`) with serde + `schemars::JsonSchema`, preserving wire-compatibility for the existing `{ "path": ... }` form.
- [x] T003 [P] Scaffold the `grounded::glob` submodule — `src/grounded/glob/mod.rs` with the public `expand` entry signature and `GlobError`→`AppError::InvalidInput` mapping — and register `pub mod glob;` in `src/grounded/mod.rs`.

## Phase 2: Foundational (blocking prerequisites)

**Goal**: the deterministic substrate — a pure pattern→regex translator and a root-confined expander — that the user stories build on.

- [x] T004 Implement the extended-glob → anchored-regex translator in `src/grounded/glob/translate.rs`: `*`, `**` (recursive), `?`, `[class]`/`[!class]`, brace `{a,b}` (nestable), extglob `@(...)` `?(...)` `*(...)` `+(...)` `!(...)`, and leading `!` (whole-pattern negation). Matching is **case-sensitive**; `*`/`**` **match dotfiles**; extglob groups are **segment-scoped** — never cross `/`, and `!(p)` matches a single segment not matching `p` (FR-010). A malformed pattern (unbalanced `(`/`[`/`{`, empty alternation) is a named `InvalidInput`. Pure — string in, compiled `Regex` out.
- [x] T005 [P] Unit tests for the translator in `src/grounded/glob/translate.rs` (ground-truth table): each construct matches the intended paths and rejects others; `**` crosses segments while `*` does not; nested brace; each extglob operator incl. `!(...)` as a segment-scoped negation (does not cross `/`); case-sensitivity (a differently-cased path does not match); dotfile matching (`*`/`**` match a leading-dot file); malformed patterns rejected named. No disk.
- [x] T006 Implement the expander in `src/grounded/glob/expand.rs`: walk the canonical root with `walkdir` set to NOT follow symlinks; take each file's root-relative path; match against the compiled regex; sort matches lexicographically; re-confine each via the reader's canonical-prefix check; return the ordered `Vec<PathBuf>`. Zero matches ⇒ named `InvalidInput` identifying the pattern.
- [x] T007 [P] Unit tests for the expander in `src/grounded/glob/expand.rs` (tempdir fixtures): expands to the sorted set; deterministic order across runs; zero-match errors named; a file reached only through a symlinked directory is not matched (no symlink follow).

**Checkpoint**: translation and expansion exist and are unit-tested — US1 can begin.

## Phase 3: User Story 1 - Verify against a pattern-matched set (P1) 🎯 MVP

**Goal**: a glob locator expands to its matching files and is judged as evidence, each in the manifest.

**Independent test**: a glob matching several files yields a verdict over all of them and a manifest naming each expanded file.

- [x] T008 [US1] Validate `SourceLocator` in `src/grounded/mod.rs`: exactly one of `path`/`glob` present; a line range only with `path`; a `glob` with a range is a named `InvalidInput` (FR-007). Unit-tested.
- [x] T009 [US1] In `src/grounded/assemble.rs`, expand every glob locator (via `grounded::glob::expand`) into its sorted concrete path locators before the existing read loop, then feed the unified locator list through 008's per-locator read + manifest + total-byte loop unchanged (FR-002/FR-009).
- [x] T010 [P] [US1] Integration tests in `tests/integration.rs` (009 block): a glob expands to multiple files with one manifest entry each (concrete paths, not the pattern); the same glob over unchanged files yields byte-identical evidence twice (determinism); a glob mixed with an exact-path locator in one call; and the existing 008 path/range tests pass unchanged.

**Checkpoint**: US1 independently shippable — globs work end to end.

## Phase 4: User Story 2 - A zero-match glob fails loudly (P2)

**Goal**: a glob matching nothing is a named error, never a clean empty verdict.

**Independent test**: a glob matching no file returns a named error and renders no verdict.

- [x] T011 [US2] Integration test in `tests/integration.rs`: a glob that matches no file under the root returns a named `invalid_params` error identifying the pattern, with no verdict and (per the all-or-nothing record path) one `invalid_input` invocation record.

## Phase 5: User Story 3 - Expansion respects the ceilings (P3)

**Goal**: a broad glob can't silently blow or be trimmed past the per-call budgets.

**Independent test**: an over-locator-ceiling glob and an over-byte-ceiling glob each return a named error and no verdict.

- [x] T012 [US3] In `src/grounded/assemble.rs`, enforce `GROUNDED_VERIFY_MAX_LOCATORS` against the running total as globs expand (loud named error on overflow, never truncation); confirm the 008 total-byte and per-file ceilings still apply to every expanded file.
- [x] T013 [P] [US3] Integration tests in `tests/integration.rs`: a glob expanding past the locator ceiling errors named; a glob whose expanded files exceed the byte ceiling errors named; a glob carrying a line range is rejected named (FR-007 end-to-end).

**Checkpoint**: all three stories complete.

## Phase 6: Polish & Cross-Cutting Concerns

- [x] T014 [P] Corpus currency (Principle I): one-line note in `docs/design/NEW_SERVER_DESIGN.md` that `grounded-verify` locators include glob patterns (refining the 008 entry).
- [x] T015 [P] Extend `examples/acceptance_grounded_verify.rs` with a glob pass covering SC-001..005: expand-to-set + manifest, determinism across runs, zero-match error, ceiling overflow error, and root confinement via a symlinked directory.
- [x] T016 [P] Docs sync: note glob locators in the `grounded_verify` README Tools row, and add `src/grounded/glob/` to the CLAUDE.md repo layout.
- [x] T017 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.

## Dependencies & Execution Order

- **Setup (T001–T003)** → **Foundational (T004–T007)** → **US1 (T008–T010)** → **US2 (T011)** → **US3 (T012–T013)** → **Polish (T014–T017)**.
- US2 and US3 depend only on US1's expansion-in-assemble being wired; they are additive assertions/guards on it.
- T014 (corpus note) is independent of the code and can run any time; it gates merge (Principle I).

## Parallel Execution Examples

- **Setup**: T002 and T003 run together (distinct files), after/with T001.
- **Foundational**: T005 and T007 (unit tests) parallel once T004/T006 exist; T004 (translator) and T006 (expander) are largely independent and can overlap.
- **US1**: T010 runs after T008–T009.
- **Polish**: T014, T015, T016 all parallel; T017 last.

## Implementation Strategy

MVP = **Setup → Foundational → US1**: globs expand, are judged, and appear in the
manifest. US2 (zero-match) and US3 (ceilings + glob+range) are thin independent
guards on top. The translator (T004) is the bulk of the work and the bulk of the
tests; everything else reuses 008. T014 and T017 are merge gates.

Note: like 008, the integration and acceptance tests use the wiremock-mocked
model, so this whole feature is implementable offline — the rebuilt live binary
is needed only to dogfood globs through the running MCP server, not to land 009.
