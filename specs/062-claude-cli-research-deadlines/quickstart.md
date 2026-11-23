# Quickstart: Research deadlines that fit claude_cli

## Check it

With `PARALLAX_BACKEND=claude_cli`, run `research` at standard and deep depth
with no `constraints.deadline_ms`. Each should finish with `stopped_early`
false.

## Results (2026-11-23, concurrency 4)

| Run | Deadline | Time | Stopped early | Claims verified |
|---|---|---|---|---|
| Standard, first values | 360 s | 394 s | yes (deadline) | 133 of 177 |
| Standard, measured (caller 900 s) | 900 s | 538 s | no | 180 of 180 |
| Deep, measured (caller 1800 s) | 1800 s | 1632 s | no | 400 of 400 |
| Standard, default (SC-001) | 660 s | 517 s | no | 180 of 180 |
| Deep, default (SC-002) | 1980 s | 1594 s | no | 400 of 400 |

- SC-001 and SC-002 pass.
- The first values (360 / 720 s) failed because they were estimated from runs
  that had themselves been cut off at their deadline.
- Deep runs used 4.3–4.4 M of the 5.5 M token budget; a larger deep question
  may reach the budget before the deadline.
