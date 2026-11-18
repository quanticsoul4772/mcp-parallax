# Feature Specification: grounded_verify Refuses Files Whose Names Mark Them as Secrets

**Feature Branch**: `055-grounded-verify-secret-refusal`

**Created**: 2026-11-17

**Status**: Draft (sixth version; narrowed after review of the first, corrected after each of four further reviews, the last two against the code)

**Input**: User description: "grounded_verify: confine each session to its own project and refuse secret-looking files". The owner then split the work: this specification is the refusal of secret-named files only. Confining each session to its own project is a separate, later specification (see Deferred).

## Why this exists

`grounded_verify` reads the files a calling agent names and sends their text
to the model provider to be judged. Its findings can quote that text back to
the agent. Its only limit today is one configured folder, the source root:
any text file inside the root can be read.

An agent's own file-reading tool is subject to the owner's permission prompts
and path rules. A `grounded_verify` call is approved per tool, not per path,
so it can reach files those guards would stop. Nothing in the tool treats a
file named `.env` differently from a file named `README.md`.

On 2026-11-17 the root was widened to the folder holding all of the owner's
projects, for about two hours, and narrowed again when this was pointed out.
The server's records for that window hold five calls to the tool. Those
records carry a session number, token counts and an outcome; they carry no
file names and no identity. That the five calls were the developer's own
checks, and what they named (one 19-byte ignore file and four names that do
not exist), is known from the developer's session, not from the records.

This feature makes the tool refuse files whose names mark them as secrets.

## What this protects against, and what it does not

**It protects against** an agent naming a secret file by mistake, sweeping
one up in a broad glob, or doing so because text it read told it to.

**It does not protect against** any of the following, and nothing in this
feature may be described as if it did:

- A secret kept in a file with an ordinary name.
- A secret-looking name that is not on the list. The list is fixed and will
  always trail new tools.
- An agent that renames or copies a secret file to an ordinary name and then
  names that. An agent with a shell can do this.
- A hard link: a second ordinary name for the same file.
- A swap made in the instant between a file's last check and its being
  opened: another process replacing a folder in the path with a link. Every
  file is checked again immediately before it is read, which narrows this to
  that instant; it does not close it. Closing it is named in Deferred.
- Anything an agent's own file-reading tool can do.

It is a guard against accident and simple misdirection, not a boundary.

**What a refusal reveals.** A refusal decided from the path as written is
given before the disk is touched and reveals nothing. A refusal decided from
what a path resolves to, and the list a glob returns, both reveal that a file
of that name exists inside a folder the caller was already allowed to
search. This is accepted. Contents are never revealed.

## Scope

**In:** which files a `grounded_verify` call may read by name, what the
caller is told when a file is refused, and how a glob treats a refused file.

**Out:** which folder the tool may read (still the one configured root),
pass counts, size ceilings, the glob grammar, the confidence number, and the
prompt and output shape of the judging passes.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A file named as a secret is refused (Priority: P1)

An agent names a file exactly. If its name is refused, the whole call is
refused: no file in the call is read, nothing is sent anywhere, and the error
says which locator was refused and why.

**Why this priority**: It is the direct case, and the one the incident
exposed.

**Independent Test**: With a `.env` file and an ordinary file side by side in
the root: naming the ordinary file is read; naming `.env` is refused with no
file read and no model call.

**Acceptance Scenarios**:

