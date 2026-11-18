# Quickstart: checking this feature by hand

Nothing here calls a model.

## 1. The rules

```bash
cargo test secrets::
```

Every name, prefix, ending, backup form, part name and path sequence is
tested in three letter cases, for both platforms, straight from the tables
the code uses. The must-read names (`id_rsa.pub`, `environment.rs`, and the
rest of SC-003) are shown to pass.

## 2. The reader and assembly

```bash
cargo test grounded::
```

- An ordinary file named before a refused one: neither is read.
- A glob over two ordinary files and `.env`: two read, one listed.
- A link with an ordinary name into a refused folder: refused.

On Windows this also runs the four real-filesystem tests (an alternate data
stream, a `\\?\C:\` path, a trailing dot or space, a folder junction). On other systems the link test
uses a symbolic link.

## 3. The result

```bash
cargo test grounded_verify
```

With something left out: the list, the count, the flag set, the notice, the
same confidence the passes give over the files read, and a countable claim
returns inconclusive with the note saying how to get it settled. With nothing
left out: empty list, zero, flag false, empty notice.

## 4. The contract matches the code

```bash
cargo test the_contract_lists_exactly_the_rules_the_code_uses
```

## 5. Everything

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test
```

Then the markdown check on the documents this feature wrote or amended.
