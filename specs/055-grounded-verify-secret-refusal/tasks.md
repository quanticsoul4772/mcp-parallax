---

description: "Task list for 055: grounded_verify refuses files whose names mark them as secrets"
---

# Tasks: grounded_verify Refuses Files Whose Names Mark Them as Secrets

**Input**: Design documents from `/specs/055-grounded-verify-secret-refusal/`

**Prerequisites**: spec.md (sixth version), plan.md (re-checked against the sixth version), research.md, data-model.md, contracts/refusal.md

**Second version**, corrected after a review against the code.

**Tests**: REQUIRED (Constitution Principle IV). Every piece is test-first: the test is written, run, and its stated first result recorded, before the code it checks.

**Every step that changes a file, commits, pushes or calls a model is asked of the owner first.** If a test's first result is not the one stated here, stop and tell the owner; do not adjust the test to fit.

**The gate** is `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test` (run on this machine with `-j 2` on the clippy and test steps). It does not include the markdown check, which is its own step (T044).

**No assertion about reading, confinement, ceilings, judging, settlement or confidence in an existing test may be weakened.** Where a test's calls change shape, the task says what it asserted before and asserts after.

**How the reader is split without breaking the build.** The new calls are added beside the old ones. Callers move over one group at a time, and the full gate passes at the end of each phase (T011, T019, T025), and once mid-phase after the result fields (T022). The old calls are removed last. New code has no placeholder bodies: each new test's first result is a compile failure naming exactly what is missing, and where empty values let it compile, the expected result is stated bullet by bullet, with any bullet that already passes marked as a pin.

- New on the trait: `check`, `read_checked`, and `list_flagged` (returning `ListedFile`s).
- Old, removed in T024: `read`, `list_files` returning plain paths, and the old `expand`. `list_flagged` is then renamed `list_files` and the flagged expansion `expand`, so the final shape is the plan's.

## Format: `[ID] [Story] Description`

Tasks run in order. None is marked parallel.

---

## Phase 1: The rules as text (US1)

**Goal**: FR-001's rules exist as pure functions, tested for both platforms on every system.

