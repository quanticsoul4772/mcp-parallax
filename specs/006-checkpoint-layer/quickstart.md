# Quickstart: Checkpoint Layer

## Enable

The three `checkpoint_*` tools are always in the catalog, but **nothing
invokes them until you install the sensor plane** — the layer is off by
default (FR-007). Install:

1. Open `integrations/claude-code/README.md` and merge
   `integrations/claude-code/hooks.json` into your Claude Code hooks
   configuration (project `.claude/settings.json` or user settings).
2. Restart the session. Three hooks now fire: pre-action (risk-matched tools
   only), post-tool-batch, and end-of-turn.

Uninstall = remove the hook entries; nothing else changes (SC-006).

Credentials: no new ones. The gate and turn boundaries use `VOYAGE_API_KEY`
(memory recall) when present — without it, memory-paired signals are silently
inactive and loop/failure screening still works. The turn review pass uses
the existing `ANTHROPIC_API_KEY`.

## What you'll see

- **Nothing, almost always.** Silence is the default; SC-001 holds the layer
  to ≥95% silence on benign sessions.
- **A flag** after a tool batch when the agent loops ("`cargo test` has
  failed 3 consecutive times with the same invocation…") or at turn end when
  the conclusion contradicts an earlier decision (both statements quoted),
  delivered as a forced continuation the model must address.
- **A hold** when a risk-matched action conflicts with a verified stored
  constraint — the action pauses for your confirmation with the memory
  quoted. Denying it lets the agent course-correct; confirming runs the
  action unmodified.

## Spikes (run before implementation hardens)

```bash
# S1 - hook->tool plumbing (manual, live Claude Code; protocol in examples/spike_hooks.md):
#   payload shape, result->hook-control mapping, self-trigger exemption,
#   PostToolBatch availability, Stop field names.
# S2 - gate latency:
cargo run --release --example spike_embed_latency   # Voyage query-embed p50/p95 -> research.md D4
```

## Acceptance (SC-001/002/003/005/007)

```bash
cargo run --release --example acceptance_checkpoint
```

Replays ≥20 recorded benign trajectories and ≥12 seeded-failure trajectories
(all four v1 signals, plus one evidence-justified reversal that must stay
silent) through all three boundaries via the in-process server: asserts ≥95%
silence + zero holds on benign (SC-001), ≥80% catch / 100% seeded-hold
(SC-002), gate budget compliance (SC-003), fail-open under unavailable deps
(SC-004), one record per evaluation with rates computable by SQL (SC-005),
and evidence-bearing messages (SC-007). The session-level halves of SC-004
(server killed mid-session) and SC-006 (hooks uninstalled → zero new
records) are verified by the live protocol in T011. Results recorded below
when run.

## Inspect

```bash
sqlite3 ./data/parallax.db "SELECT boundary, verdict, suppressed, fail_open, latency_ms FROM checkpoint_records ORDER BY created_at DESC LIMIT 20;"
sqlite3 ./data/parallax.db "SELECT boundary, COUNT(*) total, SUM(verdict!='silence') fired, SUM(suppressed) suppressed, SUM(fail_open) fail_open FROM checkpoint_records GROUP BY boundary;"
```
