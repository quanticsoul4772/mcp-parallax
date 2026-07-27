# Contract: Every Way This Check Fails, And What It Says

**Feature**: 040 | **Date**: 2026-07-27

The contract of a check is its **failure surface**. A contributor meets this
code exactly once — when it fails — and what it says then is the whole
interface. This enumerates every failure, because the defect being fixed is a
failure that had no message at all.

## The failures

### 1. A default cannot be resolved — FR-001

The core requirement. Fires when a default is written in a form the resolver
does not handle.

```text
DEFAULT_UNRESOLVED: `FETCH_TIMEOUT_MS` has a default this check cannot read.
  found:    parse_env("FETCH_TIMEOUT_MS", compute_timeout())
  handled:  numeric literal, string literal, named constant
  Either teach the resolver this shape, or add the variable to EXCLUSIONS
  with a reason. It must not be skipped: a scan that skipped what it could
  not read is why this check exists.
```

Names the variable, quotes what it found, lists what it handles, and states
both remedies. It does not guess which remedy applies.

### 2. A named constant is not in the declared file set — FR-004b

```text
CONSTANT_NOT_FOUND: `GROUNDED_VERIFY_MAX_BYTES` defaults to the constant
  `DEFAULT_GROUNDED_VERIFY_MAX_BYTES`, which is not declared in any file this
  check reads.
  searched: src/config.rs, src/client/anthropic.rs
  If the constant lives elsewhere, add that file to SOURCES.
```

**It does not name the file the constant lives in.** The resolver read only the
listed files, so it does not know. Saying otherwise would require the
whole-crate search this design rejects (research D2), and the confirmation
`verify` refuted a variant that claimed it could.

### 3. A document states the wrong default — FR-006

```text
DEFAULT_MISMATCH: `RESEARCH_CONCURRENCY` applies 8.
  README.md      says 16   | `RESEARCH_CONCURRENCY` | no | `16` | ...
  --help         says 8    RESEARCH_CONCURRENCY  ... (default: 8)
```

Names every document, so it is visible when one is right and another wrong —
the case where a fix landed in one place and not the other.

### 4. A document omits a variable — FR-006

```text
DEFAULT_UNDOCUMENTED: `MAX_RETRIES` carries a default that README.md
  does not state.
```

### 5. A document states a default for a variable that has none — FR-009

The reverse direction.

```text
DEFAULT_PHANTOM: README.md states a default for `FETCH_RETRY_MS`, which the
  configuration does not read.
  found: | `FETCH_RETRY_MS` | no | `500` | ...
  Either the variable was removed and the row outlived it, or the row was
  written for a variable that never existed.
```

Read from the Default column only. A number in the Purpose column produces
nothing (FR-010).

### 6. An exclusion has outlived its subject — FR-003

```text
EXCLUSION_STALE: EXCLUSIONS names `FETCH_TIMEOUT_MS`, which no longer carries
  a default. Remove the entry.
```

Without this, a suppression added for one release becomes permanent by
inattention.

### 7. Coverage does not balance — FR-007

```text
COVERAGE_UNBALANCED: resolved 17 + excluded 1 != 20 variables carrying a
  default. Three were neither resolved nor excluded, which means the loop
  exited early or a variable was matched twice.
```

An equation rather than a threshold. The floor it replaces (`checked >= 8`)
passed while three of four shapes went unexamined, because the literal-valued
variables cleared it alone.

### 8. Document extraction found nothing — the 039 failure

```text
EXTRACTION_EMPTY: found 0 rows in README.md's configuration table; expected
  more than 15. The boundary search is wrong, not the document.
```

039 shipped a version that searched for a blank line in a CRLF file, matched
nothing, extracted an empty table, and reported **every variable missing** —
blaming the document for its own parsing bug. This asserts the extraction
worked before trusting anything it produced.

## Invariants

1. **No silent skip.** Every variable carrying a default ends as resolved,
   excluded, or a failure. There is no fourth outcome.
2. **The message claims only what was searched.** Nothing tells a contributor
   where something is unless the check looked there.
3. **Prose is never parsed.** The reverse direction reads structured markers.
   Hand-written reasoning is out of bounds, so it cannot generate a false
   positive that gets the check silenced.
4. **Every document gets the same resolution.** One resolver, called by each.
   Adding a document means adding a caller, not copying a scan.

## Every mode observed firing

Run 2026-07-26. Each mode was mutated into existence one at a time, the suite
run, the message captured, and the file restored. A failure surface nobody has
seen fire is a claim, not a check.

| # | Message | Mutation that produced it |
|---|---|---|
| 1 | `DEFAULT_UNRESOLVED` | `parse_env("MAX_RETRIES", 3)` → `2 + 1` |
| 2 | `CONSTANT_NOT_FOUND` | `SOURCES` entry for `anthropic.rs` emptied |
| 3 | `DEFAULT_MISMATCH` | README `MAX_RETRIES` default `3` → `99` |
| 4 | `DEFAULT_UNDOCUMENTED` | `(default: 3)` removed from `--help` |
| 5 | `DEFAULT_PHANTOM` | README row added for a nonexistent `FETCH_RETRY_MS` |
| 6 | `EXCLUSION_STALE` | `EXCLUSIONS` name changed to `GONE_AWAY` |
| 7 | `COVERAGE_UNBALANCED` | extraction loop capped at 9 facts |
| 8 | `EXTRACTION_EMPTY` | table boundary search truncated to 4 rows |