1. **Given** a path matching a rule in FR-001, **When** it is named exactly, **Then** the whole call is refused before any file in the call is read and before any model is contacted.
2. **Given** a call naming an ordinary file first and a refused file second, **When** it runs, **Then** neither file is read. This holds both when the second is refused by its name as written (`.env`) and when it is refused only by what it resolves to (a file named through a link into a refused folder).
3. **Given** a refused name in different letter case (`.ENV`, `Id_Rsa`), **When** it is named, **Then** it is refused.
4. **Given** a link with an ordinary name inside the root that resolves to a refused folder also inside the root (a folder junction on Windows, a symbolic link elsewhere; for example `docs` pointing at `.ssh`), **When** a file is named through it (`docs/known_hosts`), **Then** the call is refused, although nothing in the path as written matches a rule.
5. **Given** a file inside a refused folder (`.ssh/known_hosts`), **When** it is named, **Then** it is refused whatever the file is called.
6. **Given** a folder `.ssh` that exists, and a path that enters and leaves it (`.ssh/../README.md`), **When** it is named, **Then** it is read: the path is simplified first, and what it resolves to is an ordinary file.
7. **Given** a backup form of a refused name, single or repeated, on a file or on a folder (`id_rsa.bak`, `.env~`, `secrets.yaml.orig`, `id_rsa.bak.old`, `.ssh.bak/known_hosts`), **When** it is named, **Then** it is refused.
8. **Given** a name on the must-read list in SC-003 (`id_rsa.pub`, `environment.rs`), **When** it is named, **Then** it is read.

---

### User Story 2 - A glob leaves refused files out, lists them, and says so (Priority: P1)

An agent gives a glob that matches ordinary files and one or more refused
files. The ordinary files are read and judged. The refused files are not
read. The result lists them, carries a plain notice that the verdict covers
only the files read, and sets a flag saying the evidence was reduced, so that
a verdict over less evidence than the caller asked for is never presented as
a complete one.

**Why this priority**: Without leaving files out, one stray secret-named file
would make every broad glob in that folder fail. Without the notice and the
flag, a claim about "every file in this folder" could come back supported
with the one relevant file left out and nothing to show it.

**Independent Test**: A glob matching two ordinary files and `.env` returns a
verdict over the two, a refused list naming `.env`, the notice, and the flag
set.

**Acceptance Scenarios**:

1. **Given** a glob whose matches include refused files, **When** the call runs, **Then** the refused files are not read, the others are read and judged, and the result lists the refused files by their paths from the root.
2. **Given** any result in which at least one file was left out, **When** it is returned, **Then** it carries the notice in FR-008 and the flag in FR-009 is true.
3. **Given** a glob that leaves exactly one file to read, **When** the claim is a countable one that the tool would normally settle by arithmetic over a single source, **Then** it is not settled that way. It returns what the tool returns today for a countable claim it cannot settle: the verdict inconclusive, with the reason that it is a computable property to route to `check`, and with the notice, which in this case also says how to get the claim settled.
4. **Given** a glob whose matches are all refused, **When** the call runs, **Then** the call is refused with an error saying every match was refused as a likely secret and how many there were; no model is contacted.
5. **Given** a call in which nothing was left out, **When** it returns, **Then** the refused list is present and empty, the count is zero, the notice is empty and the flag is false.
6. **Given** a glob leaving out more files than the list's limit, **When** it returns, **Then** the list holds the first files in sorted order up to the limit and the count gives the true total.
7. **Given** a glob whose text plainly starts in a refused folder (`.ssh/*`, or one branch of `{src,.ssh}/*`), **When** the call runs, **Then** it is refused from its text, before the disk is touched; the folder is not walked and its file names are not listed.

---

### User Story 3 - The rule is visible to callers and maintainers (Priority: P2)

The tool's description says secret-named files are refused and what a glob
does. The full rules and the limits are in the contract. Documents that
describe the tool as never dropping a matched file are amended.

**Why this priority**: A caller that does not know the rule reads a refusal
as a bug; a maintainer reading the older documents would be misled.

**Independent Test**: The served description contains the required text and
equals the registered one; each document in FR-012 is amended.

**Acceptance Scenarios**:

1. **Given** the tool catalog, **When** the description is read, **Then** it contains the text in FR-011.
2. **Given** the contract document, **When** read, **Then** it contains every rule in FR-001 and every limit listed above.

### Edge Cases

