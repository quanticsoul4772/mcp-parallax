# Research: grounded_verify Refuses Files Whose Names Mark Them as Secrets

No open technical unknown blocks the build. This file records the decisions
taken, what was rejected, and the platform behaviours to be confirmed by test
while building. Second version, corrected after two reviews against the code.

## D1 — Where the rules live

**Decision**: one new module, `src/grounded/secrets.rs`, holding the rule
tables as constants and pure functions over text. It never touches the disk.

**Rationale**: the rules are the security-relevant part and should be
readable in one place and testable without a filesystem.

**Rejected**: putting the rules in `reader.rs` (already 619 lines, and it
would tie rule tests to temporary folders); a configurable list (the
specification forbids a setting).

## D2 — Testing Windows rules on Linux

**Decision**: the text functions take the platform as an argument.
Production passes the real one; tests pass both.

**Rationale**: pull-request checks run on Linux only. Earlier on the same day
a test that asserted a Windows-only behaviour passed locally and failed
there. Passing the platform in makes the separator, leading-form and colon
rules checkable on every run.

**Rejected**: `cfg!(windows)` inside the functions.

## D3 — Splitting the reader seam

**Decision**: `SourceReader::read` is replaced by `check` and `read_checked`.
`list_files` flags refused entries. Both return a failure of one of two
kinds, refused by a rule or any other fault, so assembly never has to read an
error's wording.

**Rationale**: every exact path must be checked before any file is read.
With one combined call the first file is read before the second is examined.

**The test that proves the split is needed**: an ordinary file first, then a
file named through a link into a refused folder. The second is refused only
by what it resolves to, so only a design that checks all before reading any
reads nothing. A test with `.env` second would pass without the split,
because the text check alone catches it.

**The check is repeated at read time.** With all checks done first, an exact
path would otherwise wait longer between check and read than it does today.
`read_checked` therefore repeats the confinement and rule checks immediately
before opening. This narrows the window to the instant between that check
and the open. It does not close it; the specification states that as a limit.

**Rejected**: keeping `read` and adding a separate "would this be refused"
call, which would let the two drift.

## D4 — What a glob does with a refused file

**Decision**: leave it out; list it (at most 50) and count it; set a flag;
write a fixed notice; skip arithmetic settlement. The confidence number is
not changed.

**History**: the owner first chose leave-out-and-list over failing the call.
Review then showed a verdict could come back at full confidence over reduced
evidence, and the owner chose to add a notice and to lower confidence. A cap
was specified; review showed it changed only unanimous results while the
text said confidence was always lowered. Offered the alternatives, the owner
chose to leave the number alone and add a flag.

**Why the number is left alone**: it is defined, in the code and in the
published contract, as the share of completed passes that agreed. Any rule
that changed it would either not do what was intended or stop it meaning
that.

**A countable claim** over a reduced glob cannot "go to the judging passes":
they have already answered that it needs computing. With settlement skipped
it takes the existing exit, inconclusive with the reason "computable
property — route to `check`". The notice then adds how to get it settled:
name the file by its exact path.

## D5 — Order of checks, and what stays the same

**Decision**: the order in FR-002. Evidence stays in the order the locators
were given; the ceiling, its error text and its running count stay exactly as
today.

**Consequence, stated in FR-014**: when a call has more than one fault, which
is reported can differ from today. No error text changes.

**A glob that plainly starts in a refused folder** (`.ssh/*`) is refused from
its text in phase one. Otherwise the folder would be walked and up to 50 of
its file names returned under `refused`.

## D6 — Behaviours to confirm by test while building

Each is exercised on a real disk, through assembly with the real reader.

1. **An alternate data stream** is refused by rule 8 on the text. The test
   first writes a stream and reads it back with the standard library, so it
   fails if a stream cannot be made. Whether today's reader, given a stream
   name, returns the stream or the base file is recorded while building.
2. **A `\\?\C:\` path to an ordinary file is read**, with each slash
   direction. The backslash form has an existing test; the forward-slash form
   has none and is confirmed here.
3. **A full path to a refused file with a trailing dot, and with a trailing
   space**, is refused. Windows trims these, so the name as written does not
   match a rule; the resolved name must. If `canonicalize` does not return
   the trimmed on-disk name, the finding goes back to the owner.
4. **A folder junction with an ordinary name into a refused folder inside the
   root.** To confirm: `canonicalize` returns the target's real name.

Not testable on the machines available, and recorded in the contract as
unverified: a case-insensitive disk on macOS; a root on a network share.

## D7 — Keeping the lists honest

**Decision**: three bindings.

1. Tests iterate the code's tables.
2. A test parses the fenced block in the contract page and compares it with
   the tables in both directions; it fails on any line it cannot classify and
   asserts the number of lines of each kind.
3. A separately typed list of at least twenty refusals, covering every rule,
   including rule 8, which has no table line, is asserted directly.

**Rationale**: the first two prove the tables agree with themselves and with
a document; neither can catch a wrong table. The third is the only check
that is not derived from the thing it checks.

## D8 — The startup refusal

**Decision**: a new `ConfigError` variant carrying the folder name and the
root. One existing variant carries free text, but under a prefix about routing,
which would be wrong here.

## D9 — No new dependency

Listing uses `walkdir`, already present. Junction creation in the Windows
test shells out to the system's `mklink`, in test code only.
