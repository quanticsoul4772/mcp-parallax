# Changelog

All notable changes to mcp-parallax are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Per Keep a Changelog 1.1.0, this is the next-up change block; it persists
verbatim until the project's next SemVer cut, at which point the entries move
into a dated `## [X.Y.Z] - YYYY-MM-DD` section and this header starts the next
arc.

### Fixed

* **A wholly failed research phase billed every failed call twice (051)** — a
  run whose extraction or verification phase failed outright recorded
  `prior_phases + 2 × failure_tokens`. Two token disciplines meet on that path,
  each correct alone, neither aware of the other:

  * The per-source and per-claim `Err` arms add the failed call's tokens to the
    run meter, because a truncation or refusal is a 200 the provider charged
    for (020).
  * `dominant_failure_metered` sums `billed()` across the failure set, so the
    class vote cannot decide how much spend gets reported (also 020).

  `run_at` then closes with `error.metered(meter…)`, and **`AppError::metered`
  adds** — it does not replace. The failure tokens were in both terms.

  Both pipeline sites now call a new `verify::dominant_failure_unmetered`,
  which picks the dominant class and strips usage, leaving the run meter as the
  single authority. `dominant_failure_metered` keeps its summing and delegates
  the rooting to the new function: it remains correct for `verify` and
  `diverge`, whose ensemble passes have no meter outside their own errors.
  Changing `metered` to replace would have broken those.

  **The existing test on this path asserted `input_tokens > 0`.** A doubled
  bill is greater than zero, and that test exercises the search-phase failure,
  which never reaches this aggregation at all. The two new tests assert exact
  totals — `(24, 11)` for extraction, `(41, 19)` for verification — and are
  mutation-verified against the reintroduced defect, which produces `(38, 17)`
  and `(62, 28)`.

  The verification test runs at `Depth::Deep` deliberately: `verify_k` is
  depth-derived and `Quick` is 1, where summing and not summing agree, leaving
  the ensemble's own aggregation unexercised.

  Symptom while it stood: a research run that failed a whole phase reported
  roughly twice its true cost, in the invocation record and in the OTLP metrics
  derived from it.

* **`config_facts`'s tests move to their own file, and 0.5.0's line counts for
  that split were wrong (049)** — `src/config_facts/mod.rs` stood at 848 lines
  against the 500-line target: 301 of production and 546 of tests. The test
  module is now `src/config_facts/tests.rs`, leaving mod.rs at 304. The same
  split `src/research/pipeline.rs` already uses, without its `#[path]`
  attribute — `config_facts` is a directory module, so `mod tests;` finds the
  file unaided.

  **The 0.5.0 entry describing the original split reported `247/476/185 lines`.
  Two of those three were right, and the wrong one was wrong the day it was
  written.** At that commit `source.rs` was 476 lines and `documents.rs` 185 —
  exact, and both are total counts. `mod.rs` was **765**, not 247. It had also
  reached 260 lines of production, so 247 matched neither measure. Presenting
  one production-ish figure and two totals under a single label read as
  consistent precisely because the two files where the measures differ are the
  two with no tests in them.

  Released sections are not rewritten, so the 0.5.0 text stands and this entry
  is the correction. **The replacement is to stop stating the numbers**, not to
  restate them correctly: line counts are the one fact in that sentence nothing
  binds, and they were stale within a commit of being written. That the split
  happened is the durable claim; `wc -l` answers the rest.

  **This does not reduce how many files exceed the target — it moves which
  one does.** `tests.rs` lands at 554, still over; the count of files above 500
  under `src/` is 23 before and after. What changed is that the production
  module is now readable in one screenful and the overage sits in a test file,
  next to `src/research/pipeline_tests.rs` at 1410 — the standing precedent
  that the target is not applied to test files.

  No check is added, and one would fail on contact against those 23 files. This
  is one module brought under a target the repository states and does not
  enforce, not enforcement of it.

* **One list of test-only variable names (048)** — `PARALLAX_TEST_DEFINITELY_UNSET_KEY`
  and `PARALLAX_MODEL_NOT_A_CALL_SITE` were spread across four files, and
  **two of those were skip rules that had to agree**: `resolve_from` skipped one
  name, and the call-site counter subtracted the same name in a different
  function.

  Adding a second fixture key and updating only one of those would have made the
  resolver and the counter disagree, firing `COVERAGE_UNBALANCED` about a count
  mismatch rather than the fixture nobody excluded — a failure pointing away
  from its cause, inside the module written to stop exactly that. Both now read
  `config_facts::TEST_ONLY_KEYS`, so disagreement is unrepresentable rather than
  merely detectable.

  **Which keys are `parse_env` calls is derived, not listed again.** A second
  list would have to agree with the first, which is the defect one level up.
  `config.rs` keeps its literals: that is where the fixtures are declared and
  used, not a rule restating them.

  `assert_test_keys_are_live` fails if an entry outlives its fixture — without
  it, a real variable could later take an excused name and be dropped from every
  document silently.

  Verified by mutation: a stale entry fails naming it; adding a second
  `parse_env` fixture to the one list makes the resolver, the counter and the
  presence scan all follow, where it previously took three edits; and adding one
  without listing it anywhere still fails loudly, naming the key.

* **One `Config` fixture for the examples that build one (047)** — seven
  examples hand-rolled the same 20-field literal, six `acceptance_*` and
  `spike_embed_latency`. **Fifteen fields were identical in all seven**; the
  five that differed each did so for a reason: a live key rather than a
  placeholder, a longer timeout for the examples that reach the network,
  the one confinement root, the two that need embeddings. Those five are now
  the only thing each example writes down, via `..common::config()`.

  **Bendability was the objection, and struct update syntax answers it.** Every
  field stays overridable; what changes is that bending one no longer means
  restating nineteen the example did not mean to choose. The placeholder key
  had been written three ways across the seven (`test-key`,
  `dummy-acceptance`, `unused`) for a field none of them reads — variance with
  no intent behind it.

  `examples/common/mod.rs` is not an example target: Cargo discovers
  `examples/*.rs` and `examples/*/main.rs`, and that directory has neither.

  The model and log level come from `DEFAULT_MODEL`, `DEFAULT_VOYAGE_MODEL` and
  `DEFAULT_LOG_LEVEL` rather than repeating their values. `spike_client` and
  `spike_thinking` were restating `config.rs`'s model default by hand in their
  own `unwrap_or_else`, which is the drift class exactly; they now read the
  constant. `spike_otlp` keeps its literal — it is an arbitrary span-attribute
  value in a telemetry spike, not a restated default, and deriving it would
  claim a relationship that does not exist.

  Occurrences of the model literal across the crate: **97 → 78**.

  **These examples are not run by `cargo test`.** They are acceptance scripts
  driven by hand against a mock or the live API, so `cargo build --examples`
  and the lint gate — which reaches them only since 044 — are the whole of
  their automated coverage. That is the limit of what this change was verified
  against.

* **CLAUDE.md's active-feature block is no longer unchecked (046)** — it named
  040 as the last Spec Kit run while 041–045 had shipped since. The same shape
  as `--help` before 034 and the README before 039: a hand-written document
  that the conversation moved past.

  **Only the derivable parts are bound, and that limit is stated rather than
  papered over.** "Which feature is active" is intent — no source of truth
  exists for it, so no check can catch the block naming an old feature while
  newer ones ship. What is derivable is now checked: every `specs/<dir>` the
  block links must exist, and any "all N tasks closed" claim must match that
  feature's `tasks.md`, open tasks included.

  Claiming more would be worse than the gap. A check that looks like it covers
  staleness while covering only link validity is exactly what 036 and 039
  shipped — partial derivation reported as derivation, which is the defect 040
  was written to end.

  Mutation-verified three ways: a link to a nonexistent spec directory, a wrong
  task count, and a reopened task each fail, naming what disagrees.

