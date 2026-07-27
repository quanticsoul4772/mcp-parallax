# Quickstart: What You See When This Fails

**Feature**: 040 | **Date**: 2026-07-27

You will only ever meet this check by breaking it. Three ways that happens, and
what to do about each.

## You changed a default

`cargo test` fails:

```text
DEFAULT_MISMATCH: `RESEARCH_CONCURRENCY` applies 8.
  README.md   says 16
```

Update the document. That is the entire purpose — the code moved and the
document did not.

## You added a configuration variable

```text
DEFAULT_UNDOCUMENTED: `FETCH_RETRY_MS` carries a default that README.md
  and --help do not state.
```

Add it to both. The check does not write documentation for you: the *Purpose*
column and the help description are reasons, and reasons are hand-written.
Only the facts are derived, and only the facts are checked.

## You wrote a default in a new shape

```text
DEFAULT_UNRESOLVED: `FETCH_RETRY_MS` has a default this check cannot read.
  found:   parse_env("FETCH_RETRY_MS", compute_backoff())
  handled: numeric literal, string literal, named constant
```

Two remedies, and picking one is a decision rather than a chore:

- **Teach the resolver the shape**, if it is one that will recur.
- **Add the variable to `EXCLUSIONS` with a reason**, if it will not.

```rust
const EXCLUSIONS: &[(&str, &str)] = &[(
    "FETCH_RETRY_MS",
    "computed from the deadline at startup; there is no literal to compare",
)];
```

What you cannot do is nothing. That is the whole feature: before it, a default
the scan could not read was passed over in silence, and two operator-facing
documents contradicted the code for two releases with every test green.

## Adding a third document

Call the resolver. Do not copy an existing check — that is how the second one
inherited the first one's blind spot, then quietly grew two guards the first
still lacks.

```rust
let facts = config_facts::resolve();
config_facts::assert_document_agrees(&facts, "CHANGELOG.md", &extracted);
```

## Why the exclusion list is short

It should stay short, and it should be read as a list of decisions rather than
a list of exceptions. Every entry says a person looked at a default, judged it
not worth teaching the resolver, and wrote down why. An entry with a reason
like "TODO" or "does not parse" is the failure this feature exists to prevent,
wearing the fix as a costume.
