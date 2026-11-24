# 063 quickstart: live check

Two runs on 2026-11-23 on `PARALLAX_BACKEND=claude_cli`, concurrency 4, Exa and Voyage keys set: `cargo run --release --example acceptance_research`.

- Run 1 (at `4ccbe1f`): 0 of 85 pages flagged. Pages read 5 / 5 / 5 / 15 / 15 / 35 / 5 = 85, the same as the 061 run; 7/7 grounded, as in 061. It printed ACCEPTANCE: FAIL only because the example's standard target was still 240 s (standard max 563 s).
- The example's standard target on claude_cli now comes from 062's deadline (660 s); other backends keep 240 s.
- Run 2 (with that change): ACCEPTANCE: PASS. 85 pages, 7/7 grounded; quick max 125 s, standard max 506 s, deep 1604 s; the tiny budget stopped early (budget); the false premise was not confirmed.
- Run 2 flagged 1 page, checked by hand: s15 `hidden_text`, <https://sparkles-docs.pages.dev/research/async-io/tokio>. A false positive: its 24 hidden characters are all U+200B, and each of the 15 inspected is the only text of a heading's anchor link, not prose. The screen only warns, so it changed no finding or extraction (SC-002).
- Not shown live: a true positive. Flagging is covered by the tests in src/research/screen.rs and src/research/pipeline_tests.rs.