* **Block comments no longer mint configuration variables (045)** — the
  comment stripper handled `//` only, so a block comment quoting a removed call
  (`/* was parse_env("RETRY_BACKOFF_MS", 250) until 041 */`) was read as a live
  one.

  **The symptom was not the predictable one.** Extraction and the call-site
  counter strip identically, so both *agreed* on the phantom and the coverage
  equation stayed balanced. What failed was `DEFAULT_UNDOCUMENTED`, accusing
  `--help` and the README of omitting a variable nobody declared — and the
  cheapest way to green is to add a row for it to both. That is the corruption
  cycle 040 removed from the resolver, reached from the opposite direction.

  Both comment forms are now handled in **one pass**, never in sequence.
  Sequential stripping fails whichever order you pick: lines first, and a `//`
  inside a block comment can take that block's `*/` with it, leaving it
  unterminated so the rest of the file vanishes; blocks first, and a `/*`
  inside a line comment opens a block that was never there. Nested block
  comments and delimiters inside string literals are both handled — an
  unterminated `/*` in a string, mis-read, would silently drop every variable
  declared after it.

  **Fixing it surfaced a third scan.** `help_lists_every_variable_the_config_reads`
  (from 034) reads `config.rs` independently and stripped nothing at all, so a
  variable *name* mentioned in any comment became one `--help` was required to
  document. It now shares the stripper. 040 unified two scans; this was the one
  it did not reach, and it still carries a hand-written fixture-exclusion list.

  Verified by mutation in both directions: four comment shapes plus a string
  containing an unterminated delimiter all pass, and disabling the block-comment
  branch fails the new test.

* **The lint gate now covers tests and examples (044)** — CI ran
  `cargo clippy --all-features -- -D warnings`, which lints the library and
  binary production paths and nothing else. It reported zero while 39 errors
  stood in `#[cfg(test)]` modules and one example. All are fixed and the gate
  is widened to `--all-targets` in every place it is declared: `ci.yml`, the
  pre-commit hook, the `cargo lint` alias, and CLAUDE.md.

  **A duplicated `#[test]` attribute was inflating the suite count.**
  `routing.rs` carried `#[test]` twice on one function, so the harness
  registered and ran it twice. The reported figure has been one higher than the
  number of distinct tests; 593 was really **592**. Nothing was lost in this
  change — the count fell because the double-count stopped.

  **Three more pieces of 040's file split surfaced**, none visible to any check
  the project had. `#[cfg(test)]` and the test module's `#[allow]` attributes
  were carried into `source.rs`, where they silently attached to
  `classify_call_sites` while `mod.rs`'s own `mod tests` lost both; and two doc
  blocks were left duplicated or stranded above the wrong item. That split has
  now produced five separate attribute or doc misattachments, every one of
  which compiled, passed the suite, and left the declared gate green. Moving
  line ranges between files orphans whatever sits at a boundary, and only
  reading each item against its own docs and attributes finds it.

  Five files declare the gate command by hand and nothing binds them to each
  other. They were updated together here; the duplication remains.

* **`--help`'s routing vocabularies are derived from the enums (043)** — the
  body lists twelve call sites, two tiers and five effort levels, and the test
  pinned **three** of the twelve by hand. A thirteenth call site would be
  settable through `PARALLAX_MODEL_*` and `PARALLAX_EFFORT_*` while absent from
  `--help`, with nothing failing. That is 034's defect, still live in the one
  block 034 did not reach.

  All three vocabularies now derive from `CallSite::ALL`, `Tier::ALL` and
  `Effort::ALL`. No new constant list was introduced: each type already
  exposes `suffix()` or `as_str()`, and a fresh table of names would have been
  a fourth hand-written list reproducing the defect.

  **The check matches whole tokens, never `contains`.** `VERIFY` is a substring
  of both `GROUNDED_VERIFY` and `RESEARCH_VERIFY`, and `high` of `xhigh` — so
  the previous substring check passed on a help body listing only the longer
  name. Verified: removing the standalone `high` from the `LEVELS:` line left
  the old check green and fails the new one. Same collision class as the
  six-line window 040 removed.

* **`Config::from_env` documents what adding a variable commits you to** — that
  every default must be stated in both `--help` and the README, and that a
  named constant used as a default has to live in a file
  `config_facts::SOURCES` reads. Both were enforced already and both were
  learned by hitting the failure; the note moves that to before the build.

  The list itself is **not** copied into the doc. Restating a derivable fact is
  the defect this whole run has been removing, and the failure already prints
  the current contents. Widening `SOURCES` to every file was considered and
  rejected: `lookup_constant` returns the first match for a bare unqualified
  name, so an enumerated-everything list restores the whichever-came-first
  ambiguity FR-004a exists to make unrepresentable. Path-qualifying the
  constant is the route that scales, and it selects the file to read.

## [0.5.0] - 2026-07-27

### Fixed

* **One `Config` fixture for the library's unit tests (042)** — four modules
  hand-rolled the same 21-field literal, and it had already drifted:
  `server.rs` used `max_retries: 1` where the three client modules used `2`,
  for no recorded reason and with no test depending on it. Each copy was one
  more place a newly added field silently gets a value nobody chose. A shared
  `config::test_config()` replaces them; the three that need a capability gate
  override that one field with struct update syntax, so the difference is the
  only thing written down.

  `tests/integration.rs` keeps its own copy and that is deliberate: an
  integration test is a separate crate linking the library compiled *without*
  `cfg(test)`, so a `#[cfg(test)]` item does not exist for it — the same
  linkage that forced `config_facts` into the binary crate. Making it reachable
  would mean shipping test scaffolding in the public API, a worse trade than
  one duplicated fixture. The `examples/` fixtures are untouched.

  **Two doc blocks were attached to the wrong functions**, introduced by 040's
  module split and found while auditing this: `stated_in_help`'s documentation
  sat above `is_absence_sentinel`, and a stranded copy of it above
  `variables_without_defaults`. Misattached documentation compiles, passes
  every test, and leaves the gate green — so it was found by walking each item
  and asking whether its doc block describes it, not by any check.

* **A default model with no price row now fails the build (041)** —
  `config.rs`'s `DEFAULT_MODEL` and `telemetry.rs`'s `PRICING_PER_MTOK` both
  name model ids and nothing linked them. Renaming a default to an id absent
  from the table costed every run at the conservative fallback with
  `pricing_known = false`: an over-estimate reported as though it were a price,
  with no test failing.

  The check reads both constants rather than restating either — a test spelling
  out `"claude-opus-4-8"` would pass while the code moved underneath it, which
  is the defect and not the check. `DEFAULT_VOYAGE_MODEL` is covered too:
  naming only the constant that prompted this would be the hand-written list of
  one that 040 found *inside* the feature written to abolish hand-written
  lists.

  Verified by mutation — renaming `DEFAULT_MODEL` to an unpriced id fails
  naming the constant, the id, and both remedies.