- **A harmless file with a refused name** (`.env.example`, a public certificate ending `.pem`, an `.npmrc` that only names a registry, an AsciiDoc file ending `.asc`, a project folder named `.docker`): refused. The list errs toward refusing and there is no override. A glob leaves such a file out and lists it; an exact path fails the call.
- **Source files caught by a rule** (`client_secret.rs`, anything under a folder named `secrets`): refused, knowingly. `secrets.rs` and `token.rs` are read, because no rule names them. The rules are the list, not the idea behind it; nobody should loosen or widen one to match the other.
- **A file named `config` anywhere under a folder whose name ends in `.git`**, such as a checkout folder named `project.git`: refused, knowingly.
- **The root itself is, or sits inside, a folder that rule 5 or rule 7 refuses**: the server refuses to start, naming the folder, because every file in it would be refused.
- **Windows alternate data streams** (`notes.txt:hidden`, `.env::$DATA`): refused under rule 8.
- **Windows short names and trailing dots or spaces**: caught only because the rules are also applied to the path the operating system resolves to. The spellings known to matter are a full path whose last part ends in a dot or a space, which Windows trims. These are tested (SC-002). Whether every such spelling resolves to the on-disk name on a case-insensitive disk on macOS, and for a root on a network share, could not be tested on the machines available and is recorded as unverified.
- **Overlapping globs** that leave out the same file: it is listed once and counted once.
- **A file that is refused only when it comes to be read** (it was replaced by a link between listing and reading): for a glob match it is left out and listed; if that leaves its glob with nothing to read, the call fails as in scenario 4. For an exact path the call fails.

## Requirements *(mandatory)*

### Functional Requirements

**How a path is examined**

