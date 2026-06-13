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
`@(...)` `?(...)` `*(...)` `+(...)` `!(...)`, and a leading `!` to negate the
whole pattern. Patterns are matched against each file's path relative to the
configured root.

## Expansion behaviour

- A glob expands, server-side, to the **sorted** set of matching files within
  the root; each becomes its own read span and manifest entry naming the
  concrete file (not the pattern).
- Deterministic: the same glob over unchanged files → byte-identical evidence.
- Confined: the walk does not follow symlinks out of the root; every match is
  re-checked against the canonical root before reading.
- All-or-nothing: any expanded file that fails 008's guards (non-text,
  unreadable, over a ceiling) fails the whole call.

## Output

Unchanged from 008 — `verdict`, `confidence`, `findings`, `missing_evidence`,
and `manifest` (one entry per expanded file). No model-schema change.

## Errors (`invalid_params`, naming the offending pattern)

| Condition | Error |
|---|---|
| glob matches zero files | `[invalid_input] glob matched no files: <pattern>` |
| glob + line range | `[invalid_input] a line range is not allowed with a glob: <pattern>` |
| neither path nor glob | `[invalid_input] locator must give a path or a glob` |
| both path and glob | `[invalid_input] locator cannot give both a path and a glob` |
| malformed pattern | `[invalid_input] malformed glob pattern: <pattern> (<reason>)` |
| expansion over the locator ceiling | `[invalid_input] glob '<pattern>' expands to N files, over the limit of <max>` |
| expansion over the byte ceiling | `[invalid_input] assembled evidence exceeds <max> bytes` (008) |

## Invariants

- 008 path/range locators behave byte-identically (FR-009); their tests pass
  unmodified.
- One `InvocationRecord` per call (tool `grounded_verify`), OTLP-exported when
  telemetry is configured.
