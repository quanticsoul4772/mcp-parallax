# Contributing

## Ground rules

- The design corpus in `docs/design/` is the source of truth for *what* gets
  built; the constitution (`.specify/memory/constitution.md`) governs *how*.
  Changes to the tool surface, layers, schemas, trait seams, or dependency
  stack need to trace to the corpus — deviations are named in the PR, never
  slipped in.
- Features flow through the Spec Kit sequence (see `specs/001`–`015` for the
  shape): spec → clarify → plan → tasks → analyze → implement. Small fixes
  don't need the ceremony; new capabilities do.

## The quality gate

Every commit must pass the same gate CI enforces:

```bash
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test
```

`pre-commit install` mirrors it locally. Tests are required, written through
the trait seams (`ModelClient`, `Storage`, `Embedder`, …) so the suite runs
without network, keys, or disk. MSRV is pinned in `Cargo.toml` and verified
by CI.

## Build notes

The bundled `z3` needs cmake; the first clean build takes ~5 minutes. On
Windows, point `CMAKE` at the VS Build Tools cmake.

## PRs

Feature branches only; keep commits incremental with meaningful messages.
PRs that ship user-visible behavior also append to `## [Unreleased]` in
`CHANGELOG.md` (Keep a Changelog 1.1.0).
