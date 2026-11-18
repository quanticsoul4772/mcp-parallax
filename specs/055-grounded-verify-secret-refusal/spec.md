# Feature Specification: grounded_verify Refuses Files Whose Names Mark Them as Secrets

**Feature Branch**: `055-grounded-verify-secret-refusal`

**Created**: 2026-11-17

**Status**: Draft (fourth version; narrowed after review of the first, corrected after review of the second and of the third's rule text)

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
- A swap made between the check and the read: another process replacing a
  folder in the path with a link in that moment. The tool's folder
  confinement has the same gap today. Closing it is named in Deferred.
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
pass counts, size ceilings, the glob grammar, and the prompt and output shape
of the judging passes.

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
2. **Given** a call naming an ordinary file first and a refused file second, **When** it runs, **Then** neither file is read.
3. **Given** a refused name in different letter case (`.ENV`, `Id_Rsa`), **When** it is named, **Then** it is refused.
4. **Given** a link with an ordinary name inside the root that resolves to a refused folder also inside the root (a folder junction on Windows, a symbolic link elsewhere; for example `docs` pointing at `.ssh`), **When** a file is named through it (`docs/known_hosts`), **Then** the call is refused, although nothing in the path as written matches a rule.
5. **Given** a file inside a refused folder (`.ssh/known_hosts`), **When** it is named, **Then** it is refused whatever the file is called.
6. **Given** a folder `.ssh` that exists, and a path that enters and leaves it (`.ssh/../README.md`), **When** it is named, **Then** it is read: the path is simplified first, and what it resolves to is an ordinary file.
7. **Given** a backup form of a refused name, single or repeated (`id_rsa.bak`, `.env~`, `secrets.yaml.orig`, `id_rsa.bak.old`), **When** it is named, **Then** it is refused.
8. **Given** a name on the must-read list in SC-003 (`id_rsa.pub`, `environment.rs`), **When** it is named, **Then** it is read.

---

### User Story 2 - A glob leaves refused files out, lists them, and says so (Priority: P1)

An agent gives a glob that matches ordinary files and one or more refused
files. The ordinary files are read and judged. The refused files are not
read. The result lists them, carries a plain notice that the verdict covers
only the files read, and reports a reduced confidence, so that a verdict over
less evidence than the caller asked for is never presented as a full one.

**Why this priority**: Without leaving files out, one stray secret-named file
would make every broad glob in that folder fail. Without the notice and the
reduced confidence, a claim about "every file in this folder" could come back
supported at full confidence with the one relevant file left out.

**Independent Test**: A glob matching two ordinary files and `.env` returns a
verdict over the two, a refused list naming `.env`, the notice, and a
confidence below the value the same passes would otherwise give.

**Acceptance Scenarios**:

1. **Given** a glob whose matches include refused files, **When** the call runs, **Then** the refused files are not read, the others are read and judged, and the result lists the refused files by their paths from the root.
2. **Given** any result in which at least one file was left out, **When** it is returned, **Then** it carries the notice in FR-008 and its confidence is capped as FR-009 says.
3. **Given** a glob that leaves exactly one file to read, **When** the claim is a countable one that the tool would normally settle by arithmetic over a single source, **Then** it is not settled that way; it is judged by the ordinary passes, with the notice and the cap.
4. **Given** a glob whose matches are all refused, **When** the call runs, **Then** the call is refused with an error saying every match was refused as a likely secret and how many there were; no model is contacted.
5. **Given** a call in which nothing was left out, **When** it returns, **Then** the refused list is present and empty, the count is zero, the notice is empty, and confidence is exactly what it would have been before this feature.
6. **Given** a glob leaving out more files than the list's limit, **When** it returns, **Then** the list holds the first files in sorted order up to the limit and the count gives the true total.

---

### User Story 3 - The rule is visible to callers and maintainers (Priority: P2)

The tool's description says secret-named files are refused and what a glob
does. The full rules and the limits are in the contract. Documents that
describe the tool as never dropping a matched file are amended.

**Why this priority**: A caller that does not know the rule reads a refusal
as a bug; a maintainer reading the older documents would be misled.

**Independent Test**: The served description contains the required sentences
and equals the registered one; each document in FR-012 is amended.

**Acceptance Scenarios**:

1. **Given** the tool catalog, **When** the description is read, **Then** it contains the text in FR-011.
2. **Given** the contract document, **When** read, **Then** it contains every rule in FR-001 and every limit listed above.

### Edge Cases

- **A harmless file with a refused name** (`.env.example`, a public certificate ending `.pem`, an `.npmrc` that only names a registry, an AsciiDoc file ending `.asc`, a project folder named `.docker`): refused. The list errs toward refusing and there is no override. A glob leaves such a file out and lists it; an exact path fails the call.
- **Source files caught by a rule** (`client_secret.rs`, anything under a folder named `secrets`): refused, knowingly. `secrets.rs` and `token.rs` are read, because no rule names them. The rules are the list, not the idea behind it; nobody should loosen or widen one to match the other.
- **The root itself sits inside a folder named in rule 5**, or is named like one: the server refuses to start, naming the folder, because every file in it would be refused.
- **Windows alternate data streams** (`notes.txt:hidden`, `.env::$DATA`): refused under rule 8.
- **Windows short names and trailing dots or spaces**: caught only because the rules are also applied to the path the operating system resolves to. Whether every such spelling resolves to the on-disk name is to be confirmed by test in planning, per platform; any spelling found not to is added to the limits. The same confirmation is needed for a case-insensitive disk on macOS and for a root on a network share.
- **Overlapping globs** that leave out the same file: it is listed once.

## Requirements *(mandatory)*

### Functional Requirements

**How a path is examined**

- **FR-001**: A path is *refused* when any rule below holds for (a) the path as the caller gave it, after simplifying it, or (b) the whole path the operating system resolves it to, including the root's own folders. "Simplifying" is done on the text alone, without touching the disk: on Windows a leading `\\?\UNC\`, `\\?\`, `\\.\` or `\??\`, written with either slash, and then a leading drive letter with its colon, are set aside; the rest is split at the separators; empty parts and `.` parts are dropped; a `..` part removes the part before it when that part is not itself `..`, and is otherwise kept. Folder separators are `/` everywhere and also `\` on Windows. Names are compared without regard to ASCII letter case. "File name" is the last part of the path.
  1. **Exact file names**: `.env`, `.envrc`, `.netrc`, `_netrc`, `.npmrc`, `.yarnrc.yml`, `.pypirc`, `pip.conf`, `pip.ini`, `.pgpass`, `pgpass.conf`, `.my.cnf`, `.htpasswd`, `.git-credentials`, `.gitconfig`, `.modal.toml`, `.mcp.json`, `.claude.json`, `claude_desktop_config.json`, `.credentials.json`, `kaggle.json`, `.vault-token`, `.dockercfg`, `.terraformrc`, `terraform.rc`, `credentials.tfrc.json`, `kubeconfig`, `.dev.vars`, `local.settings.json`, `token.json`, `rclone.conf`, `.boto`, `.s3cfg`, `application_default_credentials.json`, `.secrets`, `secret.json`, `secret.yaml`, `secret.yml`, `secret.toml`, `secrets.json`, `secrets.yaml`, `secrets.yml`, `secrets.toml`, `credentials`, `credentials.json`, `credentials.toml`, `credentials.yaml`, `credentials.yml`, `credentials.xml`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519`, `id_ecdsa_sk`, `id_ed25519_sk`.
  2. **File name starts with**: `.env.`, `.env-`, `.env_`, `client_secret`.
  3. **File name ends with**: `.env`, `.pem`, `.key`, `.pfx`, `.p12`, `.p8`, `.ppk`, `.kdbx`, `.jks`, `.jceks`, `.keystore`, `.keytab`, `.gpg`, `.asc`, `.ovpn`, `.tfvars`, `.tfvars.json`, `.tfstate`, `.tfstate.backup`, `.kubeconfig`; or the file name contains `.tfstate.`.
  4. **Backup forms**: the file name, after repeatedly removing a trailing `~`, `.bak`, `.old`, `.orig`, `.backup` or `.save` until none is left, matches rule 1, 2 or 3.
  5. **Any part of the path, whether a folder or the file itself, is named**: `.ssh`, `.aws`, `.gnupg`, `.azure`, `.kube`, `.kaggle`, `.docker`, `.envs`, `.secrets`, `secrets`.
  6. **Git configuration**: the file is named `config` or `config.worktree` and some folder above it in the path is named `.git` or has a name ending in `.git`.
  7. **Path contains the folder sequence** `.config/gcloud` or `.config/gh`.
  8. **On Windows only**: once the leading forms named above are set aside, what remains contains a colon. On other systems a colon is an ordinary character and this rule does not apply.
- **FR-002**: For the exact-path locators of a call the checks MUST run in two phases. **Phase one**, for every locator in the order given, without touching the disk: (1) everything about the locator that can be checked without reading, namely that it is a path or a glob and not both, that a start and end line are given together, and that the range is valid; (2) the rules of FR-001 on the simplified path as given. **Phase two**, only if phase one passed for every locator, again in order: (3) confinement to the root, as today; (4) the rules of FR-001 on the resolved path. The first failure, in that order, decides the error. Only when both phases have passed for every locator is any file read, and the file read MUST be the resolved path that was checked, not a second resolution of the name. The reader checks and reads in one step today; separating them is part of this feature. Glob locators are expanded after phase two; each match is checked under (4) when it is listed and again when it is read, and a match that fails either check is left out under FR-006.
- **FR-003**: If any folder in the root's own resolved path is named in rule 5, the server MUST refuse to start, with an error naming the folder, since every file would be refused.

**Exact paths (User Story 1)**

- **FR-004**: If any exact-path locator in a call is refused, the whole call MUST be refused, before any file is read and before any model is contacted, as FR-002 orders.
- **FR-005**: The refusal error MUST name the locator as the caller wrote it and say that it is, or resolves to, a file whose name marks it as a likely secret. It MUST NOT include the resolved name or any file contents.

**Globs (User Story 2)**

- **FR-006**: A refused file among a glob's matches MUST NOT be read, MUST be left out of the evidence, and MUST NOT count toward the locator ceiling.
- **FR-007**: The result MUST carry, on every call: a list of the paths (from the root) left out under this feature, sorted, without duplicates, holding at most 50; and the true count of files left out. Both are empty and zero when nothing was left out.
- **FR-008**: When the count is above zero the result MUST carry this server-written notice, with N the count: `N file(s) matching a glob were not read because their names mark them as likely secrets. The verdict covers only the files that were read.` When the count is zero the notice is empty. The judging passes are not told and their prompt does not change.
- **FR-009**: When the count is above zero, the reported confidence MUST be the lower of the value the passes gave and `k / (k + 1)`, where k is the number of passes used: the value it would have had if one more pass had disagreed. When the count is zero, confidence MUST be unchanged.
- **FR-010**: When the count is above zero, the tool's single-source arithmetic settlement MUST NOT be used; the claim goes to the ordinary judging passes. If every match of a glob is refused, the call MUST be refused with an error giving the number refused, and no model is contacted.

**Visibility (User Story 3)**

- **FR-011**: The tool description, in both places it is written, MUST contain exactly: `Files whose names mark them as secrets (for example .env, key files and credential files) are refused. An exact path to one fails the call. A glob leaves them out, lists them under refused, and lowers the confidence; if every match is refused the call fails.` A test MUST hold the served description equal to the registered one and check for this text.
- **FR-012**: These documents MUST be amended in the same change, because each states something this feature makes untrue or incomplete:
  1. `specs/008-grounded-verify/spec.md` and its contract: the all-or-nothing rule gains one named exception; the error table gains the refusals; the result gains the list, count and notice.
  2. `specs/009-glob-locators/spec.md` and its contract: "0% of matched files silently dropped" becomes "no matched file is dropped silently; a refused file is left out, listed and counted"; the zero-match and ceiling rules state how refused files count.
  3. `specs/011` (the arithmetic settlement): it is not used when any file was left out.
  4. The published result contract file for the tool.
  5. `SECURITY.md`, the README's row for the tool, and the change log.
- **FR-013**: The contract document MUST contain every rule in FR-001, the limits in "What this protects against, and what it does not", and what a refusal reveals.

**Unchanged**

- **FR-014**: For a call in which nothing is refused or left out, reading, confinement, ceilings, the glob grammar, evidence assembly, judging, settlement and confidence MUST be exactly as before, and the result MUST differ only by the presence of the empty list, the zero count and the empty notice.
- **FR-015**: No setting is added and no rule is configurable. The prompt and the output shape of the judging passes do not change.

### Key Entities

- **Refused path**: a path matching any rule in FR-001.
- **Refused list, count and notice**: the three additions to the result.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every name, prefix, ending, backup form (single and repeated), part name and path sequence in rules 1 to 7 of FR-001 has a test, including rules 6 and 7 with the root itself as the first folder of the pair, in lower, upper and mixed case, showing a refusal with no file read and no model call. The tests are driven from the same list the code uses, and a separate test holds that list equal to the list in the contract document, so a rule cannot be added to one and not the other.
- **SC-002**: Rule 8 and the resolved-path arm are tested on Windows with: an alternate data stream on a refused and on an ordinary name; a path given in the `\\?\C:\` form, with each slash direction, to an ordinary file inside the root, which must be read; and a folder junction with an ordinary name whose target is a refused folder inside the root, with a file named through it, so that only the resolved-path check can refuse it. On other systems the same is tested with a symbolic link with an ordinary name pointing at a refused folder, and a file whose name contains a colon is shown to be read. A test that cannot create its link or stream MUST fail with a message saying so; it MUST NOT skip.
- **SC-003**: Each of these names is shown to be read, not refused: `id_rsa.pub`, `id_ed25519.pub`, `environment.rs`, `env.rs`, `keys.rs`, `token.rs`, `secrets.rs`, `config.toml`, `.gitignore`, `README.md`, and `ci.yml` inside `.github/workflows`.
- **SC-004**: A glob over a folder holding two ordinary files and one refused file yields a verdict over two files, a list of exactly one path, a count of one, the notice, and a confidence no higher than `k / (k + 1)`; a reader that records every read shows the refused file was never opened.
- **SC-005**: A call naming an ordinary file and then a refused file reads neither (shown the same way).
- **SC-006**: A countable claim over a glob that leaves one file to read and one left out is shown to go to the judging passes and not to arithmetic settlement.
- **SC-007**: Every existing test of the tool passes with no edit to an assertion about reading, confinement, ceilings, judging, settlement or confidence. Tests that compare the whole result shape are updated for the three additions, and each such edit is named in the task list.
- **SC-008**: Each document in FR-012 is amended, checked off in the task list.

## Assumptions

- The root stays set to the Parallax repository, as the owner chose on 2026-11-17, until the later session-folder work is specified, built and deployed. This feature does not make a wide root safe.
- Delivered as one pull request.
- All development and automated tests use a substituted model and temporary folders. No live model call is needed to build or test. A live check after merge, if wanted, is asked of the owner with its price.
- **The project's automated checks on pull requests run on Linux only.** The Windows tests in SC-002 therefore run only on a Windows machine, through the local gate. This is a known gap in what the pull-request checks prove; planning decides whether to add a Windows check, and the pull request says which tests ran where.
- No override is offered. If a refused harmless file must be verified, the owner copies it to an ordinary name. That is the same act that defeats the guard, and it is the owner's to make.

## Departure from an existing rule, named

Since 008 the tool has been all-or-nothing: a call either reads everything it
named or fails. This feature makes one exception, for globs only: a refused
file is left out. The reason is that a single secret-named file would
otherwise make every broad glob in its folder fail. Three things keep the
spirit of the rule: nothing is left out silently (the list, the count and the
notice are always present); the confidence is lowered whenever anything was
left out; and an exact path to a refused file still fails the call.

## Deferred (named, not built here)

- **Confining each session to its own project.** A separate specification. It must begin with a test against the real client program to establish how the server can learn the folder a session is working in. Review of this specification's first version fixed three requirements for it in advance: the folder must come from the client program and never from anything the calling agent writes; there must be no default mode, so that setting the root to a folder of projects without choosing a mode stops the server at startup; and "the session's folder" must be defined against the cases found in review (several folders, a folder that changes, links, a session started above several projects).
- Detecting secrets by content.
- Refusing files that have more than one hard link.
- Closing the gap between check and read, by checking the file once it is open. Named as part of the session-folder specification, where confinement between projects is the goal and a race matters; it needs a new dependency, which that work must justify.
- Matching by word (any name containing "secret" or "token"), which the owner considered and set aside in favour of a fixed list with backup forms.
- An operator-editable list, or an override.
- Recording refusals in invocation records.
- More than one root.