* **Every configuration default is now resolved or the build fails (040)** —
  three features built checks binding an operator-facing document to
  `config.rs`, and each closed part of the loop while reporting it closed. 034
  pinned five defaults by hand; 036 replaced them with a scan that read defaults
  out of source, but only those written as a bare numeric literal; 039 copied
  that scan for the README and inherited its blind spot. When the scan's digit
  filter came up empty it moved on **without recording that it had skipped one**.

  Demonstrated before the fix: setting both documents to a wrong
  `GROUNDED_VERIFY_MAX_BYTES` of `999999` left every test green. It now fails in
  both documents at once, naming the variable and quoting each.

  **The silent skip was the defect, not the missing coverage.** A default that
  cannot be read is `Unresolvable`, which fails naming the variable, the
  expression found, the shapes handled, and both remedies — there is no state
  meaning *skipped*. A default naming a constant that is not in any file the
  check reads is a separate state with the opposite remedy, because "teach the
  resolver this shape" is wrong advice when the shape was read fine and the
  missing thing is an entry in `SOURCES`. Coverage is now an equation replacing
  a `checked >= 8` floor that the literal-valued variables cleared on their own
  while three of four shapes went unexamined.

  One resolver in `src/config_facts.rs` replaces two near-duplicate scans that
  had already drifted — one guarded a quote the other did not, one excluded the
  test fixture, only one had a coverage floor. That duplication is how 039
  inherited 036's blind spot, so both copies are gone rather than both patched.
  Constants resolve from an **enumerated file set**, never a crate search: Rust
  permits one name in several modules, and an unrestricted search would compare
  a document against whichever declaration it reached first.

  **Three checks that passed for reasons unrelated to what they verified.** The
  document comparison joined a six-line window and asked whether the value
  appeared anywhere in it, so a wrong single-digit default matched a
  neighbouring row and passed — `999999` had failed only because six digits
  rarely collide. It now compares against each document's structured default
  marker exactly, which immediately surfaced that `--help` stated no default for
  `RESEARCH_CONCURRENCY`. The coverage equation compared the fact vector against
  itself, so a dropped variable shrank both sides and stayed balanced; it now
  compares against a count taken from the call markers alone, sharing no code
  with extraction. And the first extractor reported *confidently wrong values* —
  `ANTHROPIC_API_KEY`, which has no default, resolved to the API base URL —
  because its window ran past the end of its own statement.

  All eight documented failure messages were mutated into firing one at a time
  and the message each produced recorded, in
  `specs/040-unresolvable-default-fails/contracts/failure-modes.md`. Two of the
  three defects above were found that way; a failure surface nobody has seen
  fire is a claim, not a check.

  The pre-merge review then found four more shapes returning wrong values, all
  one root cause: **resolution succeeded on a prefix of what it read instead of
  requiring it consumed the whole expression.** `RESEARCH_CONCURRENCY_MAX / 4`
  resolved to `32` for a default of 8; `3u32` to `332`; a doc comment quoting a
  superseded `const` beat the real declaration; a path-qualified constant
  resolved to whichever file declared the name first. Each balanced every
  invariant and then failed naming the *documents* as wrong, whose cheapest
  green is to copy the fabricated value into both — a corruption cycle 036's
  skip never had. Literals are now parsed and re-formatted rather than built by
  deleting characters, a path qualifier selects which source is searched,
  comments are stripped before scanning, and an expression with anything left
  over is `Unresolvable`.

  Two duplications were removed rather than documented: `main.rs` carried a
  second `LOG_LEVEL` default duplicating `config.rs` (both now read one named
  `DEFAULT_LOG_LEVEL`), and the no-default variable list was five hand-written
  names inside the feature that exists to abolish hand-written lists. The module
  was split at the seam the review named — `config_facts/{mod,source,documents}.rs`
  at 247/476/185 lines — after the single file reached 853.

* **The README's configuration table is checked against `config.rs` (039)** —
  22 rows restating a variable name and its default, hand-written with nothing
  binding them to the code. 034 and 036 fixed exactly this for `--help`, first
  for presence and then for values, and left the identical table one file over
  unguarded. That is the §10 rule broken in the file next to where it was
  applied.

  The test derives the pairs from `config.rs` the same way, so both drift modes
  fail. Mutation-verified: changing a README default gives
  `RESEARCH_CONCURRENCY applies 8; README row: … 16`, and deleting a row gives
  `README config table omits: ["MAX_RETRIES"]`.

  The table stays hand-written rather than generated, because its *Purpose*
  column is reasons and reasons have no source to derive from. Derive the
  facts, hand-write the reasons — this checks the facts and leaves the prose
  alone.

  **The first version of this test reported every variable missing, and the
  test was wrong, not the README.** `include_str!` reads the file as it sits on
  disk, which is CRLF here, so a blank-line boundary search on `

` matched
  nothing and the extracted table came out empty. It now normalises line
  endings and asserts the extraction found more than fifteen rows, so a
  boundary search that breaks again says so instead of blaming the document.

### Added

* **Contract files and tests for the four tools that shipped without them
  (038)** — `decide`, `diverge`, `elicit` and `grounded_verify` had no
  `.tool.json`, so four of fifteen tools had nothing for the constraint
  comparison 029 added to run against. Contract coverage goes 12 files / 5
  testing modules to 16 / 9.

  **These tools were not unguarded in the usual sense** — their schemas are
  derived from Rust types and validated at registration. What was missing is a
  checked-in statement of intent to diff against, so a change to the input or
  output surface passed silently. 029 exists because two defects walked through
  a contract test that compared names only; these four could not run that check
  at all.

  The files are **baselines captured from today's schemas**, not specifications
  authored ahead of the code, and say so in their `$comment`. Nobody reviewed
  whether the current surface is the intended one; what starts now is that
  changing it fails until the contract is updated deliberately.
  Mutation-verified — adding an input property to `DecideParams` fails with
  `decide input properties drifted from the contract`, listing both sets.

## [0.4.0] - 2026-07-26

An audit release. Nothing here was a feature request — every entry came from
asking what the code says about itself and finding it disagreed.

**The one behaviour change is 033**, sizing the research tier budgets from
measured history: `standard` 450 000 → 1 600 000 and `deep` 1 000 000 →
5 500 000. That raises what a run with no explicit `budget_tokens` may spend,
which is why this is a minor bump rather than a patch. Everything else is
documentation the binary now enforces, a refactor with no behaviour change, and
three principles recorded in the design corpus.

**What the audits kept finding was documents restating facts they could have
derived** — `--help` claiming a 30 000 ms timeout the code has not used since
018, a tool list of fourteen where the server exposes fifteen, five defaults
pinned by hand while fifteen more drifted freely. 037 records the rule that
falls out of it, and this release is the first where the checks enforcing it
exist.

### Fixed

* **`--help` defaults are derived from `config.rs`, not pinned by hand (036)**
  — 034 tied the help text to the config for *presence* and called the loop
  closed. It was half closed. Changing `MEMORY_RECALL_LIMIT` from 5 to 7 and
  `RESEARCH_CONCURRENCY` from 8 to 12 in `config.rs` left every help test
  green, because neither number was on 034's hand-written list of five.

  That is the same class as the defect 034 existed to fix — the help saying
  30000 while the code read 120000 — so the instance was fixed and half the
  class left open. The test now reads `parse_env("NAME", DEFAULT)` pairs out of
  the source and checks each against the help entry for that name, so a default
  that moves without its documentation fails, including for variables nobody
  thought to pin. Mutation-verified with the exact change that was previously
  silent: `MEMORY_RECALL_LIMIT applies 7, help says: … (default: 5)`.

  Writing it derived immediately found an edge a hand-written list would not
  have: `VERIFY_MAX_CLAIM_CHARS` is documented as a continuation line under
  `INPUT_MAX_CHARS` and shares its default, so the entry window has to reach
  backwards as well as forwards.

* **`--help` matches the runtime, and a test now keeps it that way (034)** —
  the help body advertised `REQUEST_TIMEOUT_MS` at 30000 while the code has
  read 120000 since 018, listed 7 of 20 environment variables, omitted both
  routing namespaces entirely, and named `VERIFY_MAX_CLAIM_CHARS` as though it
  were canonical rather than the deprecated 002-era alias `config.rs` honours
  only when `INPUT_MAX_CHARS` is unset.

  It now covers every variable, grouped by whether it is always read, gates a
  capability, or belongs to a subsystem, plus the `PARALLAX_MODEL_*` /
  `PARALLAX_EFFORT_*` namespaces with their call sites, tiers and levels.

  **The reason it drifted is the part worth fixing.** 027 corrected that same
  timeout in `README.md` and `CLAUDE.md` and never touched this block; a later
  loose-ends sweep re-checked those same two files. Both passes looked for
  documentation where documentation *looks* like it lives, and `--help` is the
  only pre-runtime contract a caller sees — an operator who runs `--help` and
  then starts the server was getting two different mental models.

  So the help body is now a testable `help_text()` string, and three tests
  compare it against `config.rs` itself: every variable the config reads must
  appear, both routing namespaces and their vocabulary must appear, and stated
  defaults must be the ones the code applies. Mutation-verified — restoring the
  stale timeout and dropping one variable fails with `timeout default drifted`
  and `--help omits variables that config.rs reads: ["RESEARCH_CONCURRENCY"]`.

### Changed