### Two defects this sweep found

**Mode 3 did not fire on the first run, and the reason was the comparison.**
The forward pass joined a **six-line window** around the variable's name and
asked only whether the value appeared *somewhere in it*. A single-digit default
like `3` matches any neighbouring row containing that digit, so setting
`MAX_RETRIES` to a wrong `99` passed. `GROUNDED_VERIFY_MAX_BYTES` → `999999`
had fired only because six digits are rare enough not to collide — the check's
sensitivity depended on how unusual the value was, which is not a check. Both
documents carry a structured default marker (the README's Default column,
`--help`'s `(default: X)`), so the window was replaced with exact per-variable
comparison against those markers. This preserves FR-010: still one structured
marker per document, never surrounding prose.

Exact comparison immediately surfaced two documentation gaps the window had
hidden: `--help` stated no default at all for `RESEARCH_CONCURRENCY`, and
`VERIFY_MAX_CLAIM_CHARS` carries its own default that neither document states.

**Mode 7 could not fire as originally written.** `assert_coverage_balances`
compared `resolved + excluded` against `facts.len()` — the same vector both
sides. An extractor that dropped a variable shrinks both sides equally and the
equation still balances, so it could not detect the early-exit case the contract
above says it detects. `assert_extraction_is_complete` now compares against
`default_bearing_call_sites`, a count taken from the call markers alone that
shares no code with pair extraction — no quote scanning, no paren matching, no
statement bounding. The two agree at 16.

The through-line: both defects were checks that passed for reasons unrelated to
what they claimed to verify. That is the same shape as the silent `continue`
this feature was written to remove, which is why firing every mode was worth
doing rather than assuming the messages worked.

## What the pre-merge review changed (T033)

The review was pointed at one thing: find where the resolver returns a **wrong
value** rather than a failure, because a confidently wrong answer is strictly
worse than the silent skip this feature replaced. It found four, and they shared
one root cause — **resolution succeeded on a prefix of what it read instead of
requiring it consumed the whole expression.**

| Source shape | Was reported | Now |
|---|---|---|
| `RESEARCH_CONCURRENCY_MAX / 4` | `Resolved("32")` for a default of 8 | `Unresolvable` |
| `3u32`, `0x40000`, `1 << 18` | `332`, `040000`, `118` | parsed and re-formatted, or `Unresolvable` |
| a doc comment quoting an old `const` | the historical value | the real declaration |
| `crate::client::anthropic::API_BASE` | first file declaring the name | the file the path names |
| `// Was parse_env("X", 5)` in a comment | a second fact for `X` | comments stripped first |
| `const KB` (2 chars) | `Unresolvable`, advising a shape already handled | resolved |

Each ended the same way: every coverage invariant balanced, and the suite then
failed **naming the documents as wrong**. The cheapest path to green is to copy
the fabricated value into `--help` and `README.md`, which leaves a green suite
and two corrupted documents. 036's skip left them merely unchecked; this would
have left them actively wrong. That is why prefix-resolution had to go rather
than be special-cased shape by shape.

Three further findings, each a check passing for a reason unrelated to what it
claimed:

- **The cross-check was not independent of the population.** It counted the two
  shapes extraction recognises, so a third shape
  (`.ok().and_then(...).unwrap_or(5_000)`) would be invisible to extraction and
  to its own cross-check at once — the 036 defect rebuilt one level up. Every
  read is now classified by a positive rule, and one matching no rule raises
  `UNKNOWN_CALL_SHAPE` rather than being counted either way.
- **A `(default:)` marker could bind to the wrong variable.** Any uppercase word
  on a continuation line became an entry, so a description reading "API waits
  this long before failing (default: 120000)" would capture the marker under
  `API` and report the real variable as undocumented in a document that
  documents it correctly. Entry names are now anchored to their column.
- **`stated.len() >= 10` was the `checked >= 8` shape this feature abolishes**,
  reinstated one file over. Removed; `EXTRACTION_EMPTY` already covers it.

Two duplications were removed rather than documented: `main.rs` held a second
`LOG_LEVEL` default duplicating `config.rs` (both now use one named constant),
and the FR-008 test listed five variable names by hand inside the feature that
exists to abolish hand-written lists (now derived).

The review confirmed the one `EXCLUSIONS` entry is genuine rather than a
suppression: `VERIFY_MAX_CLAIM_CHARS` is excluded from the *document*
comparison only, and the property that would actually break — the alias drifting
from `INPUT_MAX_CHARS` — is asserted directly.

All eight messages above were re-fired after the split. All eight still fire.
