# Contract: glob locator (009, extends grounded-verify)

Adds a glob shape to the `grounded_verify` tool's `locators`. Present only when
`GROUNDED_VERIFY_ROOT` is configured (008 gating, unchanged).

## Locator input shapes

```jsonc
// 008 — unchanged:
{ "path": "src/server/record.rs" }
{ "path": "src/server/record.rs", "start_line": 55, "end_line": 111 }

// 009 — new:
{ "glob": "src/**/*.rs" }
{ "glob": "src/modes/{verify,grounded_verify}.rs" }
{ "glob": "tests/**/!(*_bench).rs" }
```

- Exactly one of `path` / `glob` per locator.
- `start_line`/`end_line` only with `path`. A `glob` with a range → error.

## Grammar (full extended globbing)

`*`, `**`, `?`, `[class]` / `[!class]`, brace `{a,b}` (nestable), extglob
`@(...)` `?(...)` `*(...)` `+(...)` `!(...)`. Patterns are matched against each
file's path relative to the configured root.

**Amended 2026-11.** A glob is searched only where it points: in the folder it
starts with (`src/*.rs`), in each folder of a leading brace list
(`{src,tests}/**/*.rs`), or, with no `/`, among the root's own files (`*.md`).
A glob that would search the whole root is refused: `**/x.py`, a leading `!`
(whole-pattern negation, no longer accepted), or a folder part that is `.`,
`..` or absolute. Use `!(...)` inside a folder instead (`src/!(test)*.rs`).

## Expansion behaviour

- A glob expands, server-side, to the **sorted** set of matching files within
  the root; each becomes its own read span and manifest entry naming the
  concrete file (not the pattern).
- Deterministic: the same glob over unchanged files → byte-identical evidence.
- Confined: the walk does not follow symlinks out of the root; every match is
  re-checked against the canonical root before reading.
- All-or-nothing: any expanded file that fails 008's guards (non-text,
  unreadable, over a ceiling) fails the whole call. 055: a match whose name
  marks it as a likely secret is left out and listed in `refused` instead.

## Output

Unchanged from 008 — `verdict`, `confidence`, `findings`, `missing_evidence`,
and `manifest` (one entry per expanded file). No model-schema change. 055 adds
`refused`, `refused_count`, `evidence_reduced` and `evidence_note`.

## Errors (`invalid_params`, naming the offending pattern)

| Condition | Error |
|---|---|
| glob matches zero files | `[invalid_input] glob matched no files under the source root <root>: <pattern>` |
| glob has a `/` but no plain folder name in front (its first part is a pattern, a one-name brace list, or an extglob group), or has no `/` and contains a backslash | `[invalid_input] glob needs a folder in front: <pattern> - start it with the folder to search, using / between folders, for example src/*.rs` |
| glob's folder turns out to be the root itself (for example a link back to it) | `[invalid_input] the glob's folder resolves to the source root itself: <folder> - name a folder inside the root` |
| glob folder is `.`, `..`, empty or absolute, a folder name contains `:`, or the glob starts with a drive letter or a backslash | `[invalid_input] a glob must be relative to the source root and start with a plain folder name (no ., .. or absolute path): <pattern> - for example src/*.rs` |
| glob starts with `!` | `[invalid_input] a glob starting with ! (everything except a pattern) is not accepted: it would search the whole source root - name a folder and use !(...) inside it, for example src/!(test)*.rs` |
| glob + line range | `[invalid_input] a line range is not allowed with a glob: <pattern>` |
| neither path nor glob | `[invalid_input] locator must give a path or a glob` |
| both path and glob | `[invalid_input] locator cannot give both a path and a glob` |
| malformed pattern | `[invalid_input] malformed glob pattern: <pattern> (<reason>)` |
| expansion over the locator ceiling | `[invalid_input] too many locators (<n>, max <max>) after glob expansion` |
| expansion over the byte ceiling | `[invalid_input] assembled evidence exceeds <max> bytes` (008) |
| (055) glob plainly starts in a secret folder | `[invalid_input] invalid input: glob refused: <pattern> - it searches a folder whose name marks its files as likely secrets` |
| (055) every match of a glob is refused as a likely secret | `[invalid_input] invalid input: glob matched only files refused as likely secrets (<n>): <pattern>` |

## Invariants

- 008 path/range locators behave byte-identically (FR-009); their tests pass
  unmodified. (055: they moved onto the split reader with the same assertions;
  an exact path to a secret-named file now fails the call.)
- One `InvocationRecord` per call (tool `grounded_verify`), OTLP-exported when
  telemetry is configured.