- [X] T001 [US1] Windows only, before anything else: with a temporary test in `src/grounded/reader.rs`, write an alternate data stream on a file in a temporary folder, read it back with the standard library, then pass the stream's name to today's `read`. Record what comes back (the stream's content, the base file's content, or an error). Remove the temporary test. This is the record T031 cites.
- [X] T002 [US1] Create `src/grounded/secrets.rs` with only its types (`Platform`, `Simplified`, `Rule`), and declare the module in `src/grounded/mod.rs`. No tables and no functions yet. Gate. **Expected: passes** (nothing uses it yet).
- [X] T003 [US1] Add tests of `simplify` for both platforms: `.` and empty parts dropped; `a/../b`; `.ssh/../README.md` becomes `README.md`; a leading `..` kept; `../..`; a trailing separator; on Windows only, `\` as a separator and each leading form (`\\?\UNC\`, `\\?\`, `\\.\`, `\??\`, each with both slashes) and a drive letter set aside; on `Other`, a backslash is an ordinary character. Run. **Expected: fails to compile**, naming `simplify`. Then implement `simplify`. **Expected: pass.**
- [X] T004 [US1] Add the separately typed list (SC-001's third binding): at least twenty paths, written out by hand in the test and not derived from the tables, covering every rule, each with the rule expected. A backup form reports the rule its reduced name matches (rule 4 is never itself the answer); when more than one of rules 1 to 7 matches, the lowest number is reported; rule 8 is reported only when none of 1 to 7 matches. Include: `.env` (1), `config/.ENV` (1), `.env.local` (2), `prod.env` (3), `id_rsa` (1), `keys/id_ed25519` (1), `server.pem` (3), `a/b.key` (3), `id_rsa.bak.old` (1), `.env~` (1), `secrets.yaml.orig` (the rule its reduced name matches in the tables), `.ssh/known_hosts` (5), `.ssh.bak/known_hosts` (5), `x/secrets/y.txt` (5), `.git/config` (6), `.git/config.bak` (6), `project.git/config` (6), `.config/gh/hosts.yml` (7), `.config.old/gcloud/x` (7), `terraform.tfstate.1699999999.backup` (3), `client_secret_1.json` (2), and on Windows `notes.txt:hidden` (8) and `C:\r\notes.txt::$DATA` (8). Run. **Expected: fails to compile**, naming `refusal`. Record.
- [X] T005 [US1] Add the must-read list of SC-003 as a test: each name is not refused, on both platforms; plus `src/a.rs:12` is refused on Windows by rule 8 and read on `Other`. Run. **Expected: fails to compile**, naming `refusal`, until T006. Record.
- [X] T006 [US1] Add the rule tables with FR-001's values and implement `refusal`. Run T004 and T005. **Expected: all pass.**
- [X] T007 [US1] Add table-driven tests: every entry of every table in lower, upper and mixed case, as a file name, under a folder, and (rules 6 and 7) with the pair spanning from a root-like prefix; backup forms single and repeated on file names, on folder names, on git configuration names and on each folder of a path sequence. Run. **Expected: pass** (these iterate the tables and so cannot catch a wrong table; T004 is the check that can). Gate.

**Checkpoint**: the rules exist and are tested as text. Nothing calls them yet.

---

## Phase 2: Startup refusal (US1, edge case)

- [X] T008 [US1] In `src/grounded/reader.rs` tests, add: a root inside a folder named `.ssh`, a root that is itself named `secrets`, a root inside `.config/gh`, and a root inside `.ssh.bak`, each fails to construct with a configuration error whose text is exactly the data model's, naming the folder (for the pair, both) and the root; a root inside `.git`, a root named `.config`, and a root named `x.git` each construct. Run. **Expected: the four refusals fail** (the reader constructs today); the three that construct pass. Record.
- [X] T009 [US1] Add the new `ConfigError` variant in `src/error.rs` and the check in `SystemSourceReader::new`. Run T008 and the existing `nonexistent_root_is_a_config_error`. **Expected: all pass.** Gate.

---

## Phase 3: The new reader calls, beside the old (US1)

**Goal**: `check`, `read_checked` and `list_flagged` exist and are proven. `read` and `list_files` still exist, behave as today, and keep all their tests. No caller has moved. Not a commit point of its own.

- [X] T010 [US1] Write the reader's new tests first, in `src/grounded/reader.rs`:
  - `check` then `read_checked` on an ordinary file returns the same text and byte count `read` returns;
  - `check` refuses a file inside `.ssh` reached through an ordinary-named link (symbolic link elsewhere; folder junction on Windows) with a `Refused` failure, not an `Other`;
  - `check` returns `Other` holding invalid input, with today's text, for a missing file, a path escaping the root, a network path, and a link out of the root;
  - `read_checked` fails with `Other`, with the data model's "locator changed while being read" text, when the name now leads to a different file than `Checked` holds, and with `Refused` when it now leads to a refused one;
  - `read_checked` returns today's text for an oversized file, a non-text file, an empty file, and a start line past the end;
  - `list_flagged` flags `.env` and does not flag `a.rs`; does not descend through a linked folder; and, with a root named `.config` holding `gh/x` and a root named `x.git` holding `config`, flags both (rules 6 and 7 spanning the root).
  The tests name `Checked`, `CheckFailure` and `ListedFile` in their assertions. A test that cannot create its link fails with a message. Run. **Expected: fails to compile**, naming exactly `check`, `read_checked`, `list_flagged`, `Checked`, `CheckFailure` and `ListedFile`. Record.
- [X] T011 [US1] In `src/traits/source.rs`, add `Checked`, `CheckFailure`, `ListedFile` and the three trait methods; extend `mock_source_reader_honors_the_contract` with the three new calls, keeping its existing assertions. In `src/grounded/reader.rs`, implement them: `check` is today's `resolve`, then the rules on the whole resolved path; `read_checked` resolves the caller's spelling again, repeats confinement and the rules, requires the same resolved path as `Checked` holds, then today's size, text, empty and range checks; `list_flagged` is today's walk with each entry flagged. No placeholder bodies and no lint exemptions. `read` and `list_files` are left as they are. Run T010. **Expected: all pass.** Gate. **Expected: passes.**

---

## Phase 4: Globs and assembly move over (US1, US2)

**Goal**: assembly runs in the FR-002 order on the new calls. Phases 4 and 5 are committed together: between them a glob would drop files without saying so. The old `expand` stays, unused, until T024.

- [X] T012 [US2] In `src/grounded/glob/expand.rs`, write the new tests first, against a text-stage function and a flagged expansion that returns matched files and refused files:
  - a glob starting in `.ssh`, in `.ssh.bak`, in `.config/gh`, and with one brace branch in `.ssh`, is refused by the text stage with the data model's "glob refused" text, and the listing double is never called;
  - a glob starting in a folder named `credentials` is not refused (only the folder rules apply to a starting folder);
  - a listed file that is flagged and matches the pattern lands in the refused files and not in the matched files; a flagged file the pattern does not match appears in neither; an unflagged matching file lands in the matched files;
  - every match flagged: no matched files, all of them refused (the all-refused error is assembly's).
  Run. **Expected: fails to compile**, naming exactly the two new functions and the new result type. Record.
- [X] T013 [US2] Implement, beside the old `expand`: the text stage (pattern validity, the leading-`!` refusal, the starting folder or folders, the folder rules on each) and the flagged expansion on `list_flagged`, splitting matches only after the pattern filter. The flagged expansion runs the text stage itself before listing, so a malformed or refused glob gets the same error whether or not phase one ran. The old `expand` is not changed. Run T012. **Expected: pass.**
- [X] T014 [US1] In `src/grounded/assemble.rs`, write the new tests first, against a recording test double that implements the whole trait and logs every `read`, `check`, `read_checked`, `list_files` and `list_flagged` call. Its old `read` returns the file's content and never fails, so no bullet can pass through it:
  - an ordinary file then one entry of each kind of rule (exact, prefix, ending, contains, backup, part, git configuration, sequence): the error is the data model's refusal text, exactly; **zero** `check` and zero reads of any kind;
  - an ordinary file then `docs/known_hosts` where the double's `check` returns `Refused`: the error is the refusal, naming the path as written and nothing the double resolved it to; `check` called for both; **zero** reads. **This is the test that fails unless every file is checked before any is read.**
  - an exact path whose `read_checked` returns `Refused`: the call fails with the refusal text; one whose `read_checked` returns `Other`: the call fails with that error;
  - an ordinary file then a path with only a start line, then a path with a start line of 0, then a path whose start line is after its end line: each fails with the text the reader gives today (`reader.rs` pairing and `slice_lines` texts), with **zero** `check` calls;
  - a path with a colon, with the Windows platform value: the data model's colon text, not the "likely secret" one;
  - a glob of two ordinary files and one refused: the call succeeds; two read; the refused list holds one path; the refused file was never checked or read;
  - a glob whose matches are all refused: the all-refused text naming that glob and the number; no reads;
  - a glob match that `check` refuses at read time, and one that `read_checked` refuses: each left out, and the call succeeds with it in the refused list; when that leaves its glob with nothing, the all-refused error naming that glob;
  - a glob match that fails at read time with `Other`: the call fails with that error;
  - a ceiling of two with a glob of two ordinary files and three refused: accepted;
  - two globs matching the same refused file: the call succeeds and the refused list holds it once;
  - sixty-six exact paths against a ceiling of sixty-four: `too many locators (65, max 64) after glob expansion`, zero `check` calls (a running count, not a total);
  - a glob matching ten files, then sixty exact paths: `too many locators (65, max 64)`, zero `check` calls;
  - sixty-five exact paths, then a glob: the same error, and the glob's folder is never listed.
  Run. **Expected: fails to compile**, naming exactly the refused list on the assembled evidence and the platform field in the assembly settings. Record.
- [X] T015 [US1] Add the refused list to the assembled evidence in `src/grounded/assemble.rs`, always empty, and the platform field to `AssemblyLimits` in `src/grounded/mod.rs`, set but not read. Production sets the real platform where the limits are built in `src/server.rs`; the test helper `limits()` in `src/modes/grounded_verify.rs` and the assembly tests set it explicitly. Run T014. **Expected per bullet:** the last three **pass**, as pins: today's running count already gives these results with no `check`; they are there so the reordering cannot change them. Every other bullet **fails** on an assertion: today's assembly calls `read`, not `check`, knows no refusal, and leaves the refused list empty. Record each.
- [X] T016 [US1] In `src/modes/grounded_verify.rs`, add a test through the tool's run function with a recording reader double and a model double: for one entry of each of the eight kinds of rule, the call fails, zero files are checked or read, and zero model calls are made. Run. **Expected: fails**: today the file is read, and the model double, which expects no call, panics on the call it receives. Record.
- [X] T017 [US1] Implement assembly in the FR-002 order on the new calls: phase one with the pairing and range checks; phase two, listing only, with today's running count; phase three `check`; then reads. Move the ten existing assembly tests to the recording double, each asserting what it did:
  - `assembles_verbatim_with_manifest_in_order`, `a_glob_expands_into_concrete_manifest_entries`, `a_glob_and_an_exact_path_mix_in_one_call` (this one already proves the manifest follows locator order), `assembly_is_deterministic_for_the_same_inputs`: same text and manifest.
  - `a_glob_with_a_range_is_rejected`, `a_zero_match_glob_aborts_named`, `empty_locator_set_is_rejected`, `byte_ceiling_is_enforced_and_names_the_overflow`: same error text.
  - `expansion_past_the_locator_ceiling_is_rejected`: still asserts the error contains "too many locators".
  - `any_failing_unit_aborts_the_whole_call_named`: the failing unit's error now comes from `check`; same text asserted.
  Run T014 and the ten moved tests. **Expected: all pass.**
- [X] T018 [US2] Move the tool's sixteen tests in `src/modes/grounded_verify.rs` to the new calls; without this they fail, since assembly no longer calls `read`. The helpers `ok_reader` and `reader_returning` are rewritten once. These then assert exactly what they did: `a_lone_computable_pass_is_accepted_not_dropped`, `a_lowered_pass_count_runs_and_is_reported`, `a_single_needs_computation_among_three_is_not_a_majority`, `advisory_missing_evidence_alone_does_not_force_inconclusive`, `compound_claim_with_a_judgment_finding_abstains`, `computable_byte_and_match_specs_settle`, `computable_line_count_over_1000_settles_supported`, `computable_line_count_over_5000_settles_refuted`, `disagreeing_compute_specs_abstain`, `majority_needs_computation_returns_inconclusive_routed_to_check`, `multi_source_computable_abstains`, `non_computable_judgment_path_has_no_executed_form`, `out_of_class_property_abstains`, `verdict_manifest_and_completeness_are_assembled`. Two are named apart:
  - `an_unresolvable_locator_aborts_before_any_model_call` builds its own mock, whose `read` errors today; after, its `check` returns that error, and the same "source not found" text is asserted.
  - `empty_claim_is_rejected_before_any_read_or_model_call` **changes meaning**: it must now assert that **neither** `check` **nor** `read_checked` is called.
  Run T016. **Expected: passes.**
- [X] T019 [US1] Gate. **Expected: passes.** Confirm from the diff that in T017 and T018 only the doubles' calls changed, apart from the one named meaning change. Record per file.

---

## Phase 5: The result, then removing the old calls (US2, US1)

- [X] T020 [US2] In `src/modes/grounded_verify.rs`, write the new tests first:
  - with one file left out, the result has the list, count one, `evidence_reduced` true, the notice with the count; and the same confidence as the same passes give with nothing left out (asserted equal, with unanimous passes and with a two-to-one split);
  - with nothing left out: an empty list, zero, false, an empty notice;
  - sixty files left out: the list holds fifty, sorted, with no duplicates, and the count is sixty;
  - a countable claim over one read file and one left out returns inconclusive, no `executed_form`, and the notice **with** its closing sentence;
  - the same claim with nothing left out settles as today;
  - a countable claim outside the settled class, with a file left out: the notice **without** the closing sentence;
  - a countable claim over two read files and one left out: the notice **without** the closing sentence;
  - a compound countable claim (a pass reports a judgment finding) over one read file and one left out: the notice **without** the closing sentence.
  Run. **Expected: fails to compile**, naming exactly the four fields. Record.
- [X] T021 [US2] Add the four fields to `GroundedVerdict` with empty values (an empty list, zero, false, an empty notice), and in the **same task** add them to `specs/008-grounded-verify/contracts/grounded_verify.tool.json`, including its list of required fields, checked by hand since the contract test does not compare that list. Run T020 and `derived_schemas_match_the_contract_file`. **Expected per bullet:** "nothing left out" and "settles as today" **pass**, as pins: they describe today's behaviour. Every other bullet **fails** on an assertion: the fields are always empty and settlement is not yet skipped. The contract test passes. Record each.
- [X] T022 [US2] Fill the fields from the assembled refused list, skip settlement when the count is above zero, and write the notice. Run T020. **Expected: all pass.** Gate.
- [X] T023 [US1] Search the crate for calls of `SourceReader`'s `read`, of its old `list_files` and of the old `expand`. The trajectory reader's `read` in `src/checkpoint` belongs to another trait and is not counted. **Expected: only** the trait, the real reader, the doubles, the reader's own tests of `read` and `list_files`, the eleven expand tests, and `mock_source_reader_honors_the_contract`. Record the search.
- [X] T024 [US1] Remove `read`, the old `list_files` and the old `expand`; rename `list_flagged` to `list_files` and the flagged expansion to `expand`. Move the tests that used them, each asserting what it did:
  - `mock_source_reader_honors_the_contract`: its `read` assertions move to `check` then `read_checked`.
  - `the_displayed_root_is_the_configured_directory`: `check` then `read_checked` returns `"inside"`; its root-display assertions are unchanged.
  - `a_network_path_outside_the_root_is_refused_from_its_text`: its three network paths now error from `check`, with the escape text on Windows and, elsewhere, "source not found" or the escape text, as today; its screening assertions are unchanged; its verbatim inside read becomes `check` then `read_checked`, still `"inside"`; its outside local path errors from `check` with the escape text.
  - `an_inside_absolute_path_is_accepted_in_any_case_and_slash_direction`, `whole_file_read_returns_verbatim_with_byte_len`, `line_range_is_one_based_inclusive`: `check` then `read_checked`; same content and byte assertions.
  - `range_end_past_eof_clamps_but_start_past_eof_errors`: the clamped read is unchanged in result; the past-the-end error comes from `read_checked` and is asserted as an `Other` fault holding invalid input, with the same text.
  - `empty_file_is_a_named_error`, `non_text_file_is_rejected`, `a_file_over_the_per_file_limit_is_rejected_before_read`: `check` succeeds; the same text now comes from `read_checked`.
  - `an_absolute_path_works_inside_the_root_and_is_named_outside_it`: first half `check` then `read_checked`, still `"inside"`; second half the same text from `check`.
  - `missing_file_is_a_named_not_found_error`: the same text, from `check`.
  - `traversal_outside_the_root_is_rejected`: from `check`, now asserted as an `Other` fault holding invalid input (today it asserts only invalid input), so a refusal could not pass it; `read_checked` is never reached.
  - `symlink_escape_is_rejected`: from `check`; keeps its "escapes the source root" text assertion and also asserts an `Other` fault holding invalid input; `read_checked` is never reached.
  - `list_files_does_not_follow_symlinked_directories`, `list_files_walks_only_the_named_folder`: compare the path half; same expected paths.
  - `mismatched_single_bound_is_rejected`: removed from the reader with the call it tested; its text is asserted by the assembly test written in T014. **A removed test; the assertion lives on there.**
  - The eleven expand tests move to the renamed `expand`. Each compares the file half of the result and keeps every other assertion:
    - `a_glob_with_no_slash_lists_only_the_roots_own_files`: keeps "folder is empty and no descent" and the five expected file lists.
    - `a_leading_brace_list_searches_each_named_folder`: keeps the recorded folders (`src`, `tests`; then `src/unit`, `tests/unit`), that descent is on, the expected files, and the "matched no files" text.
    - `a_glob_with_no_folder_in_front_is_refused_without_listing`: keeps the exact text for five patterns, with no listing.
    - `whole_pattern_negation_is_refused_by_name`: keeps the exact text for three patterns with no listing, "only `src` is listed" and the expected file.
    - `a_glob_folder_that_is_not_a_plain_relative_folder_is_refused_without_listing`: keeps the exact text for twelve patterns with no listing, "`.github/workflows` is listed" and its file.
    - `a_glob_is_listed_only_inside_its_leading_folder`: keeps, for four patterns, "this folder, with descent, exactly once", and the "matched no files" text.
    - `a_glob_matching_nothing_names_the_root`, `zero_match_is_a_named_error`, `malformed_pattern_propagates_named`: keep the exact text.
    - `expands_sorted_and_filtered_to_the_match_set`, `doublestar_is_recursive`: keep the expected lists.
    The shared stand-in `reader_with` is rewritten once.
  The canonical-prefix and missing-root tests make no read and are unchanged.
  The tests and doubles written in this feature change only their names: T010's tests call `list_files` for `list_flagged`; T012's tests call `expand` for the flagged expansion and set up `expect_list_files` for `expect_list_flagged`; the recording doubles from T014 and T016 drop `read` and the old `list_files` and rename `list_flagged`; the `list_flagged` call T011 added to `mock_source_reader_honors_the_contract` becomes `list_files`.
- [X] T025 [US1] Gate. **Expected: passes.** Confirm from the diff that in T024 only the calls changed, apart from the named removal and the two assertions tightened to the other-fault kind. Record the gate output and per-file line counts. (SC-007)

**Checkpoint**: commit point for phases 4 and 5 together, when the owner says.

---

## Phase 6: Real-disk tests (US1, SC-002)

These go through `assemble` with the real reader. A test that cannot create its link or stream fails with a message.

- [X] T026 [US1] All systems: with a folder `.ssh` inside the root, `.ssh/../README.md` is read. A root named `.config` holding `gh/x`: `gh/x` is refused. A root named `x.git` holding `config`: `config` is refused.
- [X] T027 [US1] Windows: a `\\?\C:\` path, with each slash direction, to an ordinary file inside the root is read.
- [X] T028 [US1] Windows: a full non-verbatim path to a refused file with a trailing dot, and with a trailing space, using `id_rsa` (not `.env`, whose trailing-dot form is caught from text by another rule). **Expected: refused, with the "likely secret" text.** If it is instead read or reported not found, stop and tell the owner (research D6.3).
- [X] T029 [US1] Windows: the stream test. Write a stream on a file in a temporary folder and read it back with the standard library; if either fails, the test fails with a message. Then: a locator naming the stream on an ordinary file is refused with the colon text; one on `.env` likewise. The test's comment cites what T001 recorded.
- [X] T030 [US1] Windows: a folder junction `docs` to `.ssh`, both inside the root, made with `mklink /J`; `docs/known_hosts` is refused, and the error does not contain `.ssh`. And a junction inside a folder that a glob walks: the walk does not descend through it and nothing under it is read or listed.
- [X] T031 [US1] Other systems: a symbolic link `docs` to `.ssh`, and a symbolic link `notes.txt` to `.env`, each refused, with an error that does not contain the target's name; a file whose name contains a colon is read.
- [X] T032 [US1] Gate on Windows. State in the task's record which of T026 to T031 ran on this machine and which run only in the pull-request checks.

---

## Phase 7: Visibility (US3)

- [X] T033 [US3] In `tests/integration.rs`, extend `the_served_grounded_verify_description_is_the_registered_one` to also assert the registered description contains the FR-011 text exactly once. Run. **Expected: fails** on the missing text. Record.
- [X] T034 [US3] Add the FR-011 text to `GROUNDED_VERIFY_DESCRIPTION` in `src/modes/grounded_verify.rs`. Run T033. **Expected: fails** on the two copies now differing. Record. Then add it to the literal in `src/server.rs`. **Expected: passes.**
- [X] T035 [US3] Add end-to-end tests in `tests/integration.rs`, through the served tool with the real reader, each asserting no model call was made where the call is refused: one entry of each kind of rule as an exact path (exact, prefix, ending, contains, backup, part, git configuration, sequence); a glob over two ordinary files and `.env` returns a verdict with the four additions; a glob starting in `.ssh` is refused. The existing integration tests of the tool are unchanged.
- [X] T036 [US3] Gate.

---

## Phase 8: The record (US3, FR-012, FR-013)

Each document is shown to the owner where the task says so.

- [X] T037 [US3] Write the binding test: parse the fenced block in `specs/055-grounded-verify-secret-refusal/contracts/refusal.md`; fail on any line that is not one of the eight kinds; assert the number of lines of each kind (55 exact, 4 prefix, 20 ending, 1 contains, 6 backup, 10 part, 2 gitconfig, 2 sequence); assert each list equals the matching table in `secrets.rs` in both directions. Run. **Expected: passes.** Then prove it can fail: temporarily change one line of the block, see the failure, restore it, and record both runs.
- [X] T038 [US3] `docs/design/NEW_SERVER_DESIGN.md`: a dated amendment (FR-012 item 1). **New text longer than ten lines in the source of truth: the owner is shown its outline and asked before it is written.**
- [X] T039 [US3] `specs/008-grounded-verify/spec.md` and `contracts/grounded-verify.md`: the one exception to all-or-nothing; the new errors; the four result additions.
- [X] T040 [US3] `specs/009-glob-locators/spec.md` and `contracts/glob-locator.md`: the "silently dropped" wording; "all-or-nothing" and "output unchanged"; how refused files count; the superseded requirement that exact-path code and tests are unchanged.
- [X] T041 [US3] `specs/011-grounded-compute-settle`: spec, contract and data model state that settlement is not used when any file was left out.
- [X] T042 [US3] `SECURITY.md`; the README's row for the tool and its description of the root; the tool's line in `src/main.rs` help text; the server instructions text in `src/server.rs`; the header of `examples/acceptance_grounded_verify.rs`; the opening documentation of `src/traits/source.rs`, `src/grounded/mod.rs`, `src/grounded/assemble.rs` and the run function in `src/modes/grounded_verify.rs`.
- [X] T043 [US3] `CLAUDE.md`: the tool's description. **An instruction file: the owner is shown the exact before and after and asked.** `CHANGELOG.md`: one entry under Unreleased; the owner is shown it before it is written.
- [X] T044 [US3] Run the markdown check on every document written or amended, as its own command. Then the gate. Check SC-008 by hand against FR-012's seven items.

---

## Phase 9: Review and close

- [X] T045 Independent review of the whole diff by a fresh reviewer given the diff and told to trace every caller, to look for any file the rules intend to refuse that could still be read, and for any change to a call with nothing refused. The owner is shown the brief and asked first. Findings go to the owner, most severe first.
- [X] T046 Ask the owner: whether to add a Windows check to pull requests; and whether a live call is made before the pull request (the project's note says a model-facing description or schema change must pass the live example first; the vehicle is `examples/acceptance_grounded_verify`; state how many model calls it makes and their price from the project's price table).
- [X] T047 Mark every task in this file done with a capital X, this one included; the test in `src/main.rs` counts exactly that form and requires none left open. Then `CLAUDE.md`'s Active feature section: the owner is shown the exact before and after and asked. Gate afterwards.

## Delivery (steps, not counted tasks)

These follow T047, so that everything written is in what is pushed. Each is asked.

1. Commits: phases 1 to 3 may be one; phases 4 and 5 are one.
2. Search the commits for the Windows account name before pushing.
3. Push, then the pull request. It states: what the guard stops and what it does not; that a call with nothing refused is unchanged apart from which fault is reported when there are several; which tests ran on Windows only; the reviews at each stage; what is deferred.

---

## Dependencies

- Phase 1 before everything: the rules are used by every later phase.
- Phase 3 is additive and leaves behaviour unchanged.
- T017 and T018 land together: once assembly stops calling `read`, the tool's tests need their doubles moved.
- T016 before T017, so the tool-level refusal test is seen failing.
- Phases 4 and 5 are committed together.
- T024 after T023: the old calls are removed only when nothing else uses them.
- T034's order (constant first, then the served literal) is what makes T033 fail a second time for its stated reason.

## Requirement coverage

| Requirement | Tasks |
|---|---|
| FR-001 | T003 to T007 |
| FR-002 | T010 to T017 |
| FR-003 | T008, T009 |
| FR-004, FR-005 | T014, T016, T017, T030, T031, T035 |
| FR-006 | T012 to T015, T017 |
| FR-007 | T014, T020 to T022 |
| FR-008, FR-009, FR-010 | T014, T020 to T022 |
| FR-011 | T033, T034 |
| FR-012 | T021, T038 to T043 |
| FR-013 | T037 (the contract page exists from planning) |
| FR-014 | T017, T018, T019, T024, T025 |
| FR-015 | T025 (no setting; prompt and pass schema untouched, confirmed in the diff) |
| SC-001 | T004, T007, T014, T016, T035, T037 |
| SC-002 | T027 to T030, T032 |
| SC-003 | T005 |
| SC-004 | T014, T020 |
| SC-005 | T014 |
| SC-006 | T020 |
| SC-007 | T017, T018, T019, T024, T025 |
| SC-008 | T044 |
