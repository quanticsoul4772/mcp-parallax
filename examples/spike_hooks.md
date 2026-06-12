# S1 spike: hook → checkpoint-tool plumbing (006, research.md D1/D2)

**Status: PROTOCOL — findings not yet recorded.** This spike needs a live
Claude Code session running a build that serves the `checkpoint_*` tools
(merge `006-checkpoint-layer`, rebuild, restart). It gates the final shape
of `integrations/claude-code/hooks.json`, which ships as a draft until the
boxes below are checked.

## What S1 must verify

1. **Payload shape**: does the `mcp_tool` hook handler deliver the hook
   event's JSON (with `session_id`, `transcript_path`, and for `PreToolUse`
   the `tool_name`/`tool_input`) as the tool's input arguments — and in what
   field mapping? The checkpoint tools require exactly
   `{session_id, transcript_path, ...}`; if the harness passes the raw hook
   payload (`hook_event_name`, `cwd`, …), the tools' params need a
   pass-through field strategy or the integration needs a `command`-type
   adapter.
2. **Result → hook-control mapping**: does a `mcp_tool` handler map the
   tool's structured result onto hook control fields? The required mappings:
   - `verdict: "hold"` → `permissionDecision: "ask"` + reason = `message`
   - `verdict: "flag"` → `decision: "block"` + reason = `message`
   - `verdict: "silence"` → no-op
   If the mapping is not automatic, the fallback is `command` handlers
   invoking a one-shot CLI mode of the binary (D1 named fallback — record
   the deviation in research.md D1 before finalizing hooks.json).
3. **Self-trigger exemption** (spec edge case 1): with the hooks installed,
   does a hook-invoked call to a `checkpoint_*` tool re-fire `PreToolUse`/
   `PostToolUse` hooks? The matchers below exclude `mcp__parallax__*`
   defensively either way — verify the exclusion holds.
4. **`PostToolBatch` availability** in the installed Claude Code version
   (fallback: `PostToolUse` with the matcher narrowed, accepting per-call
   volume until batch events are available).
5. **`Stop` payload fields**: confirm `last_assistant_message` (or record
   the actual field name) and the continuation indicator
   (`stop_hook_active`-style). If the final-message field is absent, the
   recorded fallback is: `checkpoint_turn` reads the final assistant message
   from the transcript tail and `final_message` becomes optional in the
   contract.
6. **Latency in anger**: time a `PreToolUse` round trip end to end in the
   live session (hook dispatch + tool call + verdict) — the harness-side
   overhead on top of the measured 136 ms p95 gate evaluation.

## Protocol

1. Merge + rebuild + restart so the live server carries the checkpoint
   tools; confirm with one direct `checkpoint_batch` call (real
   `transcript_path` from this session — it returns silence or a flag, and
   one row lands in `checkpoint_records`).
2. Install `integrations/claude-code/hooks.json` (draft) into the project's
   `.claude/settings.json`; restart.
3. Induce a loop: run the same failing command 4 times; observe whether the
   flag reaches the model (visible course-correction or the message in the
   transcript).
4. Store a constraint memory (`save`), then ask the agent to run a
   conflicting risky command; observe the hold (permission prompt quoting
   the memory).
5. End a turn that reverses an earlier statement; observe the forced
   continuation.
6. SC-004 live check: kill the server process mid-session; run benign
   commands — the session must proceed (hooks fail open on handler errors).
7. SC-006 inertness check: remove the hook entries, restart, run a benign
   session; `checkpoint_records` must gain zero rows.

## Findings

### Round 1 (2026-06-12, Claude Code 2.1.176, settings schema + first install)

1. **The original draft hooks.json was wrong and silently inert.** The user
   installed it; zero hooks registered (no checkpoint rows accrued, no
   errors surfaced). Root causes, verified against the authoritative
   settings JSON schema (`json.schemastore.org/claude-code-settings.json`):
   - `mcp_tool` handlers require `server` (the configured MCP server name)
     plus `tool` (the **bare** tool name on that server, not the
     `mcp__server__tool` form).
   - **The hook event payload is NOT auto-passed as tool arguments.** The
     handler takes an explicit `input` object; string values support
     `${path}` substitution from the hook event JSON
     (`${session_id}`, `${transcript_path}`, `${tool_name}`,
     `${tool_input}`, `${last_assistant_message}`, `${stop_hook_active}`).
   - hooks.json rewritten to the schema-correct shape in the same change.
2. **`PostToolBatch`, `Stop`, and `mcp_tool` all exist in 2.1.176** (schema
   lists all 29 events; the five handler types match the research). The
   matcher field is per-event-semantics; the draft's negative-lookahead
   exclusion for `PostToolBatch` was dropped — self-trigger exemption is
   verified empirically instead (round 2).
3. `command` hooks support an `if` permission-rule filter (tool events
   only) — a future option for harness-side risk narrowing.

### Round 2 — open questions (verify after restart with the new config)

- [ ] Hooks fire at all three boundaries (checkpoint_records accrues rows
      with the live session id).
- [ ] `${tool_input}` substitution: the hook payload's `tool_input` is an
      object; does substitution into a string JSON-stringify it, and does
      `${stop_hook_active}` (boolean) deserialize into the tool's boolean
      `continuation` param? A type mismatch errors the call → fail-open;
      fix would be loosening the wire contract types.
- [ ] **Result → hook-control mapping**: does the tool's structured result
      drive hook behavior (`hold` → block/ask, `flag` → `decision:"block"`)
      or is it ignored? If ignored, the D1 named fallback (`command`
      handler + one-shot CLI mode emitting hook-output JSON) replaces
      `mcp_tool`.
- [ ] Self-trigger exemption: hook-invoked checkpoint calls must not
      re-fire `PreToolUse`/`PostToolBatch` (watch for runaway records).
- [ ] `Stop` payload field names in anger (`last_assistant_message`,
      `stop_hook_active`).
- [ ] Live protocol: induced loop → visible flag; seeded constraint →
      hold prompt; SC-004 kill-test; SC-006 inertness after uninstall.