* **Three cross-cutting principles recorded in `NEW_SERVER_DESIGN.md` §10
  (037)** — the 027–036 arc produced rules for placing future work, not just
  fixes to past work, and §10 is where they belong beside the operator-owned vs
  caller-owned test.

  **The verification ladder.** Five rungs, and the point is not that a test
  gets stronger — it gets *closer to what it checks*, and each rung fails
  differently. A test listing what to check inherits whoever wrote the list;
  deriving expectations from the same source production reads collapses the
  test's blind spot into the editor's, so drift between the two stops existing.
  Recorded with its cost, which is what says when not to climb: a derived check
  can no longer tell you a value is wrong, only that the two agree. Both
  directions were run in this arc and both were right — `--help` defaults are
  derived because the failure was drift, while the research tier budgets are
  deliberately *not* derived from observed runs, because a truncated run
  encodes the ceiling that truncated it. That is the circularity that produced
  the quick tier's 250 000.

  The top rung — making the class unrepresentable — is what
  `deny(clippy::unwrap_used)` does at the type level, and Principle III already
  says so. What the audits found is that it was being applied only where a lint
  happened to exist.

  **Derive the facts, hand-write the reasons.** The caller-visible surface is
  canonical for facts; `README.md` and `CLAUDE.md` are downstream of it, and a
  sweep that searches the documents is one refactor from being wrong. It
  inverts for reasons, which have no source to derive from. Nearly every
  documentation defect in this arc was a document restating a fact it could
  have derived.

  **Ask what fails silently, not what is currently wrong.** Two silences worth
  telling apart: `CallSite::index` would have failed while the system kept
  working — a valid client, a real model, a plausible bill — and `--help`
  failed because nobody re-reads it.

  Docs only.

* **`CallSite::index` is derived from the discriminant instead of hand-written
  (035)** — it was a twelve-arm `match` mapping each variant to a literal,
  which meant two orderings (`index` and `ALL`) that had to agree. `ClientPool`
  keys its per-site array on the result, so disagreement would not fail loudly:
  one call site would receive another's client, and the invocation record would
  attribute cost to a model that never ran.

  **This is not a bug fix, and the distinction matters.** Both drift directions
  were already caught — reordering `ALL` failed `index_matches_all_order`
  naming the site, and adding a variant was a compile error at two
  exhaustiveness sites. Verified by mutation before changing anything. What the
  change buys is removing the class rather than guarding it: a fieldless enum
  casts to its declaration order, so there is no second ordering left that
  *can* disagree.

  A linear `ALL.iter().position(..)` was rejected — it returns an `Option`
  whose `None` arm is unreachable, and Principle III's ban on `unwrap` in
  production would make that arm either a silent fallback or a panic. The cast
  has neither problem. `strum::EnumCount` was rejected as a dependency for
  something a cast already gives.

  The guarding test is kept, narrowed to the one invariant that survives —
  `ALL` written in declaration order — and extended to catch a duplicate entry,
  which would give two sites the same index and leave a slot unreachable.

* **Research tier budgets sized from measured history (033)** — `standard`
  450_000 → 1_600_000 and `deep` 1_000_000 → 5_500_000, each the tier's own
  declared caps multiplied by one measured figure (2 508 tokens per claim per
  pass).

  **The base rate decided this, not the derivation.** A `decide` pass
  recommended leaving the budgets alone and making truncation louder instead;
  its confirmation `verify` refuted that 3/3, on the ground that three observed
  trips against an *unknown* total supports no conclusion either way. That
  total turned out to be queryable — 019 put the tier on the invocation record
  — and the history says: **standard tripped 3 of 3**, spending 503k, 643k and
  949k against a 450_000 cap, while **quick trips 0 of 7** with a median of
  239k. The old standard default was not an occasional edge case; no standard
  run has ever finished inside it.

  Deep is the weaker half and is labelled as such in the code: **no deep run
  has ever executed.** It was nearly held back for that reason, until the tier
  ordering forced it — deep declares strictly more than standard in every
  dimension, so a smaller budget would make the more thorough tier truncate
  harder. `tier_table_matches_the_design` caught that when it was first written
  the other way.

  Also records that **the ceiling is soft**: the budget is probed before and
  inside each unit of work, so it stops new tasks while in-flight ones finish.
  Those standard runs recorded 1.1× to 2.1× over their nominal cap. A budget is
  the point a run starts winding down, not the most it can cost — which is both
  why the old 450_000 was already yielding 949k runs and why a larger nominal
  value is the honest way to say what was happening anyway.

* **The five single-shape corrective handlers moved to
  `src/server/correctives.rs` (032)** — `verify`, `unstick`, `decide`, `elicit`
  and `diverge` are structurally identical and sat in `server.rs` with
  everything else. Both reviewers of 028 named this the split seam; it was
  deferred from that feature so its diff would show only the move, which is
  what it does — the moved bodies are content-identical apart from the
  visibility keyword, and `server.rs` shows deletions plus one `mod` line.

  `check` and `grounded_verify` stay put deliberately. Each holds its client in
  a startup-built deps struct and needs a different entry point, so moving them
  would fold a reshape into a relocation.

  Effect is smaller than the file size suggests, and worth stating plainly:
  `server.rs` goes 2108 → 1956 lines, but 908 of those are its test module.
  Production drops 1201 → 1048. The remainder is dominated by fifteen
  `#[tool(...)]` declarations whose description strings rmcp requires in a
  single impl block — not further splittable without fighting the macro.

  No behaviour change.

* **Every configuration variable is now classified against the operator-owned
  vs caller-owned test (031)** — `NEW_SERVER_DESIGN.md` §10 previously stated
  outright that the remaining variables had never been checked against the rule
  it records. They have been, and the section now carries the result rather
  than the disclaimer.

  All 21 settings land: credentials, deployment identity, and the two security
  boundaries (`GROUNDED_VERIFY_ROOT`, `FETCH_ALLOW_PRIVATE`) are operator-owned
  by construction — a caller-settable confinement root is the hole it exists to
  close. `CHECKPOINT_GATE_PATTERNS` is operator-owned on the layer argument:
  the watchdog fires what the model cannot self-diagnose, so letting the caller
  tune its triggers inverts the layer. The three already-caller-facing settings
  are unchanged.

  **Two genuine candidates were found and deliberately not built** —
  `REQUEST_TIMEOUT_MS` and `GROUNDED_VERIFY_MAX_BYTES`, both recorded with the
  reason. The rule says which way a setting leans; leaning is not the same as
  earning a tool argument, and writing that distinction down is the point of
  auditing rather than expanding.

  No code changes.

## [0.3.0] - 2026-07-26

Makes the server's cost-and-rigor controls reachable by the thing that actually
makes the calls. Every addition is backward-compatible: each new tool argument
is optional, both new record columns are nullable and additive, and a caller
that supplies none of them produces requests byte-identical to 0.2.0.

**Two features documented in the 0.2.0 notes are first tagged here.** The
`[0.2.0]` block below describes per-call-site reasoning effort (022) and the
silent-phase-failure fix (023), but both merged after the `v0.2.0` tag was
placed — `git show v0.2.0:src/routing.rs` contains no `EFFORT_PREFIX`. A reader
comparing that tag against its own notes would find features the code does not
have. Released history is not rewritten and the tag is not moved, on the same
reasoning 027 used when correcting a false claim in those notes: the record of
what was published stays as published, and the correction lives in the block
that follows. So 022 and 023 ship for the first time in this tag.

**Everything in this release was verified against live infrastructure**, not
only against tests. That mattered: verifying 027 is what surfaced 030.

### Added

