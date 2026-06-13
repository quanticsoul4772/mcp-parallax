# Implementation Plan: Glob Locators for grounded-verify

**Branch**: `009-glob-locators` | **Date**: 2026-06-13 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/009-glob-locators/spec.md`

## Summary

Add a glob locator shape to `grounded_verify` (008): the caller names a glob
pattern, the server expands it — deterministically, confined to the configured
root, all-or-nothing — into the concrete set of matching files, each of which
flows through 008's existing read/manifest/ceiling machinery unchanged. The
clarified grammar is **full extended globbing including bash extglob operators**
(`!(...)`, `+(...)`, `@(...)`, `?(...)`, `*(...)`), which no off-the-shelf Rust
crate provides — so the engine is a small **custom pattern→regex translator**
over a non-symlink-following tree walk. The translator (the hard, correctness-
critical part) is pure and unit-tested exhaustively; the walk is tested against
temp dirs, like 008's confinement tests.

## Technical Context

**Language/Version**: Rust 1.94 (edition 2021) — the existing crate.

**Primary Dependencies**: existing 008 grounded layer (`SystemSourceReader`
confinement, all-or-nothing `assemble`). New: `regex` (compile the translated
pattern) and `walkdir` (non-symlink-following directory walk). Both are vetted,
widely-used crates; this is a small SDK-landscape addition documented in
research.md.

**Storage**: none new — one `invocation_record` per call, unchanged.

**Testing**: `cargo test`. The pattern→regex translator is pure and tested
against a ground-truth table (each grammar feature: `*`, `?`, `**`, classes,
brace, each extglob operator, negation, and rejection of malformed patterns).
Expansion (walk + match + sort + confine) is tested against temp-dir fixtures.

**Target Platform**: stdio MCP server (Linux / Windows / macOS).

**Project Type**: single Rust project, extended in place.

**Performance Goals**: expansion is a bounded tree walk + per-path regex match;
linear in the file count under the root. No new target beyond staying within
008's byte/locator ceilings.

**Constraints**: deterministic (sorted) expansion; root-confined (walk does not
follow symlinks out; every match re-checked); all-or-nothing; text-only and the
byte/per-file/locator ceilings from 008 apply to every expanded file.

**Scale/Scope**: one new locator shape, one new submodule (`grounded::glob`,
translator + expander), a `SourceLocator` extension, two new deps. The
exact-path and line-range paths and their tests are untouched.

## Constitution Check

*Evaluated against `.specify/memory/constitution.md`.*

- **I. Design-Corpus Fidelity** — ✅ 009 broadens the locator surface of
  `grounded-verify`, already registered in the corpus by the 008 amendment. A
  one-line update to that entry (globs are a supported locator shape) keeps the
  corpus current; tracked as a task. Not a new corrective.
- **II. Constrained-Output Contract** — ✅ no new model schema; globs are
  input-side only. The pass schema is unchanged.
- **III. Compiler-Enforced Discipline** — ✅ `#![forbid(unsafe_code)]`, no
  `unwrap`/`expect` in production paths; lints unchanged.
- **IV. Seams, Composition, Tests** — ✅ the translator is a pure function
  (string → regex), the correctness-critical part, fully unit-tested without
  disk. The walk is composition over the existing reader's confinement; tested
  against temp dirs as in 008.
- **V. Deterministic Over Probabilistic** — ✅ expansion is wholly deterministic
  (a pure regex translation + a sorted walk); no model involvement at all.
- **VI. Capabilities Off By Default** — ✅ no new capability or gate; globs are
  available only when `grounded_verify` is enabled (`GROUNDED_VERIFY_ROOT` set).
- **VII. Simplicity and Scope Discipline** — ⚠️ **Named deviation.** A custom
  extglob engine is more machinery than a `globset` wrapper. The simpler
  globset-grammar path (no extglob operators) was offered at `/speckit-plan` and
  **explicitly rejected** in favor of true extglob (clarification 2026-06-13).
  Justified by direct user requirement; bounded by keeping the engine to one
  pure translator + a thin walk, and reusing all of 008's confinement and
  assembly. Tracked in Complexity Tracking.

**Gate result**: PASS with one named deviation (Principle VII — custom engine,
user-directed), justified and tracked. No unjustified violations.

## Project Structure

### Documentation (this feature)

```text
specs/009-glob-locators/
├── plan.md              # This file
├── research.md          # Phase 0 — engine decision (custom translator), deps, semantics
├── data-model.md        # Phase 1 — SourceLocator v2, expansion entities, validation
├── quickstart.md        # Phase 1 — enable + a worked glob call
├── contracts/
│   └── glob-locator.md   # Phase 1 — the locator input shape + grammar + errors
└── tasks.md             # Phase 2 — /speckit-tasks (not this command)
```

### Source Code (repository root)

```text
src/grounded/
├── glob/
│   ├── mod.rs           # NEW — public expand(pattern, &reader/root) entry; GlobError mapping
│   ├── translate.rs     # NEW — extended-glob pattern → anchored regex (pure, recursive-descent: *, ?, **, [class], {a,b} nested, !()/+()/@()/?()/*(), leading !)
│   └── expand.rs        # NEW — walkdir (no symlink follow) under root → relative paths → regex match → sort → re-confine → ordered Vec<PathBuf>
├── mod.rs               # MODIFIED — SourceLocator gains a glob shape (path XOR glob; range only with path)
├── assemble.rs          # MODIFIED — expand glob locators into concrete path locators before the existing read loop; zero-match + ceiling errors; glob+range rejected
└── reader.rs            # UNCHANGED — confinement/read reused as-is

Cargo.toml               # MODIFIED — add `regex` and `walkdir`

tests/integration.rs     # MODIFIED — 009 block: glob expands to a set, determinism, zero-match, ceiling overflow, glob+range rejection, confinement, mixed with exact paths
examples/acceptance_grounded_verify.rs  # MODIFIED — add a glob SC pass (SC-001..005)

docs/design/NEW_SERVER_DESIGN.md  # MODIFIED — one-line note: grounded-verify locators include globs (Principle I currency)
```

**Structure Decision**: a focused `grounded::glob` submodule isolates the
translator (the substantial, pure, heavily-tested piece) from the thin walk and
from 008's untouched read path. `SourceLocator` grows one shape; everything
downstream of expansion is 008 verbatim.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Principle VII — custom extglob engine | The clarified grammar is full extended globbing incl. bash extglob operators (`!(...)`, `+(...)`, …), which no Rust crate provides off-the-shelf | The `globset` grammar (wildcards + `**` + brace + leading-`!` negation, no extglob operators) was offered at `/speckit-plan` and explicitly rejected by the user in favor of true extglob. Honoring the stated requirement; the engine is bounded to one pure translator + a thin walk reusing 008. |
| Two new deps (`regex`, `walkdir`) | The translator emits a regex; the expander needs a non-symlink-following walk | Hand-rolling a regex matcher or a recursive `std::fs` walk would reinvent vetted, audited crates for no benefit; both are standard and minimal. |
