# Phase 1 Data Model: An Unresolvable Default Fails Instead of Being Skipped

**Feature**: 040 | **Date**: 2026-07-27

## Entities

### Default

What the configuration applies when a variable is unset. Four expressions, all
of which must resolve to a comparable value:

| Shape | Written as | Resolves by |
| --- | --- | --- |
| Numeric literal | `parse_env("X", 8)` | reading the literal, stripping `_` separators |
| Named numeric constant | `parse_env("X", DEFAULT_MAX_BYTES)` | looking the name up in the declared file set |
| Inline string literal | `unwrap_or_else(\|_\| "info".to_string())` | reading the literal |
| Named string constant | `unwrap_or_else(\|_\| DEFAULT_MODEL.to_string())` | looking the name up in the declared file set |

A fifth shape will appear eventually. It resolves to **Unresolvable**, which is
a failure rather than a skip — that is the feature.

### Resolution

The outcome of examining one variable. Three states, and the third is why this
feature exists:

| State | Meaning | Effect |
| --- | --- | --- |
| Resolved | a comparable value was obtained | compared against every document |
| Excluded | named in the exclusion list with a reason | not compared; counted separately |
| Unresolvable | neither of the above | **fails**, naming the variable and what was found |

There is deliberately no fourth state for *skipped*. The scan this replaces had
one and did not report it.

### Exclusion

A variable the resolver does not resolve, paired with a written reason. A `const`
array of pairs (research D4). Zero or one entries expected.

| Property | Rule |
| --- | --- |
| Variable | must exist in the configuration source **and carry a default** — FR-003 |
| Reason | hand-written prose; no source to derive it from |

An exclusion whose variable no longer carries a default fails. A suppression
list that outlives its subject is how suppression becomes permanent.

### Structured default marker

Where a document states a default, as distinct from where it discusses one.

| Document | Marker | Not read |
| --- | --- | --- |
| README configuration table | the Default column | the Purpose column |
| `--help` | `(default: X)` | the surrounding description |

The reverse direction reads only these (research D3).

## Rules

### Forward — every variable's default is stated correctly

For each variable carrying a default: resolve it, then for each document, find
the variable's entry and require the resolved value to appear in it. A variable
absent from a document is a failure; a variable present with a different value
is a failure naming both.

### Reverse — every stated default belongs to a variable that has one

For each structured default marker in each document: the variable it belongs to
must exist and must carry a default. A marker for a variable the configuration
does not read, or reads without a default, fails.

### Coverage is a count, not a floor

The check reports how many variables it resolved. The number this replaces was
`checked >= 8` — a floor the literal-valued variables clear on their own, so it
measured effort rather than coverage and passed while three shapes went
unexamined.

```text
resolved + excluded == variables carrying a default
```

Stated as an equation rather than a threshold, so it cannot pass while
examining fewer than everything.

## Validation rules

| Rule | Source | On failure |
| --- | --- | --- |
| Every default resolves or is excluded | FR-001 | fail, naming the variable and the expression found |
| An exclusion carries a reason | FR-002 | compile error — the pair type has no default |
| An exclusion's variable still carries a default | FR-003 | fail, naming the stale entry |
| Constants resolve only from the declared file set | FR-004a | fail, naming the constant, the variable, and the set |
| Failure messages claim only what was searched | FR-004b | — |
| Every document is checked by the same resolution | FR-006 | — |
| Coverage is reported as a count | FR-007 | fail if the equation does not balance |
| Reverse direction reads markers, never prose | FR-010 | — |

## What has no state here

Variables read with no default — absence gates a capability rather than
selecting a value. They never enter resolution and remain subject to the
existing must-be-listed assertions.