* **Per-call reasoning effort, pass count, and research concurrency (028)** —
  the calling model can now set these for a single invocation, with no file
  edited and no session restarted. `effort` on the seven correctives (`verify`,
  `unstick`, `diverge`, `decide`, `elicit`, `grounded_verify`, `check`),
  `passes` on the three ensemble tools, and `constraints.concurrency` on
  `research`. Every argument is optional; omit them all and outbound requests
  are byte-identical to before.

  **Why this was wrong before.** 022 put effort in `PARALLAX_EFFORT_*` by
  mirroring 018's `PARALLAX_MODEL_*` shape — a copy, not a decision. The two
  are not the same kind of knob: which model runs a call site sets the rate the
  operator is billed at, while how much reasoning one task deserves is a
  per-task judgment. The consumer of this server is a model, so a control
  reachable only by editing JSON and restarting is unreachable in practice.
  `research`'s `depth` and `recall`'s `limit` were already caller-facing in the
  same codebase; the test that distinguishes them is now recorded in
  `NEW_SERVER_DESIGN.md` §10. `MEMORY_RECALL_LIMIT` needed no work — `recall`
  already took a per-call `limit`, and it is now cited as the prior art with a
  confirming test.

  **Lowering only, for anything that multiplies model calls.** `passes` and
  `concurrency` may be reduced by a caller, never raised: each raise buys work
  the operator did not authorise. They differ in how the ceiling is enforced,
  deliberately — an over-large `passes` is **rejected**, because a silently
  reduced count would make the returned confidence rest on a narrower basis
  than the caller believes, and confidence is what the tool is read for. An
  over-large `concurrency` is **clamped**, because it is advice about running
  work already authorised and failing a valid run over a performance hint costs
  more than it protects. Effort is exempt from the rule: it changes how one
  call's budget is spent, and `MAX_TOKENS` already caps that.

  **Spend stays explainable.** `invocation_records` gains nullable `effort` and
  `passes` columns holding the caller's **overrides only** — never the configured level,
  which is constant, already printed in the startup routing table, and not a
  single value at all for an invocation spanning several call sites. Additive
  migration on the `depth` pattern, and both are exported as
  `parallax.effort`/`parallax.passes` spans attributes so the SQLite record and
  the OTLP surface cannot disagree — the property the 007 contract exists to
  guarantee.

  `ANTHROPIC_API_BASE` is now configurable. It exists for the test suite: the
  per-effort clients were previously built against a hard-coded production
  endpoint, so a call carrying an effort left the `ModelClient` seam entirely
  and a `cargo test` run opened real connections to the Anthropic API. Pointing
  the whole pool at one base makes the suite hermetic, which Principle IV
  requires and which is what let the wire-level test for this feature exist at
  all.

  Internally, the client pool now pre-builds every `(routed model, effort
  level)` pair over one shared HTTP transport. The set is bounded at six effort
  states by the routed model count, so a per-call effort is a lookup rather
  than a construction — and the `ModelClient` seam is untouched, upholding 018
  research D2 rather than reversing it.

### Changed

* **An effort rejection names which setting caused it (027)** — when the
  provider rejects a request for the reasoning-effort parameter, the error now
  appends the model it was sent to, the level that was sent, and both remedies
  (unset the `PARALLAX_EFFORT_*` variable covering the call site, or route the
  site to a model that accepts effort). The provider's own message is kept
  beside it, never replaced.

  The guard is narrow on purpose — a client-error status, an effort actually
  configured, and a body naming the parameter. A rejection with any other cause
  reads exactly as it did before, because a confident wrong diagnosis in front
  of an operator is worse than a bare message. If the provider rewords its
  rejection the hint stops appearing and the message degrades to today's
  behaviour, which is the safe direction.

  No model-capability table and no startup refusal: the fact comes from the
  provider at the moment it is true, so a family released after this binary was
  built behaves identically to one known at build time, and no configuration
  that would have worked is prevented from starting.

### Fixed

* **The effort rejection names the per-call source too (030)** — 027's message
  offered two remedies: unset the `PARALLAX_EFFORT_*` variable, or route the
  call site to a model that accepts effort. 028 then added a third way to set
  an effort — the per-call argument — and did not update the message.

  Found by verifying 027 live, which is the case that had never been exercised:
  a caller-supplied `effort` with **no variable set anywhere**, told to unset a
  variable that did not exist, while the remedy that actually applied went
  unmentioned. The message now names all three, with the per-call argument
  first because dropping it is the only one needing no restart.

  All three are listed rather than the applicable one selected, because the
  client genuinely cannot tell them apart: the pool serves the *same* client
  for a configured `low` and a caller-supplied `low`, and that sharing is the
  point of keying on `(model, effort)`.

* **The published schema no longer contradicts the server (029)** — `passes`
  derives from a Rust `u8`, so the schema shown to the calling model advertised
  `minimum: 0, maximum: 255` while the server rejects `0` and rejects anything
  above the configured count. A model reading the schema was told values were
  valid that the server refuses. The derived schema now states `minimum: 1`,
  and the contract records that the effective maximum is the deployment's
  configured count rather than a fixed number, so it cannot appear in a schema
  at all — a request above it is rejected with an error naming the ceiling.

  **The larger half is the test gap that let it ship.** The contract tests
  compared property *names* and the `required` list, never constraints. Two
  defects passed through: this one, and `effort` shipping as an untyped string
  in 028 while its contract claimed an enum. All five contract tests now compare
  declared constraints, following `$ref` into `$defs` so a referenced enum
  neither hides a mismatch nor manufactures one. Mutation-verified: removing the
  bound fails the test naming the property and both values.

* **A false claim in the 0.2.0 notes, corrected here rather than rewritten
  (027)** — the 022 entry below states that `output_config.effort` is "the
  control that every routed family accepts". **That is false.**
  `claude-haiku-4-5` rejects it with a 400, and that is the model
  `018/quickstart.md` recommends for the bulk tier — so the project's two
  standing recommendations were jointly a failing run, observed in production on
  2026-07-25. Released history is not edited; the sentence stands below as
  shipped, and this is its correction. The same false claim in
  `docs/design/SDK_LANDSCAPE.md` and `specs/018-model-routing/research.md` — not
  released history — has been fixed in place. 022's own
  `spec.md:186` said support varies by family and was right; the other two
  artifacts were the error.

* **`PARALLAX_MODEL_*` and `PARALLAX_EFFORT_*` are documented (027)** — features
  018 and 022 shipped both namespaces without adding them to `README.md` or
  `CLAUDE.md`, whose configuration sections enumerate every other environment
  variable. Also corrects `REQUEST_TIMEOUT_MS`'s documented default, which read
  `30000` in three places after 018 raised the code to `120000`.

## [0.2.0] - 2026-07-25

The project's first tagged release. Everything before it shipped only as merged
commits on `main`.

**Two internal constants in this release are underived rather than measured**,
and are recorded here so a later change to them reads as routine rather than as
a correction. The per-call output ceiling (32 000 tokens) was raised after a
real truncation whose size was unknown, because the failure recorded zero
tokens — the defect that the metered-failures work in this same release fixed.
No truncation has occurred since, so it remains unmeasured. The default token
budgets for the standard and deep research tiers have never been measured
either; the invocation record only began storing a run's tier in this release,
and those tiers accrue data only when deliberately exercised. Both are internal
defaults, overridable per call, and not part of any published interface.
Retuning either is an ordinary change for a subsequent release.

Rolls up the post-#38-merged work on `main` (#38–#42) alongside features
018–021. The agent doesn't carry `ANTHROPIC_API_KEY`, so live-dogfood freshness
is not re-fired in this arc; the #42 stamps therefore read "Mechanism
re-verified" rather than "Re-verified" (see the *Docs* entry below for the
rationale).

### Added

* **Per-call-site reasoning effort (022)** — each of the twelve model call sites
  can request a reasoning-effort level from the provider, over a
  `PARALLAX_EFFORT_*` namespace mirroring `PARALLAX_MODEL_*`: per-site and
  per-tier, most-specific-first, and resolved **independently of the model**, so
  the bulk tier can carry a cheap effort without also naming a model. Levels are
  `low`, `medium`, `high`, `max`, `xhigh`; an unrecognised suffix or an
  unparseable level is a startup error naming the variable, on the same reasoning
  018 used — a setting that silently does nothing leaves the operator believing a
  call site changed when it did not.

  **Off by default, and provably so.** Absent is a distinct state from `high`,
  not a synonym: an unset call site sends no `effort` field at all, so the
  request body is byte-identical to before. A wire test fails if the key appears.

  The client pool now keys on `(model, effort)` rather than model alone — two
  call sites on one model at different efforts need separate clients, or the
  first site's effort would ride on the second's calls.

  This discharges the deferral named in 018 research D7, under a different
  mechanism than the one deferred: `thinking: {"type": "disabled"}` is still
  rejected by Fable 5, and the control that every routed family accepts is
  `output_config.effort`. Which level suits which call site is deliberately not
  shipped — that wants measurement, and a recommended default on no evidence
  would be the guessing this project keeps having to undo.

