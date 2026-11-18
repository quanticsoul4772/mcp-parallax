# Contract: secret-named files in `grounded_verify`

## What a caller sees

- **An exact path to a refused file** fails the whole call, before any file
  is read and before any model is contacted:
  `[invalid_input] invalid input: locator refused: <path as written> - it is, or resolves to, a file whose name marks it as a likely secret`
- **A glob that plainly starts in a refused folder** (`.ssh/*`) fails the
  call from its text, before the disk is touched:
  `[invalid_input] invalid input: glob refused: <pattern> - it searches a folder whose name marks its files as likely secrets`
- **On Windows, a path with a colon in it** (after any drive letter) fails
  the call:
  `[invalid_input] invalid input: locator refused: <path as written> - a colon is not accepted in a path; give line numbers as start_line and end_line`
- **Any other glob** leaves refused files out. The result then has:
  - `refused`: the paths left out, from the root, sorted, at most 50;
  - `refused_count`: how many distinct files in all;
  - `evidence_reduced`: `true`;
  - `evidence_note`: `N file(s) matching a glob were not read because their names mark them as likely secrets. The verdict covers only the files that were read.`
  - no arithmetic settlement: a countable claim returns `inconclusive`; when it would
    otherwise have been settled, the note ends, after a space, with `To have a countable claim settled, name the file to count by its exact path.`
- **A glob whose every match is refused** fails the call:
  `[invalid_input] invalid input: glob matched only files refused as likely secrets (<n>): <pattern>`
- **When nothing was left out**, `refused` is empty, `refused_count` is 0,
  `evidence_reduced` is `false`, `evidence_note` is empty, and everything
  else is as before.
- **`confidence` is never changed by any of this.** It remains the share of
  completed passes that agreed. Read `evidence_reduced` to know whether the
  verdict covers everything the glob matched.

## The rules

A path is refused when any rule holds for the path as written (after
simplifying it on the text alone) or for the whole path it resolves to.
Letter case is ignored. On Windows both slash directions separate folders.

The block below is read by a test and compared with the tables in the code.
Do not reformat it.

```text
exact .env
exact .envrc
exact .netrc
exact _netrc
exact .npmrc
exact .yarnrc.yml
exact .pypirc
exact pip.conf
exact pip.ini
exact .pgpass
exact pgpass.conf
exact .my.cnf
exact .htpasswd
exact .git-credentials
exact .gitconfig
exact .modal.toml
exact .mcp.json
exact .claude.json
exact claude_desktop_config.json
exact .credentials.json
exact kaggle.json
exact .vault-token
exact .dockercfg
exact .terraformrc
exact terraform.rc
exact credentials.tfrc.json
exact kubeconfig
exact .dev.vars
exact local.settings.json
exact token.json
exact rclone.conf
exact .boto
exact .s3cfg
exact application_default_credentials.json
exact .secrets
exact secret.json
exact secret.yaml
exact secret.yml
exact secret.toml
exact secrets.json
exact secrets.yaml
exact secrets.yml
exact secrets.toml
exact credentials
exact credentials.json
exact credentials.toml
exact credentials.yaml
exact credentials.yml
exact credentials.xml
exact id_rsa
exact id_dsa
exact id_ecdsa
exact id_ed25519
exact id_ecdsa_sk
exact id_ed25519_sk
prefix .env.
prefix .env-
prefix .env_
prefix client_secret
ending .env
ending .pem
ending .key
ending .pfx
ending .p12
ending .p8
ending .ppk
ending .kdbx
ending .jks
ending .jceks
ending .keystore
ending .keytab
ending .gpg
ending .asc
ending .ovpn
ending .tfvars
ending .tfvars.json
ending .tfstate
ending .tfstate.backup
ending .kubeconfig
contains .tfstate.
backup ~
backup .bak
backup .old
backup .orig
backup .backup
backup .save
part .ssh
part .aws
part .gnupg
part .azure
part .kube
part .kaggle
part .docker
part .envs
part .secrets
part secrets
gitconfig config
gitconfig config.worktree
sequence .config gcloud
sequence .config gh
```

Reading the block:

- `exact`, `prefix`, `ending`, `contains`: tests on the file name.
- `backup`: these endings are removed, repeatedly, from the end of every part
  of the path before that part is tested. So `id_rsa.bak.old` is tested as
  `id_rsa`, and the folder `.ssh.bak` as `.ssh`.
- `part`: any part of the path, folder or file, with that name.
- `gitconfig`: a file with that name anywhere under a folder whose name ends
  in `.git` (a folder named exactly `.git` included).
- `sequence`: two adjacent parts of the path with those names, in that order.

One rule has no line in the block, and is bound to the code by the separately
typed test list instead: on Windows, once a leading `\\?\UNC\`, `\\?\`,
`\\.\` or `\??\` (written with either slash) and then a drive letter with
its colon are set aside, any colon in what remains.

## How a path is simplified before the rules are applied to it as written

On the text alone, without touching the disk: the leading forms above are set
aside on Windows; the rest is split at `/` (and at `\` on Windows); empty
parts and `.` parts are dropped; a `..` part removes the part before it when
that part is not itself `..`, and is otherwise kept.

## Order of checks

1. Without touching the disk, for every locator in order: its shape,
   including that start and end lines come together and form a valid range; a glob's
   text; then the rules on the text (all of them on the whole path for an
   exact path; only the folder rules on the starting folder or folders of a
   glob).
2. For every locator in order: an exact path counts as one; a glob is
   expanded, its refused matches set aside and the rest counted. After each
   locator the running total is compared with the limit on the number of
   locators, exactly as before. A glob that matches nothing, or only refused
   files, fails the call.
3. For every exact path in order: confinement to the root, then the rules on
   the whole resolved path.
4. Files are read in the order the locators were given. Each name is resolved and
   checked again immediately before it is read, and must lead to the same
   file.

The first failure decides the error. Compared with before this feature, the
error reported can differ when a call has more than one fault.

## At startup

If the root is, or is inside, a folder the `part` or `sequence` rules refuse,
the server does not start:
`GROUNDED_VERIFY_ROOT is, or is inside, <name>, whose files are refused as likely secrets: <root>`

## What this does not protect against

- A secret kept in a file with an ordinary name.
- A secret-looking name that is not on the list.
- An agent that renames or copies a secret file to an ordinary name first.
- A hard link: a second ordinary name for the same file.
- A swap made in the instant between a file's last check and its being
  opened.
- Anything an agent's own file-reading tool can do.

It is a guard against accident and simple misdirection, not a boundary.

## What a refusal reveals

A refusal decided from the path as written is given before the disk is
touched and reveals nothing. A refusal decided from what a path resolves to,
and the list a glob returns, reveal that a file of that name exists inside
the folder the caller was allowed to search. Contents are never revealed.

## Refused knowingly

Templates such as `.env.example`; public certificates ending `.pem`; an
`.npmrc` that only names a registry; AsciiDoc files ending `.asc`; a project
folder named `.docker` or `secrets`; source files such as `client_secret.rs`; a file named `config` anywhere under
a folder whose name ends in `.git`. There is no override.

## Not verified

How a case-insensitive disk on macOS and a root on a network share resolve
unusual spellings of a refused name. Neither could be tested on the machines
available.
