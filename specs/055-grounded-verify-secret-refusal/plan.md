# Implementation Plan: grounded_verify Refuses Files Whose Names Mark Them as Secrets

**Branch**: `055-grounded-verify-secret-refusal` | **Date**: 2026-11-17 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/055-grounded-verify-secret-refusal/spec.md` (fifth version)

**This is the second version of the plan**, corrected after two independent reviews of the first against the code.

## Summary

`grounded_verify` gains a fixed set of name rules. An exact path that matches
one fails the call before any file is read. A glob leaves matching files out,
lists and counts them, says so in a notice, sets a flag, and does not take
the arithmetic shortcut. The confidence number is not changed. No setting and
no dependency is added, and the judging passes' prompt and output shape do
not change.

The work is six pieces, each test-first, each ending with the full gate:

1. **The rules as pure text functions** in a new module, with the platform
   passed in as a value so the Windows rules are tested on every system.
2. **A new startup error kind**, and the startup refusal for a root inside a
   refused folder.
3. **The reader seam split** into "check" and "read".
4. **Assembly in the order the specification gives**, with globs leaving
   refused files out.
5. **The result**: list, count, flag, notice, no settlement; with the
   published contract file changed in the same step.
6. **Visibility**: description, the contract page and its binding test, the
   amended documents.

## Technical Context

**Language/Version**: Rust, toolchain pinned at 1.100.0 (MSRV 1.94).

**Primary Dependencies**: none added. `walkdir` (already used) for listing.

**Storage**: N/A.

**Testing**: `cargo test`, through the `SourceReader` and `ModelClient` seams, plus real-filesystem tests in temporary folders.

**Target Platform**: unchanged (stdio MCP server). The owner's machine is Windows; pull-request checks run on Linux only.

**Project Type**: single Rust crate.

**Performance Goals**: N/A. Each file is resolved twice instead of once (at check and again at read); both are local metadata lookups.

**Constraints**: no unsafe code; no new dependency; no change to the judging prompt, `GroundedPass` or the confidence number; for a call with nothing refused, behaviour is as before except which fault is reported when there is more than one (FR-014).

**Scale/Scope**: one new source module; eight existing source files changed (`error.rs`, `traits/source.rs`, `grounded/mod.rs`, `grounded/reader.rs`, `grounded/glob/expand.rs`, `grounded/assemble.rs`, `modes/grounded_verify.rs`, `server.rs`) plus `main.rs` help text and the acceptance example's header; `tests/integration.rs`; one contract file; and the documents of FR-012.

## Constitution Check

*GATE: evaluated before research and again after design. Both passes: no unjustified violation.*

| Principle | Assessment |
|---|---|
| I. Design-Corpus Fidelity | The tool stays a Verify-family corrective. The departure from all-or-nothing is named in the specification, and FR-012 item 1 amends the design document in the same change, with a dated block, as 054 did. |
| II. Constrained-Output Contract | Untouched: `GroundedPass`, its schema and the prompt do not change. The additions are to the server-assembled result. |
| III. Compiler-Enforced Discipline | No unsafe, no `unwrap` in production paths. Refusals are loud errors. The glob exception is reported four ways, never hidden. A root inside a refused folder is a startup error. |
| IV. Seams, Composition, and Tests | The reader seam changes shape and stays mockable. The rule tables are tested from themselves, against the contract page, and against a separately typed list. |
| V. Deterministic Over Probabilistic | The refusal is a deterministic name check. The flag and notice are server-assembled; the model is not asked whether evidence was left out, and the confidence number keeps its meaning. |
| VI. Capabilities Off By Default | No capability added. The tool stays gated on `GROUNDED_VERIFY_ROOT`. |
| VII. Simplicity and Scope Discipline | No setting, no override, no dependency. See Complexity Tracking. |

## Project Structure

### Documentation (this feature)

```text
specs/055-grounded-verify-secret-refusal/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/refusal.md
├── checklists/requirements.md
└── tasks.md             # written by /speckit-tasks
```

### Source Code (repository root)

```text
src/grounded/secrets.rs      # NEW: rule tables and pure functions over them
src/grounded/mod.rs          # declare the module; opening documentation
src/error.rs                 # NEW ConfigError variant for the startup refusal
src/traits/source.rs         # SourceReader: `read` -> `check` + `read_checked`; `list_files` marks refused entries
src/grounded/reader.rs       # the split; rules on resolved paths; startup refusal
src/grounded/glob/expand.rs  # refuse a glob that starts in a refused folder; partition matches
src/grounded/assemble.rs     # the FR-002 order; refused list and count
src/modes/grounded_verify.rs # result additions, notice, settlement gate; description
src/server.rs                # served description; server instructions text
src/main.rs                  # the tool's line in the help text
examples/acceptance_grounded_verify.rs  # header comment
tests/integration.rs
specs/008-grounded-verify/contracts/grounded_verify.tool.json
docs/design/NEW_SERVER_DESIGN.md, specs/008, 009, 011 documents,
SECURITY.md, README.md, CLAUDE.md, CHANGELOG.md
```

**Structure Decision**: the rules live in one new module with no filesystem access. Everything that touches the disk stays in `reader.rs`.

## Design

### 1. The rules (`src/grounded/secrets.rs`)

- The rule tables are `pub const`: exact names, prefixes, endings, the one "contains" entry, backup endings, part names, the two git-configuration names, and the two path sequences.
- `simplify(path, platform) -> Simplified` implements FR-001's simplification on text alone. `Simplified` holds the parts and whether a colon remains after the leading forms are set aside.
- `refusal(&Simplified, platform) -> Option<Rule>` applies rules 1 to 8. It takes the whole `Simplified`, so the colon rule has what it needs.
- `platform` is a two-valued enum passed by the caller. Production passes the real platform; tests pass both.
- For a glob, a small function returns the folder or folders its text plainly starts in, and only the folder rules (5 and 7, after rule 4's reduction) are applied to each. Glob expansion is split into a text stage, which phase one runs, and a listing stage; the scope logic is private to expansion today and moves into the text stage.

### 2. Startup (`src/error.rs`, `src/grounded/reader.rs`)

- A new `ConfigError` variant carries the refused folder name (for a two-folder rule, both names) and the root. One existing variant carries free text, but under a prefix about routing, which would be wrong here.
- `SystemSourceReader::new` applies rules 4, 5 and 7 to the root's resolved path and returns that error. The server already fails to start when the reader cannot be built; the existing test that a missing root is a configuration error is unchanged.

### 3. The reader seam (`src/traits/source.rs`, `src/grounded/reader.rs`)

- `check(&self, path) -> Result<Checked, CheckFailure>`: confinement as today (canonicalize, then prefix), then the rules on the whole resolved path. `CheckFailure` is one of two kinds: refused by a named rule, or any other fault carrying today's error. Assembly needs the difference and must not read it from an error's wording.
- `read_checked(&self, &Checked, range) -> Result<SourceContent, CheckFailure>`: first resolves the caller's spelling again, repeats the confinement and rule checks, and requires the result to be the same resolved path that `Checked` holds; then the size, text, empty and range checks as today.
- `Checked` holds the caller's spelling and the resolved path. Its constructor is visible within the crate, so the real reader and the test doubles can make one; nothing outside the crate can.
- `list_files` returns each file with a flag saying whether the rules refuse it.
- The check that a start and end line come together moves out of the reader into assembly's phase one; the range's validity is checked there too and again where the range is applied.

### 4. Assembly (`src/grounded/assemble.rs`)

In the order FR-002 gives:

1. **Phase one, no disk**: per locator its shape, a glob's text validity (the checks expansion makes today before listing), and the name rules on the text. First failure wins. The locator ceiling is not checked here; it stays exactly where it is today.
2. **Phase two**: `check` on every exact path, in order.
3. **Globs expanded** in locator order, building one list of read units **in the order the locators were given**, exactly as today. Refused matches go to the refused set and do not count toward the ceiling; the running total is checked against the ceiling as today, with the same error text. A glob whose matches are all refused fails the call.
4. **Reads, in that same order**: exact paths through `read_checked` with the `Checked` from phase two; glob matches through `check` then `read_checked`. A glob match refused by a rule at this point is moved to the refused set; if that leaves its glob with nothing to read, the call fails with the all-refused error naming that glob. A glob match that fails in any other way at this point fails the call. Refused matches are separated only after the pattern has been applied, so a file the pattern does not match is never listed.

`refused_count` is the number of distinct files in the refused set.

### 5. The result (`src/modes/grounded_verify.rs`)

- `GroundedVerdict` gains `refused` (sorted, at most 50), `refused_count`, `evidence_reduced` and `evidence_note`.
- When `refused_count > 0`: settlement is skipped, so a countable claim takes the existing inconclusive exit; the note is the FR-008 sentence, with its closing sentence added only when the claim would have been settled had nothing been left out (the same conditions settlement tests today, evaluated without the left-out check).
- `confidence` is not touched.
- The published contract file gains the four fields **in this same step**, because an existing test compares the result's field names with that file.

### 6. Tests that need a real disk

These go through `assemble` with the real reader, so the text check and the resolved-path check are both in the path under test:

- Windows: an alternate data stream; the `\\?\C:\` form with each slash; a trailing dot and a trailing space on a full path to a refused file; a folder junction (made with the system's `mklink /J`, which needs no special rights) into a refused folder.
- Elsewhere: a symbolic link to a refused folder and to a refused file; a file whose name contains a colon.
- The stream test first writes a stream and reads it back with the standard library directly, which proves a stream exists on this disk and fails with a message if it cannot; it then shows the call refused. What today's reader does when given a stream name (return the stream, or the base file) is recorded while building; either way the rule refuses it from its text.
- A junction inside a folder that a glob walks is tested too: the walk must not descend through it.
- A test that cannot create its link or stream fails with a message; it does not skip.

Pull-request checks run on Linux, so the Windows tests run only through the local gate on Windows. The pull request states which tests ran where.

## Existing tests the seam split touches

Changing `read` and the return type of `list_files` breaks these at compile time. Each keeps proving what it proved; the task list carries them one by one (SC-007).

| Where | Tests | What changes |
|---|---|---|
| `src/traits/source.rs` | the mock contract test | call shape |
| `src/grounded/assemble.rs` | all ten | mock expectations move from `read` to `check` and `read_checked` |
| `src/grounded/glob/expand.rs` | all | the listing mock returns flagged entries; assertions on the returned files compare the file half of the result |
| `src/modes/grounded_verify.rs` | about seventeen | mock expectations, as for assembly |
| `src/grounded/reader.rs` | thirteen tests that call `read` directly | call shape, and where the asserted error comes from: not-found, escape, the network screen and the link escape now come from `check`; size, text, empty and range errors from `read_checked`. The task list names each test with the step its error comes from |
| `src/grounded/reader.rs` | the two listing tests | compare the path half of the flagged entries |
| `src/grounded/reader.rs` | the single-bound test | **meaning changes**: the pairing check moves to assembly, so this test moves there with it |
| `src/modes/grounded_verify.rs` | the empty-claim test | **meaning changes**: "nothing was touched" must now be asserted of both `check` and `read_checked` |
| `src/modes/grounded_verify.rs` | the contract-file test | the four additions; it compares field names and numeric limits only, so the contract file's list of required fields is checked by hand in the task list |

## Order of work and gates

| Step | Work | Ends with |
|---|---|---|
| 1 | Rule tables and pure functions; tests from the tables, both platforms; the separately typed list; the must-read list | full gate |
| 2 | The startup error kind and the startup refusal | full gate |
| 3 | Split the reader seam; move the existing tests as the table above says | full gate; a per-file account that only call shapes moved, apart from the two named tests |
| 4 | Assembly in the FR-002 order; glob text refusal; leave-out; the test that an ordinary file followed by a link into a refused folder reads nothing | full gate |
| 5 | Result additions and settlement gate, **with the published contract file** | full gate. Steps 4 and 5 are committed together: between them a glob would drop files without saying so |
| 6 | Real-disk tests | full gate on Windows, and on Linux through the pull-request checks |
| 7 | Description in both copies; end-to-end tests | full gate |
| 8 | The contract page and its binding test; the documents of FR-012; change log | full gate and the markdown check |
| 9 | Independent review of the whole diff; then the owner's decisions below; then commit, push and pull request, each asked | — |

## What this plan does not decide

- Whether to add a Windows job to the pull-request checks (a long build, since the solver dependency is compiled from source).
- Whether a live call is made before the pull request. The project's note says a model-facing description or schema change must pass the live example first; this change is one; the tool's existing acceptance example is the live vehicle. Asked of the owner with its price.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| `src/grounded/reader.rs` is 619 lines and `src/modes/grounded_verify.rs` 1,099, both already over the 500-line target; each grows | The split seam and the result additions belong beside the code they change | The rules themselves go in a new small module; splitting the two existing files as well would bury a security change inside a refactor |