* **Rigor tier on the invocation record (019)** — `invocation_records` gains a
  nullable `depth` column, stamped `quick`/`standard`/`deep` for `research` and
  left NULL for every other tool. Additive, pragma-guarded `ALTER TABLE`, the
  same shape as the 017 and 018 migrations; rows written before it read back as
  `None` rather than a guessed default. The gap this closes is concrete: 59
  recorded research runs exist and not one of them can be attributed to the
  ceiling it ran under, which is why the standard and deep budgets are *not*
  being re-tuned in this change. The tier is captured at request time, so a run
  that fails or is cancelled still records which ceiling it was held to — the
  case most worth knowing about when sizing a budget. Mirrored to OTLP as
  `parallax.depth`, emitted only when the tool has a tier so tiered runs stay
  separable (`specs/007-observability-layer/contracts/telemetry.md`).

* **Per-call-site model routing (018)** — each of the server's twelve model
  call sites can run on a model chosen for the work it does, instead of all
  twelve sharing one `ANTHROPIC_MODEL`. Two work-kind tiers (`bulk`, holding
  only research extraction — the one call site whose volume scales with the
  number of fetched sources — and `judgment`, holding the other eleven) over a
  reserved `PARALLAX_MODEL_*` namespace, with per-call-site overrides resolved
  most-specific-first. The namespace is validated as a whole: an unrecognised
  suffix is a startup error naming the variable, which is what makes a
  misspelled route visible rather than a bill that silently never drops. A
  resolved routing table is logged to stderr before serving. `ModelClient`
  keeps its exact signature — the model stays a property of the client
  instance, so routing is construction-time and the process holds one client
  per distinct resolved model. **Off by default**: unset means the pre-018
  behavior, byte-identical costs included.

  Cost accounting became per-model as a consequence: once one invocation can
  span models, a single rate applied to summed tokens is wrong under either.
  An invocation still produces exactly one audit record and one exported span;
  the record gains `models` and `usage_by_model` (two nullable columns via a
  pragma-guarded `ALTER TABLE`; pre-existing rows read back as the
  single-model records they were, no backfill), and `cost_usd` becomes the sum
  over participants at each one's own rate. The attributed model — the record's
  `model` column and the span's `gen_ai.request.model` — is the participant with
  the most **measured tokens**, deliberately not the most cost, because an
  unpriced model falls back to Opus-tier rates and would otherwise win
  attribution by merely lacking a price. `parallax.cost` and
  `gen_ai.client.token.usage` are now recorded once per participating model;
  the invocation counters are not split, since that would stop them counting
  invocations. The 007 telemetry contract is amended in-change.

  Pricing gains the Claude 5 rows. This was not theoretical: `claude-fable-5`
  bills at $10/$50 against an Opus-tier fallback of $5/$25, so a deployment
  routed there was under-reporting spend by half. A new `pricing_known` flag
  distinguishes a looked-up price from the fallback — necessary precisely
  because `claude-opus-5` matches the fallback exactly, making "correct by
  lookup" and "correct by coincidence" otherwise indistinguishable.

  The per-call output budget rises from 4 096 to 16 000 tokens, derived from
  the largest mode schema's own bounds (~3 500 tokens of answer) at ≥4×, with
  the request timeout raised alongside it: on Claude 5 families, omitting
  `thinking` runs adaptive reasoning charged against the same ceiling, so the
  old budget could truncate a verdict before its JSON was emitted. The request
  shape stays family-agnostic — no `thinking` field, which is the one form
  every family accepts. **Named deferral**: per-family thinking suppression,
  to be decided on measured cost (research D7). No tool's input or output
  schema changes, and no caller-supplied value influences which model answers.
  `SDK_LANDSCAPE.md` amended in-change; artifacts under
  `specs/018-model-routing/`.
* **Memory consolidation and auto-capture (017)** — the write-path half of
  the memory layer. Supersession and merge run on admission: a
  deterministic cosine screen (0.75) gates one budgeted decline-biased
  judgment; updates supersede (status change with attribution, never
  deletion), near-duplicates merge at ≥ 0.90 to a byte-identical survivor
  under a trust guard that doubles as the promotion path
  (re-admission). Decay is ranking-only via a reinforcement-refreshed
  recency clock. The end-of-turn review hop gains a third judgment:
  capture — a demonstrably working approach or diagnosed failure may
  propose one candidate memory per turn (capped 2/session), stored
  untrusted/quarantined and never auto-promoted; with memory configured
  the turn hop now runs every turn end. First `ALTER TABLE` migration
  (pragma-guarded, fixture-tested); new `memories.status` dimension
  filtered to active across every retrieval path; new
  `consolidation_records` audit table mirrored to OTLP. All three spec
  clarifications decided via `decide` under the margin protocol.
  `MEMORY_LAYER.md` amended in-change; artifacts under
  `specs/017-memory-consolidation/`.
* **Push memory (016)** — the push half of `MEMORY_LAYER.md`'s "effortless,
  not manual" contract: a new harness-triggered, memory-gated `surface`
  tool (invoked by an installable `UserPromptSubmit` hook) surfaces the few
  most relevant trusted stored memories into the assistant's context at
  each turn start as clearly-labeled advisory context (verbatim content +
  memory id + trust + a `forget(<id>)` contestability pointer).
  Deterministic end-to-end — no model pass; relevance floor 0.55 / cap 3;
  once-per-session suppression derived from the feature's own audit rows;
  hard 500 ms fail-open budget; new `push_records` audit table mirrored to
  OTLP. Memory-off behavior unchanged; nothing fires until the hooks
  integration is installed. All three spec clarifications were decided via
  `decide` under the order-bias experiment's margin protocol. Spec/plan/
  contracts under `specs/016-push-memory/`.
* **decide order-bias experiment** (`claudedocs/experiments/decide-order-bias/`):
  pre-registered test of the design corpus's "permute order" judge-bias clause
  against the shipped single-pass `decide` — 250 live calls over 70 fixture
  decisions with an identical-order retest arm as the noise floor, including
  a power extension the `decide` tool itself selected (dogfooded, with a
  permuted confirmation pass). Final result: **no order bias at any tested
  k** — 2 options 5%/5% (measured null), 4 options pooled n=40 18.8%/17.5%
  (p=0.51; the interim 30%-vs-10% directional effect was refuted with
  power). Durable findings: sampling instability dominates four-option
  near-ties (17.5% identical-order flips), and the score margin encodes all
  instability — every flip of any kind sat at margin ≤ 16, margin ≥ 17 was
  perfectly stable across the whole experiment. Corpus §4 amended
  in-change; margin-gated permutation is rejected as a feature.
