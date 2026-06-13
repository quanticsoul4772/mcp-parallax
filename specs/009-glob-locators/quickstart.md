# Quickstart: glob locators (009)

Requires `grounded_verify` enabled (008): `GROUNDED_VERIFY_ROOT` set. No new
configuration.

## Verify against a pattern-matched set

```jsonc
// tools/call grounded_verify
{
  "claim": "every mode registers a flat, closed output schema",
  "locators": [ { "glob": "src/modes/*.rs" } ]
}
```

The server expands `src/modes/*.rs` to the matching files (sorted), reads each
verbatim, judges the claim over all of them, and returns a manifest with one
entry per expanded file:

```jsonc
{
  "verdict": "supported",
  "confidence": 1.0,
  "findings": ["..."],
  "missing_evidence": [],
  "manifest": [
    { "path": "src/modes/grounded_verify.rs", "bytes": 9210 },
    { "path": "src/modes/mod.rs",            "bytes": 8430 },
    { "path": "src/modes/unstick.rs",        "bytes": 6120 },
    { "path": "src/modes/verify.rs",         "bytes": 7050 }
  ]
}
```

## Extended grammar

```jsonc
{ "glob": "src/**/*.rs" }                              // recursive
{ "glob": "src/modes/{verify,grounded_verify}.rs" }   // brace alternation
{ "glob": "tests/**/!(*_helpers).rs" }                // extglob negation
```

Mix globs and exact paths in one call; the exact-path and line-range shapes from
008 are unchanged.

## Failure is loud and all-or-nothing

```jsonc
{ "claim": "...", "locators": [ { "glob": "src/nope/*.rs" } ] }
// => error: [invalid_input] glob matched no files: src/nope/*.rs

{ "claim": "...", "locators": [ { "glob": "*.rs", "start_line": 1, "end_line": 5 } ] }
// => error: [invalid_input] a line range is not allowed with a glob: *.rs
```

A glob can never escape the root (the walk does not follow symlinks out, and
every match is re-checked), and the expansion is bound by
`GROUNDED_VERIFY_MAX_LOCATORS` and `GROUNDED_VERIFY_MAX_BYTES`.

## Acceptance

`cargo run --example acceptance_grounded_verify` covers the glob SCs:
expand-to-set + manifest, determinism across runs, zero-match error, ceiling
overflow error, and root confinement via a symlinked directory.
