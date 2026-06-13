# Data Model: Glob Locators for grounded-verify

All in-memory; no new persistence. Builds on 008's entities.

## SourceLocator (input) — v2

A locator is now *either* a path (with optional line range) *or* a glob.

| Field | Type | Notes |
|---|---|---|
| `path` | string? | Exact relative path (008). Mutually exclusive with `glob`. |
| `glob` | string? | Extended-glob pattern (009). Mutually exclusive with `path`. |
| `start_line` | integer? | 1-based inclusive start; only valid with `path`. |
| `end_line` | integer? | 1-based inclusive end; only valid with `path`. |

Validation (each a loud `InvalidInput`, naming the locator):

- Exactly one of `path` / `glob` is present (neither, or both, is an error).
- `start_line`/`end_line` may appear only with `path`; a `glob` with a line range is rejected (FR-007).
- 008's path rules (range pairing, 1 ≤ start ≤ end) are unchanged.

Wire compatibility: an 008 `{ "path": "a.rs" }` locator is still valid (the
`path` shape); `{ "glob": "src/**/*.rs" }` is the new shape.

## GlobPattern → Regex (internal, pure)

The translator turns one extended-glob pattern into an anchored regex.

Supported constructs (the clarified grammar):

| Construct | Meaning |
|---|---|
| `*` | any run of non-separator chars |
| `**` | any number of path segments (recursive) |
| `?` | one non-separator char |
| `[abc]`, `[a-z]`, `[!abc]` | character class / negated class |
| `{a,b,c}` | brace alternation (nestable) |
| `@(p1\|p2)` | exactly one of the alternatives |
| `?(p)` | zero or one |
| `*(p)` | zero or more |
| `+(p)` | one or more |
| `!(p)` | a single path segment that does NOT match `p` (segment-scoped; never crosses `/`) |
| leading `!` | negates the whole pattern's match result |

**Matching semantics (FR-010)**: case-sensitive; `*` and `**` match dotfiles
(leading-dot files are evidence); every extglob group is segment-scoped and
never crosses `/` (cross-segment recursion is `**` only). `*` matches a run of
non-`/` characters; `**` matches any number of whole segments.

A malformed pattern (unbalanced `(`/`[`/`{`, empty alternation) is a loud
`InvalidInput` naming the pattern — never a silent empty match.

## Expanded match set (internal)

Produced by `expand`: the deterministic, root-confined, sorted list of concrete
files a glob resolves to.

- Built by walking the canonical root (no symlink following), matching each
  file's root-relative path against the compiled regex.
- Sorted lexicographically by relative path (FR-003).
- Each member becomes a path-shaped `SourceLocator` (no range) and is re-checked
  by 008's confinement before reading.
- Empty set ⇒ loud `InvalidInput` naming the pattern (FR-005).

## Assembly (008, extended)

Before the existing read loop, `assemble`:

1. Expands every glob locator to its sorted concrete locators.
2. Tracks the running total locator count; > `GROUNDED_VERIFY_MAX_LOCATORS` ⇒ loud error (FR-006).
3. Feeds the unified locator list (globs replaced by their expansions, paths as-is) into 008's per-locator read + manifest + total-byte-ceiling loop, unchanged.

Outputs are 008's: `AssembledEvidence { text, manifest }`. Each manifest entry
names a concrete file (never the pattern).

## Configuration

No new variables. `GROUNDED_VERIFY_MAX_LOCATORS` and `GROUNDED_VERIFY_MAX_BYTES`
(008) bound the expansion. `GROUNDED_VERIFY_ROOT` enables the tool.
