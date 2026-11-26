# Quickstart: Jev injection warning on research sources

## 1. Build and test

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests
```

## 2. Shadow, then on

1. Set `PARALLAX_JEV_MODE_RESEARCH_INJECTION=shadow` with `TYPESAFE_API_KEY` present; run one research question; the result is unchanged and one `research_injection` row per kept page is written.
2. Set it to `on`; run the same question; each source shows `jev_injection`, and stats show `jev_warned_pages` and `jev_unscreened_pages`.

## 3. Live acceptance (SC-005)

```bash
cargo run --release --example acceptance_jev_injection
```

It exits non-zero unless every kept page has one joined row, answered rows carry `jev-1.14.0`, and every kept page shows screened or a reason.

## 4. Report from stored records (FR-009)

```bash
cargo run --release --example jev_backtest -- report
```

The injection site's table shows the share warned, failures by outcome, and agreement with 063's pattern screen on the same pages.

## 5. Turn it on in the operator's setup (FR-010)

After merge and rebuild, with a backup of `~/.claude.json` taken: add `PARALLAX_JEV_MODE_RESEARCH_INJECTION=on` to the Parallax `env` block, changing only that key; restart Claude Code.
