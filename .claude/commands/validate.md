---
description: Run the full local quality gate (fmt, clippy, test) — same checks CI enforces.
---

Run the project's full validation gate and report the result of each step. Stop and
surface the first failure with its output; do not "fix" unrelated code to make it pass.

```bash
cargo fmt --all -- --check
cargo clippy --all-features -- -D warnings
cargo test --all-features
```

If all three pass, state that the gate is green. If any fail, show the failing
output and the root cause — do not minimize it.
