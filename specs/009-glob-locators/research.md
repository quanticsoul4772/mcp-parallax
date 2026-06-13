# Research: Glob Locators for grounded-verify

Phase 0 decisions. The one genuine unknown — how to obtain the clarified grammar
(full extended globbing incl. bash extglob operators) in Rust — is resolved
below; the rest reuse 008.

## D1 — No off-the-shelf Rust crate provides bash extglob; build a custom translator

**Decision**: implement a small custom **pattern→regex translator** for the full
extended-glob grammar, then match file paths (from a tree walk) against the
compiled regex.

**Rationale**: a web survey of the Rust glob ecosystem (2026-06):

- `globset` (ripgrep): `*`, `?`, `[class]`, `**`, brace `{a,b}`, and leading-`!`
  whole-pattern negation — but **no bash extglob operators** (`!(...)`, `+(...)`).
- `glob` (1.x): `*`, `?`, `[class]`, `**` only — no brace, no extglob.
- `fast-glob` (oxc), `glob-match`: fast, support brace; **no bash extglob**.

The clarified grammar requires extglob operators, which none provide. A regex is
the natural target: every extended-glob construct (including `!(...)` as a
negative-lookahead-style segment) maps to a regex, and the `regex` crate gives a
linear-time, audited matcher. The translator is a small recursive-descent parser
— pure, no I/O — so the correctness-critical part is exhaustively unit-testable.

**Alternatives considered**:

- **`globset` grammar, drop extglob operators** — explicitly offered at
  `/speckit-plan` and rejected by the user in favor of true extglob.
- **Shell out to bash extglob** — rejected: a shell dependency, non-portable
  (Windows), and an injection surface.
- **`globset` + a brace/extglob pre-expander** — extglob (`!()`, `+()`) is not
  pure brace expansion; it cannot be desugared into globset's grammar.

## D2 — Tree walk with `walkdir`, no symlink following

**Decision**: expand a glob by walking the canonicalized root with `walkdir`
configured to **not follow symlinks**, taking each entry's path relative to the
root, and matching it against the compiled regex.

**Rationale**: not following symlinks prevents a symlinked directory from
walking *out* of the root (FR-004 / SC-003); the walk is rooted at the canonical
root so every entry is structurally inside it. As defense in depth, every match
is still re-checked through 008's `SystemSourceReader` confinement before it is
read.

**Alternatives**: a recursive `std::fs::read_dir` — reinvents `walkdir`'s
symlink/error handling; rejected.

## D3 — Deterministic order by sorting the matched relative paths

**Decision**: collect matches, then sort the relative paths lexicographically
(byte order) before turning them into concrete locators.

**Rationale**: `walkdir` order is not guaranteed stable across platforms; sorting
makes the expanded set — and therefore the assembled evidence — identical across
runs (FR-003 / SC-002), preserving 008's determinism.

**Alternatives**: rely on walk order — rejected (platform-dependent).

## D4 — `SourceLocator` grows a glob shape (path XOR glob)

**Decision**: extend `SourceLocator` so a locator is *either* a path (with the
optional line range, 008) *or* a glob pattern — never both. A locator carrying a
glob and a line range is a loud `InvalidInput` (FR-007); a locator with neither
path nor glob is rejected; a locator with both is rejected.

**Rationale**: explicit shapes keep the 008 path/range behaviour byte-identical
(FR-009) while adding the new one unambiguously — no "is this path actually a
glob?" guessing.

**Alternatives**: infer glob-ness from metacharacters in `path` — rejected
(ambiguous; a real path may contain `[` or `{`).

## D5 — Expansion happens in the assembly stage, before the read loop

**Decision**: in `assemble`, first expand every glob locator into its sorted
concrete path locators, enforcing the locator-count ceiling as the combined list
grows; then the unified list flows through 008's existing per-locator read +
manifest + byte-ceiling loop unchanged. Zero matches for any glob → loud named
error; total expanded locators over the ceiling → loud named error.

**Rationale**: keeps all-or-nothing and the ceilings centralized in one place,
and the read/manifest path is 008 verbatim (FR-008/FR-009).

**Alternatives**: expand inside the reader — rejected (mixes expansion with
confinement/read; harder to test and reason about).

## D6 — Two new dependencies: `fancy-regex`, `walkdir`

**Decision**: add `fancy-regex` (compile/match the translated pattern) and
`walkdir` (the rooted, non-symlink-following walk).

**Rationale**: an SDK-landscape addition justified by the custom-engine
decision; hand-rolling either would reinvent vetted code. **Implementation
discovery**: the plan first named `regex`, but the RE2-style `regex` crate has
no lookahead, and correct extglob negation `!(p)` (with a suffix after the
group) requires a negative lookahead — so `fancy-regex` (backtracking) is the
necessary engine. Subject to the weekly `cargo audit` gate like every
dependency.

## D7 — Corpus currency (Principle I)

**Decision**: a one-line update to the `grounded-verify` entry in
`NEW_SERVER_DESIGN.md` noting that locators include glob patterns. Not a new
corrective — a refinement of the 008 entry.
