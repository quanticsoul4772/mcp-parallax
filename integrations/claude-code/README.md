# Parallax checkpoint layer — Claude Code sensor plane

> **Status: live-verified** by the S1 spike (`examples/spike_hooks.md`,
> 2026-06-12, Claude Code 2.1.176): flags block with the message fed back to
> the model, holds raise a real permission prompt quoting the conflicting
> memory, silence is a no-op, and everything fails open when the server is
> unreachable.

The checkpoint layer is the watchdog re-grounded for MCP
(`docs/design/WATCHDOG_LAYER.md`, 2026-06-12 amendment): the harness's hooks
are the sensor/actuator plane, the Parallax server is the brain. **It is off
by default** — the three `checkpoint_*` tools sit in the catalog, but nothing
invokes them until you install these hooks. That installation is the layer's
explicit opt-in (006 FR-007).

## What each hook does

| Hook | Tool | Boundary | Intervention |
|---|---|---|---|
| `PreToolUse` (matcher: `Bash\|Write\|Edit`) | `checkpoint_action` | before a pending action | **hold**: risk-matched actions (deploys, pushes, deletes, …) that conflict with a verified stored constraint pause for your confirmation, quoting the memory. Everything else passes with no evaluation. Hard 500 ms budget; timeout = pass. |
| `PostToolBatch` | `checkpoint_batch` | after each completed tool batch | **flag**: loops (same action ≥4× in 10 batches) and repeated failures (same action failing 3× consecutively) are named to the model. Pure and local — no model call. Delivered flags cool down for 30 min. |
| `Stop` | `checkpoint_turn` | end of turn | **flag (forced continuation)**: a confirmed self-contradiction (final message vs an earlier committed statement or stored decision, with no intervening evidence) blocks the turn from ending until the model reconciles it — at most once per turn. |

Silence is the default: the layer's make-or-break acceptance criterion is
≥95% silence on benign sessions (measured 100% in acceptance run 1).

## Install

1. Make sure the running `mcp-parallax` build serves the checkpoint tools
   (`/mcp` → parallax → tools lists `checkpoint_action/batch/turn`).
2. Merge the `"hooks"` object from `hooks.json` (this directory) into your
   project's `.claude/settings.json` (or user settings for all projects).
3. Restart the session.

The matchers exclude `mcp__parallax__*` so a checkpoint can never trigger a
checkpoint. The gate's risk patterns can be extended with the
`CHECKPOINT_GATE_PATTERNS` env var (comma-separated substrings) on the
server.

## Uninstall

Remove the three hook entries and restart. That restores the prior state
completely — no server change, no catalog change, zero checkpoint
evaluations afterward (SC-006).

## Failure behavior

Every path fails open (006 FR-008): an unreadable transcript, a slow or dead
server, an embedding timeout — all degrade to silence (recorded with
`fail_open = 1` in `checkpoint_records` when the server was reachable), and
Claude Code itself ignores failing hook handlers. A broken checkpoint layer
cannot block your session.

## Audit

```bash
sqlite3 <DATABASE_PATH> "SELECT boundary, verdict, suppressed, fail_open, latency_ms FROM checkpoint_records ORDER BY created_at DESC LIMIT 20;"
```