- **FR-001**: A path is *refused* when any rule below holds for (a) the path as the caller gave it, after simplifying it, or (b) the whole path the operating system resolves it to, including the root's own folders. "Simplifying" is done on the text alone, without touching the disk: on Windows a leading `\\?\UNC\`, `\\?\`, `\\.\` or `\??\`, written with either slash, and then a leading drive letter with its colon, are set aside; the rest is split at the separators; empty parts and `.` parts are dropped; a `..` part removes the part before it when that part is not itself `..`, and is otherwise kept. Folder separators are `/` everywhere and also `\` on Windows. Names are compared without regard to ASCII letter case. "File name" is the last part of the path.
  1. **Exact file names**: `.env`, `.envrc`, `.netrc`, `_netrc`, `.npmrc`, `.yarnrc.yml`, `.pypirc`, `pip.conf`, `pip.ini`, `.pgpass`, `pgpass.conf`, `.my.cnf`, `.htpasswd`, `.git-credentials`, `.gitconfig`, `.modal.toml`, `.mcp.json`, `.claude.json`, `claude_desktop_config.json`, `.credentials.json`, `kaggle.json`, `.vault-token`, `.dockercfg`, `.terraformrc`, `terraform.rc`, `credentials.tfrc.json`, `kubeconfig`, `.dev.vars`, `local.settings.json`, `token.json`, `rclone.conf`, `.boto`, `.s3cfg`, `application_default_credentials.json`, `.secrets`, `secret.json`, `secret.yaml`, `secret.yml`, `secret.toml`, `secrets.json`, `secrets.yaml`, `secrets.yml`, `secrets.toml`, `credentials`, `credentials.json`, `credentials.toml`, `credentials.yaml`, `credentials.yml`, `credentials.xml`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519`, `id_ecdsa_sk`, `id_ed25519_sk`.
  2. **File name starts with**: `.env.`, `.env-`, `.env_`, `client_secret`.
  3. **File name ends with**: `.env`, `.pem`, `.key`, `.pfx`, `.p12`, `.p8`, `.ppk`, `.kdbx`, `.jks`, `.jceks`, `.keystore`, `.keytab`, `.gpg`, `.asc`, `.ovpn`, `.tfvars`, `.tfvars.json`, `.tfstate`, `.tfstate.backup`, `.kubeconfig`; or the file name contains `.tfstate.`.
  4. **Backup forms**: a name is first reduced by repeatedly removing a trailing `~`, `.bak`, `.old`, `.orig`, `.backup` or `.save` until none is left. Every part of the path is reduced this way before any of rules 1 to 3 and 5 to 7 is applied to it, so the backup of a git configuration file is refused like the file itself.
  5. **Any part of the path, whether a folder or the file itself, is named**: `.ssh`, `.aws`, `.gnupg`, `.azure`, `.kube`, `.kaggle`, `.docker`, `.envs`, `.secrets`, `secrets`.
  6. **Git configuration**: the file is named `config` or `config.worktree` and some part above it in the path has a name ending in `.git` (which includes a folder named exactly `.git`).
  7. **Two adjacent parts of the path are named** `.config` then `gcloud`, or `.config` then `gh`.
  8. **On Windows only**: once the leading forms named above are set aside, what remains contains a colon. On other systems a colon is an ordinary character and this rule does not apply.
- **FR-002**: The checks of a call MUST run in this order, and the first failure decides the error.
  - **Phase one, for every locator in the order given, without touching the disk**: (1) everything that can be checked from the locator alone: that it is a path or a glob and not both; that a start and end line are given together and form a valid range; and that a glob is well formed and names a folder to search, as today; (2) the rules of FR-001 on the simplified text: for an exact path, all of them, on the whole path; for a glob, only the rules about folders (rules 5 and 7, after rule 4's reduction), on the folder or folders it plainly starts in. The limit on the number of locators is applied exactly as today, where it is today, with the same text.
  - **Phase two, only if phase one passed for every locator, for every exact path in order**: (3) confinement to the root, as today; (4) the rules of FR-001 on the resolved path.
  - **Then globs are expanded**, each match being checked under (4) as it is listed, and the ceiling applied to the running total as today.
  - **Then files are read, in the order the locators were given**, as today. Immediately before each file is read, its name is resolved again and checked again under (3) and (4), and it must lead to the same file that was checked before. An exact path that fails any of this fails the call. A glob match that is then refused under (4) is left out under FR-006; one that then fails confinement, or no longer leads to the same file, fails the call.
  The reader checks and reads in one step today; separating them is part of this feature.
- **FR-003**: If the root's own resolved path is refused by rule 5 (with rule 4's backup forms) or by rule 7, the server MUST refuse to start, with an error naming the folder and the root.

**Exact paths (User Story 1)**

- **FR-004**: If any exact-path locator in a call is refused, the whole call MUST be refused, before any file is read and before any model is contacted, as FR-002 orders.
- **FR-005**: The refusal error MUST name the locator as the caller wrote it and say that it is, or resolves to, a file whose name marks it as a likely secret. It MUST NOT include the resolved name or any file contents. A path refused only by rule 8 MUST instead be told that a colon is not accepted in a path, and that line numbers are given as a start line and an end line, since a file name pasted with a line number after it is the likely cause.

**Globs (User Story 2)**

- **FR-006**: A refused file among a glob's matches MUST NOT be read, MUST be left out of the evidence, and MUST NOT count toward the locator ceiling.
- **FR-007**: The result MUST carry, on every call: a list of the paths (from the root) left out under this feature, sorted, without duplicates, holding at most 50; and the true count of distinct files left out. Both are empty and zero when nothing was left out.
- **FR-008**: When the count is above zero the result MUST carry this server-written notice, with N the count: `N file(s) matching a glob were not read because their names mark them as likely secrets. The verdict covers only the files that were read.` When, in addition, the claim would have been settled by arithmetic had nothing been left out, the notice MUST continue, after one space: `To have a countable claim settled, name the file to count by its exact path.` When the count is zero the notice is empty. The judging passes are not told and their prompt does not change.
- **FR-009**: The result MUST carry a true-or-false field saying whether the evidence was reduced: true exactly when the count is above zero. The confidence number MUST NOT be changed by this feature; it remains the share of completed passes that agreed.
- **FR-010**: When the count is above zero, the tool's single-source arithmetic settlement MUST NOT be used. A countable claim then takes the tool's existing exit for a claim it cannot settle: the verdict inconclusive, with its existing reason. (The judging passes have already run by this point and have answered that the claim needs computing; there is no second judgment to fall back on.) If every match of any one glob is refused, at expansion or by the time of reading, the call MUST be refused with an error naming that glob and giving the number refused, and no model is contacted.

**Visibility (User Story 3)**

- **FR-011**: The tool description, in both places it is written, MUST contain exactly: `Files whose names mark them as secrets (for example .env, key files and credential files) are refused. An exact path to one fails the call. A glob leaves them out, lists them under refused and sets evidence_reduced; if every match is refused the call fails.` A test MUST hold the served description equal to the registered one and check for this text.
- **FR-012**: These MUST be amended in the same change, because each states something this feature makes untrue or incomplete:
  1. `docs/design/NEW_SERVER_DESIGN.md`: a dated amendment recording that grounded-verify gains a fixed name-based refusal inside its root, with one named exception to all-or-nothing, and that it is a guard and not a boundary.
  2. `specs/008-grounded-verify/spec.md` and its contract document: the all-or-nothing rule gains its one exception; the error table gains the refusals; the result gains the list, count, flag and notice.
  3. `specs/009-glob-locators/spec.md` and its contract document: "0% of matched files silently dropped" becomes "no matched file is dropped silently; a refused file is left out, listed and counted"; "all-or-nothing" and "output unchanged from 008" are corrected; the zero-match and ceiling rules state how refused files count; and its requirement that the exact-path code and its tests are unchanged is marked as superseded, since the reader is split.
  4. `specs/011-grounded-compute-settle`: its specification, contract and data model state that settlement is not used when any file was left out.
  5. The published result contract file for the tool.
  6. `SECURITY.md`; the README's row for the tool and its description of the root; the tool's line in the program's help text; the server's instructions text; the project guide's description of the tool; the header of the tool's acceptance example; and the change log.
  7. The opening documentation of the source files whose described behaviour changes: the reader seam, the grounded module, the assembly stage and the tool's run function.
- **FR-013**: The contract document MUST contain every rule in FR-001, the limits in "What this protects against, and what it does not", what a refusal reveals, the startup refusal, and the order of checks.

**Unchanged**

- **FR-014**: For a call in which nothing is refused or left out, reading, confinement, ceilings, the glob grammar, the order of evidence, judging, settlement and confidence MUST be exactly as before, and the result MUST differ only by the presence of the empty list, the zero count, the false flag and the empty notice. One thing may differ and is accepted: when a call has more than one fault, which of them is reported, because checks that used to run as each file was read now run first.
- **FR-015**: No setting is added and no rule is configurable. The prompt and the output shape of the judging passes do not change.

### Key Entities

- **Refused path**: a path matching any rule in FR-001.
- **Refused list, count, flag and notice**: the four additions to the result.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every name, prefix, ending, backup form (single and repeated, on files and on folders), part name and path sequence in rules 1 to 7 of FR-001 has a test, in lower, upper and mixed case, including rules 6 and 7 with the root itself as the first folder of the pair, showing that the rule refuses it; and for one entry of each kind of rule, a call through the tool showing no file read and no model call. Three bindings keep the lists honest: the tests are driven from the tables the code uses; a test holds those tables equal to the list in the contract document, in both directions, and fails on any line of that list it cannot classify; and a short hand-written list of at least twenty refusals, typed separately from the tables and covering every rule, is tested as well, so that a wrong table is caught by something that is not the table.
- **SC-002**: Four behaviours are tested on a real Windows disk, through the same entry point a call uses: an alternate data stream on a refused and on an ordinary name, where the test first writes the stream and reads it back directly, to prove a stream exists, and then shows the call refused; a path given in the `\\?\C:\` form, with each slash direction, to an ordinary file inside the root, which must be read; a full path to a refused file with a trailing dot and with a trailing space, which must be refused; and a folder junction with an ordinary name whose target is a refused folder inside the root, with a file named through it, so that only the resolved-path check can refuse it. On other systems the link case is tested with a symbolic link to a refused folder and with a symbolic link to a refused file, and a file whose name contains a colon is shown to be read. A test that cannot create its link or stream MUST fail with a message saying so; it MUST NOT skip.
- **SC-003**: Each of these names is shown to be read, not refused: `id_rsa.pub`, `id_ed25519.pub`, `environment.rs`, `env.rs`, `keys.rs`, `token.rs`, `secrets.rs`, `config.toml`, `.gitignore`, `README.md`, and `ci.yml` inside `.github/workflows`.
- **SC-004**: A glob over a folder holding two ordinary files and one refused file yields a verdict over two files, a list of exactly one path, a count of one, the notice, the flag true, and the same confidence the same passes give over those two files with nothing left out; a reader that records every check and every read shows the refused file was never opened.
- **SC-005**: Two calls each read nothing (shown the same way): an ordinary file then `.env`; and an ordinary file then a file named through a link into a refused folder. The second cannot pass unless every file is checked before any is read.
- **SC-006**: A countable claim over a glob that leaves one file to read and one left out returns inconclusive with the notice and its closing sentence, and carries no settled form; the same claim over the same single file with nothing left out settles as before.
- **SC-007**: Every existing test of the tool keeps proving what it proved. Tests whose calls change shape because the reader is split are listed in the task list one by one, with what each asserted before and after; a test that asserted "nothing was touched" asserts it of both the check and the read. Tests that compare the whole result shape are updated for the four additions. No assertion about reading, confinement, ceilings, judging, settlement or confidence is weakened.
- **SC-008**: Each item in FR-012 is amended, checked off in the task list.

## Assumptions

- The root stays set to the Parallax repository, as the owner chose on 2026-11-17, until the later session-folder work is specified, built and deployed. This feature does not make a wide root safe.
- Delivered as one pull request.
- All development and automated tests use a substituted model and temporary folders. The project keeps a note that any model-facing prompt or schema change must pass the live example before a pull request. The tool's description and published result shape change here, and both are read by calling models. Whether a live call is made before the pull request, and what it would cost, is asked of the owner on its own.
- **The project's automated checks on pull requests run on Linux only.** The Windows tests in SC-002 therefore run only on a Windows machine, through the local gate. This is a known gap in what the pull-request checks prove; whether to add a Windows check is asked of the owner, and the pull request says which tests ran where.
- No override is offered. If a refused harmless file must be verified, the owner copies it to an ordinary name. That is the same act that defeats the guard, and it is the owner's to make.

## Departure from an existing rule, named

Since 008 the tool has been all-or-nothing: a call either reads everything it
named or fails. This feature makes one exception, for globs only: a refused
file is left out. The reason is that a single secret-named file would
otherwise make every broad glob in its folder fail. Three things keep the
spirit of the rule: nothing is left out silently (the list, the count, the
flag and the notice are always present); a countable claim is never settled
over reduced evidence; and an exact path to a refused file still fails the
call.

The owner considered lowering the confidence number as well and set it
aside: the number is the share of passes that agreed, a rule that changed it
only when all passes agreed would not do what was intended, and one that
always changed it would stop it meaning what every document says it means.
The flag carries the fact instead.

## Deferred (named, not built here)

- **Confining each session to its own project.** A separate specification. It must begin with a test against the real client program to establish how the server can learn the folder a session is working in. Review of this specification's first version fixed three requirements for it in advance: the folder must come from the client program and never from anything the calling agent writes; there must be no default mode, so that setting the root to a folder of projects without choosing a mode stops the server at startup; and "the session's folder" must be defined against the cases found in review (several folders, a folder that changes, links, a session started above several projects).
- Detecting secrets by content.
- Refusing files that have more than one hard link.
- Closing the instant between a file's last check and its being opened, by checking the file once it is open. Named as part of the session-folder specification, where confinement between projects is the goal and a race matters; it needs a new dependency, which that work must justify.
- Matching by word (any name containing "secret" or "token"), which the owner considered and set aside in favour of a fixed list with backup forms.
- An operator-editable list, or an override.
- Recording refusals in invocation records.
- More than one root.
