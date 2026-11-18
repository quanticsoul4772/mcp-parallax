# Data Model: grounded_verify Refuses Files Whose Names Mark Them as Secrets

No persistence changes. The model is the rule tables, a few small types, and
four additions to the result.

## Rule tables (constants in `src/grounded/secrets.rs`)

The values are those of FR-001 in the specification. The contract page
carries them in full and a test holds the two equal in both directions.

| Table | Rule | Matches |
|---|---|---|
| `EXACT_NAMES` | 1 | the file name equals an entry |
| `NAME_PREFIXES` | 2 | the file name starts with an entry |
| `NAME_ENDINGS` | 3 | the file name ends with an entry |
| `NAME_CONTAINS` | 3 | the file name contains the entry (`.tfstate.`) |
| `BACKUP_ENDINGS` | 4 | removed repeatedly from the end of a name before the name is tested |
| `PART_NAMES` | 5 | any part of the path, folder or file, equals an entry |
| `GIT_CONFIG_NAMES` | 6 | the file name equals an entry and some part above it ends in `.git` |
| `PATH_SEQUENCES` | 7 | two adjacent parts equal an entry's pair |

Rule 8 has no table: on Windows, a colon in what remains after the leading
forms are set aside.

All comparisons ignore ASCII letter case. Backup endings are removed from
every part of the path before any of rules 1 to 3 and 5 to 7 is applied to it.

## Types

| Type | Where | Fields | Notes |
|---|---|---|---|
| `Platform` | `secrets.rs` | `Windows`, `Other` | passed to the text functions; production passes the real one |
| `Simplified` | `secrets.rs` | `parts: Vec<String>`, `colon_in_rest: bool` | the result of FR-001's simplification |
| `Rule` | `secrets.rs` | one variant per rule 1 to 8 | used in tests and logs; the caller is told only "its name marks it as a likely secret" |
| `Checked` | `traits/source.rs` | the caller's spelling; the resolved path | constructor visible within the crate only; made by a reader's `check`, consumed by `read_checked` |
| `CheckFailure` | `traits/source.rs` | `Refused(Rule)` or `Other(AppError)` | what `check` and `read_checked` return on failure; lets assembly tell a refusal from any other fault without reading its wording |
| `ListedFile` | `traits/source.rs` | `path: String` (from the root), `refused: bool` | what `list_files` returns |
| a new `ConfigError` variant | `error.rs` | folder name; root | the startup refusal; a configuration error, like a missing root |

## Result additions (`GroundedVerdict`)

| Field | Type | Always present | Meaning |
|---|---|---|---|
| `refused` | list of text | yes | paths, from the root, that a glob left out; sorted, no duplicates, at most 50 |
| `refused_count` | whole number | yes | how many distinct files were left out |
| `evidence_reduced` | true or false | yes | true exactly when `refused_count` is above zero |
| `evidence_note` | text | yes | empty, or the FR-008 sentence with the count, plus its closing sentence on the inconclusive exit |

Every existing field is unchanged, `confidence` included.

## Errors added

Call errors (`invalid_input`, before any file is read or any model contacted):

| Condition | Text after the server's `[invalid_input] invalid input:` prefix |
|---|---|
| an exact path is refused | `locator refused: <path as written> - it is, or resolves to, a file whose name marks it as a likely secret` |
| on Windows, a colon in the path (rule 8) | `locator refused: <path as written> - a colon is not accepted in a path; give line numbers as start_line and end_line` |
| a glob plainly starts in a refused folder | `glob refused: <pattern> - it searches a folder whose name marks its files as likely secrets` |
| every match of a glob is refused | `glob matched only files refused as likely secrets (<n>): <pattern>` |

Startup error (a configuration error, not `invalid_input`):

| Condition | Text |
|---|---|
| the root is, or is inside, a refused folder | `GROUNDED_VERIFY_ROOT is, or is inside, <name>, whose files are refused as likely secrets: <root>`, where `<name>` is the refused folder, or the two folders of a pair |

## Order of checks for one call

1. Claim checks and pass count, as today.
2. **No disk.** Per locator in order: shape; a glob's text validity; the name rules on the text (all of them on the whole path for an exact path; only the folder rules on the starting folder or folders of a glob).
3. **Disk.** `check` on every exact path, in order.
4. Globs expanded in locator order into one list of read units in locator order; refused matches set aside; the running total against the ceiling; zero-match and all-refused errors.
5. Reads in that order. Before each, the name is resolved and checked again and must lead to the same file.