* **Preference enforcement at the end-of-turn checkpoint (015).** The
  `checkpoint_turn` review hop now judges the turn — final message wording
  plus observable in-turn activity — against recalled **trusted** stored
  preferences (the same trusted lesson/fact population the action gate
  treats as constraints) and flags a violation quoting the stored
  preference verbatim with its provenance (memory id, trust standing), so
  the model can revise or explicitly contest it. One hop still (the two
  judgments share the layer's single model pass), flag-only authority
  (never hold, never rewrite), fail-open, cooled down by memory id, and
  byte-identical behavior when memory is unconfigured. New
  `preference_violation` signal kind on checkpoint records; no new tools,
  config, or storage schema. Closes the capture → store → recall →
  **enforce** loop from `PREFERENCE_ELICITATION.md` (amended in-change);
  spec/plan/contracts under `specs/015-preference-enforcement/`.

### Security / Dependencies

Three transitive advisories cleared via three lockfile-only commits (#38).

* `quinn-proto` 0.11.14 → 0.11.16 — high, CVSS 7.5 (RUSTSEC-2026-0185).
  Transitively pulled in via `reqwest`'s `http2` feature. Pulled in
  `chacha20 0.10.1`, `cpufeatures 0.3.0`, and a second `rand 0.10.x` major
  (parallel to the existing `rand 0.9.x` already in the lockfile).
* `anyhow` 1.0.102 → 1.0.104 — unsound `anyhow::Error::downcast_mut`
  (RUSTSEC-2026-0190). Transitively pulled in via `prost-derive` → `prost`
  → `opentelemetry-otlp`.
* `spin` 0.9.8 → 0.9.9 — yanked. Transitively pulled in via `flume`
  → `sqlx`.

Zero `Cargo.toml` changes; three sequential commits, each pinning a
single advisory to its dep bump.

### Fixed

* **Research reported success when an entire phase failed (023).** Per-source
  and per-claim failures degrade a run by design (004 FR-013) — the item is
  dropped, counted, and the run continues. Nothing distinguished *every* item
  failing from a set of pages that genuinely held no claims, so both collapsed
  to the same empty answer with `outcome: success`.

  Found in production: setting a reasoning effort on a call site routed to a
  model that does not support the parameter made every extraction call return a
  400. The run reported `sources_found: 10, sources_fetched: 0`, an empty
  answer, `confidence: 0`, and six plausible-looking gaps — indistinguishable
  from "the web does not know", which was false. Every fact needed to detect it
  was already in the run.

  The search phase had the right rule all along: it propagates when no search
  succeeded. Extraction and verification now do the same. The distinction drawn
  is between *nothing to say* and *a call that failed* — a page that loaded and
  held no readable text produced nothing without failing, so a run whose
  candidates are all unreadable still returns the honest empty answer.
  Degradation is untouched: one surviving source or claim is enough to continue.

* **Research reported confidence 0 for correct answers (021).** The top-level
  `confidence` was `mean(finding_confidences) × coverage`, and coverage was
  `sub_questions.len() − gaps.len()`, divided by the sub-question count. Those
  are two unrelated lists: sub-questions are the falsifiable questions the
  scope phase produces, gaps are free-form phrases the synthesis pass writes,
  and no entry in one corresponds to an entry in the other. Because the gap cap
  (10) exceeds the sub-question cap (7), zero was reachable by construction —
  and reached, on two live runs whose answers were factually correct and whose
  every claim had survived refute-biased verification at ~0.78.

  A confidence of exactly 0 asserts certainty of falsehood. Worse for a caller,
  it makes the field return the same value for a correct evidence-backed answer
  as for a demonstrably wrong one, so a caller that sees 0 on correct answers
  learns to ignore the field entirely — which removes the signal rather than
  making it conservative.

  The synthesis hop now keys each gap to the sub-question it concerns, as an
  index-aligned `gap_targets` array (parallel arrays because Principle II
  requires mode schemas to stay flat and closed — the idiom `decide` already
  uses). The server counts unclaimed sub-questions from those keys. A key that
  is out of range is discarded; several keys on one sub-question leave it
  unsettled once, which is the specific arithmetic whose absence caused the
  collapse. A length mismatch between the two arrays feeds the synthesis
  pass's existing retry and then its existing demotion path — never accepted,
  since reading absent keys as "concerns nothing" would report full coverage
  for a malformed response.

* **Failures that were billed now record their tokens (020).** A truncation and
  a refusal are HTTP 200 responses: the provider ran the model and charged for
  it, then returned a `stop_reason` the contract cannot use.
  `AnthropicClient::interpret` read `usage.output_tokens` to build the
  truncation message and then discarded it, and `run_recorded_usage` wrote an
  empty `ModelUsage` for every error class — so those rows read 0 input, 0
  output, $0.00.

  Investigating it showed the loss was wider than those two classes. When an
  ensemble fails to reach quorum, `dominant_failure` picks one error and drops
  the rest, discarding the tokens of the sibling failures **and** of every pass
  that completed successfully but could not form a verdict. Research's
  extraction and verification phases swallow per-item failures by design, and
  dropped their tokens with them — verification being the tool's dominant cost,
  that was the largest under-report of the three phases.

  The fix is one additive error variant, `AppError::Metered { source,
  input_tokens, output_tokens }`, rather than fields on the two classes.
  `outcome()` and `Display` delegate to `source`, so **the outcome taxonomy is
  unchanged** and attaching usage can never reclassify an error; `root()`
  recovers the wrapped error for matching. Tokens are raw rather than per-model
  because the producers (a client, an aggregator) know the count while the
  recorder knows the model — attributing them at the producer would mean
  guessing. A failure that genuinely cost nothing (timeout, transport error,
  invalid input) carries `(0, 0)` and still records empty.

  Research needed a further fix. Its pipeline can fail *after* most of its
  spend — synthesis is the last phase and propagates — so recording only the
  failing call's own tokens would show a plausible few thousand in place of the
  couple of hundred thousand the run cost. That reads as a real number, which
  is worse than the zero it replaced. The run meter is now attached to any
  error the pipeline propagates. Its per-model breakdown is flattened to totals
  in the process, so with routing configured a rare error row can price
  bulk-tier extraction at the judgment rate; over-estimating one row beats
  losing 99% of its tokens.

  Reviewing the change found the same defect at every other point that
  accumulates tokens and can then fail, so all seven are fixed rather than the
  two the issue named:

  * `decide` and `check` each run a violation-fed retry — two complete model
    calls — and recorded zero when the second attempt was still malformed.
  * `elicit` bills a recall hop before its pass, then two attempts.
  * `save` runs the entire verify ensemble before rejecting a refuted memory.
    That is the most expensive failure the memory layer has.
  * The checkpoint layer fails **open**: `recover` replaced a failed evaluation
    with a fresh zero-token result, so a boundary that ran a review hop and
    then failed reported a free evaluation that silently produced nothing.
    Failing open changes the verdict, not the bill.

  Consequences: spend stops being under-reported, and the per-call output
  ceiling becomes sizable from data, since a truncated call now says how much
  output it wanted before it ran out.

* **018 T012 and T013, deferred at the time, are done (020).** T012 needed a
  tracing capture harness only because the startup report was rendered inline;
  extracting it to the pure `RoutingTable::report` makes what the operator is
  told directly assertable — every call site present, in canonical order, each
  with a non-empty model and a setting that actually exists. T013 asserts that
  routing stays invisible to callers: no tool input property names a model or
  tier, no schema references `PARALLAX_MODEL_*` or `ANTHROPIC_MODEL`, and
  enabling routing adds no tool to the catalog.

* **Routed call sites recorded the wrong model, and were priced at its rate
  (018).** Routing itself worked — a call went to the client its call site
  resolved to — but the eleven single-model tools passed
  `config.anthropic_model` to `run_recorded` as the attributed model, so the
  invocation record named the server-wide default and `cost_usd` was computed
  at that model's rate. A `verify` routed to Sonnet 5 ($3/$15) was billed in
  the record at Opus 4.8 rates ($5/$25). Found by running 018's own T050
  validation sweep against the merged binary. `Parallax` now carries the
  resolved `RoutingTable` and every call site attributes through it; the
  now-redundant global `model` field is removed, which is how the compiler
  confirms no site was missed. Research was already correct — its `RunMeter`
  attributes per hop through the routing table.

  This is the same defect class `/speckit-analyze` caught for the checkpoint
  layer before 018 merged: attribution taken from global config rather than the
  resolved route. That instance was fixed; the eleven identical cases in the
  same file were not. The regression test this shipped without —
  `a_routed_call_site_records_the_model_that_served_it` — asserts that a routed
  tool's record names the routed model, and was confirmed to fail against the
  original bug before being kept.

* **Research verification judged from priors, not evidence (004/D3+D4).**
  The per-claim refute-biased verifier received only source titles/hosts as
  context — the fetched page text was dropped after extraction — so
  "default to refuted when support cannot be established" made verdicts a
  function of the judging model's training cutoff: a live run refuted
  "Rust 1.97.0 was released 2026-07-09" while the official announcement sat
  fetched in its own source list (found by the tool-catalog validation
  sweep). `SourceRecord` now retains the capped readable text internally,
  new `research/evidence.rs` deterministically selects a claim-relevant
  excerpt (word-overlap anchored, ≤3 sources × ≤4 000 chars), the verify
  context carries the excerpts, and the template directs the judge to test
  the claim against them rather than its memory. Nothing new reaches the
  wire (FR-012 re-asserted in tests). The scope prompt also gains a
  same-named-referent disambiguation rule (the sweep's second finding: an
  angle drifted to the RUST video game). 004 D3/D4 amended in-change.
* **Batch-screen false positive on distinct-target batches (006/D5).**
  `normalize_input` no longer drops an input-level `id` as volatile: it
  names the action's semantic target (for `forget` it is the entire
  payload), so dropping it made a finite batch of six distinct deletions
  normalize identically and fire a false repetition flag (017 T019 live
  dogfood finding). Retry loops on the *same* target still match and
  still fire — recall is unchanged; the harness's genuinely volatile ids
  (`tool_use_id`, `session_id`, `request_id`) stay dropped. Ground-truth
  table extended with both directions; `006` data-model amended in-change.

### Changed

* **Research result assembly extracted to `src/research/outcome.rs`.** The
  block was a pure function of finished run state sitting inside the five-phase
  spine; `pipeline.rs` goes from 712 to 642 lines. The extraction is what makes
  the published contract's promise testable — `coverage` equals the settled
  share of `sub_question_status` — which previously held only because two call
  sites happened to read the same helper. Three unit tests now pin it,
  including the case where the gap cap drops a gap whose sub-question stays
  correctly unsettled. No behaviour change.

* **`research` output: three fields added, one redefined (021).** Added
  `coverage` (settled share of scoped sub-questions), `refutation_rate`
  (refuted share of verified claims), and `sub_question_status` (each
  sub-question with a settled flag — the published basis for `coverage`, so the
  figure is checkable from the response rather than taken on trust). Those are
  compatible additions.

  **`confidence` keeps its type and range but changes value**: it is now the
  mean support of the findings the answer asserts, with no coverage factor. The
  prior figure remains exactly derivable as `confidence × coverage`, so nothing
  a caller could compute before is lost.

  Splitting the field was chosen over blending the multiplier so it could
  attenuate without annihilating. The blend would also have changed the field's
  stated definition — deleting the invariant that zero coverage forces zero
  confidence — while being the *less* detectable change, silently rescaling an
  unchanged field name where a new key is something a value-reading caller can
  notice. It would also have introduced a free parameter derived from nothing,
  making two deployments emit incomparable values under the same contract
  version.

  `gaps` deliberately keeps its wire shape as plain strings.

  `stats.stop_reason` gains `malformedsynthesis`. A synthesis whose gap list
  and sub-question keys disagreed in length twice used to demote under
  `grounding`, telling the caller the answer could not be cited when the
  grounding gate had never been reached. 004 FR-007 requires the accounting be
  honest, so the two now have separate reasons and separate demotion text. Making each gap an
  object carrying its key was rejected: a breaking type change for callers that
  read gaps as text, and gaps raised by the grounding gate have no sub-question
  to name and would have had to carry a false one.

* **Quick research budget raised 150 000 → 350 000 tokens (019).** The 004
  evidence-grounding fix gave each per-claim verification hop a real source
  excerpt instead of a title — the change that made verification worth running
  — and every quick run then tripped a ceiling that had not moved, dropping
  roughly 40% of its claims.

  The first attempt at the new number was 250 000: the tier's original 1.43
  headroom ratio (150 000 / 104 783) applied to a post-004 measurement of
  174 952. **That measurement was taken from a run which had itself stopped
  early and dropped 43 of 89 claims** — the cost of an incomplete run, and so
  an understatement. A run that completes measures **239 371 tokens** (77
  claims extracted, 77 verified, 8/8 sources — near this tier's structural
  maximum), which is 95.7% of 250 000. Applying 1.43 to the complete-run cost
  gives ~342 000, rounded to 350 000. Sizing a ceiling from a run that hit that
  ceiling is circular and always yields a number that is too low.

  Standard and deep are unchanged, deliberately — see the `depth` column above
  for why they are currently unmeasurable. Corpus amended in the same change
  (`RESEARCH_PRIMITIVE.md` §5, `specs/004-research-layer/research.md` D8).

* **Per-call output ceiling raised 16 000 → 32 000 tokens (019)**, after a real
  truncation. The 018 family sweep declared 16 000 validated, but it exercised
  only trivial inputs; a genuine four-option `decide` with a long context on
  `claude-sonnet-5` exhausted it. The mode schemas bound the *answer*, and on a
  model that reasons before answering the reasoning term is the larger one and
  is bounded by no schema. This number is **not** measured — the largest
  successful thinking-inclusive output on record is 3 135 tokens, and how far
  past 16 000 the failing call went is unknown, because a truncated invocation
  currently records zero usage (see *Known issues*). It should be re-derived
  once that is fixed. A post-change attempt to reproduce the failure (a
  four-option `decide` with a ~2 000-token context on `claude-sonnet-5`)
  **did not** reproduce it: the call returned 445 output tokens, nowhere near
  either ceiling. So the raise is currently unfalsified rather than validated,
  and the mechanism behind the original truncation is still unexplained.

* **OTel GenAI semconv deprecations cleared (#39, `eeb1608`).** The
  upstream `opentelemetry_semantic_conventions` crate deprecated its
  `attribute::GEN_AI_*` constants in the 0.32 train; CI's `-D warnings`
  turned the deprecations into 13 hard errors at `src/observability.rs`
  and 4 more at `examples/spike_otlp.rs`. The five canonical
  attribute-name strings (`gen_ai.operation.name`, `gen_ai.request.model`,
  `gen_ai.token.type`, `gen_ai.usage.input_tokens`,
  `gen_ai.usage.output_tokens`) are now declared locally — mirroring the
  existing `GEN_AI_PROVIDER_NAME` precedent at `src/observability.rs:37`.
  Stable OTel spec identifiers; the test assertions in
  `src/observability.rs` and `tests/integration.rs` already verify
  against them as raw string literals, so the local consts and the wire
  format share one source of truth.
* **`unstick` tolerates client `blocked`-field arg-drop (#40,
  `0ad8fdc`).** Some MCP clients intermittently drop the `blocked` field
  from the emitted tool-call while `goal` survives. `UnstickParams`
  gains `#[serde(default)]` on `blocked` (advertised optional in the
  contract: `required: ["goal"]`) and a `normalize()` pass at the top of
  `run()` that recovers `blocked` from a `||BLOCKED|| <text>` marker
  appended to `goal`. Unconditional marker strip prevents prompt-leak
  on dual-encoded calls; multi-marker robustness takes only the first
  post-marker segment so a retry-encoded client does not leak the marker
  literal into the recovered `blocked`. Four new `modes::unstick` tests
  cover dropped/missing recovery, dual-encoding, no-marker idempotence,
  and multi-marker first-segment-only.

### Docs

* **`CLAUDE.md` Active-Feature staleness pruned (#41, `a62e47e`).** Two
  stale notes pointed at a "needs the rebuilt binary" precondition and
  an "uncommitted at last check" reminder from before the unstick work
  landed. Both removed now that the rebuilt binary is shipping and the
  unstick work is on its own PR.
* **Dogfood mechanism re-verification stamps (#42, `500b5a6`).**
  Three-line diff in `specs/012-diverge-perspectives/tasks.md` (T013),
  `specs/013-decide-methodology/tasks.md` (T010), and
  `specs/014-preference-elicitation/tasks.md` (T012) — adds a
  "Mechanism re-verified 2026-07-20" sub-bullet below each existing
  inline 2026-06-14 live result. The mode source is unchanged across
  #38–#41, so the 2026-06-14 live `SC-*` results stay structurally held
  (model + `FR-*` contract unchanged); the offline integration suite
  (`cargo test --test integration`, 60/60) re-proves the mechanism.
  Live re-verification against the rebuilt binary is open follow-up
  work for the maintainer.
