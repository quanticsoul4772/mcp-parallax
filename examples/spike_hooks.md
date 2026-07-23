# S1 spike: hook → checkpoint-tool plumbing (006, research.md D1/D2)

**Status: COMPLETE (2026-06-12, three rounds).** Every protocol item ran
live; `integrations/claude-code/hooks.json` is final (schema-correct
`mcp_tool` shape; results carry the hook-output mapping). Verdict: the
`mcp_tool` handler design stands — no CLI fallback needed.

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

### Round 2 (2026-06-12, live session, schema-correct config installed)

**All three hooks fire.** `PreToolUse` -> `checkpoint_action` and
`PostToolBatch` -> `checkpoint_batch` produced one success row per
boundary crossing (records accruing with the live session id, all
silence/fail_open=0 on benign work). `Stop` -> `checkpoint_turn` fired and
errored — see finding 5. Answers to the round-2 questions:

4. **`${path}` substitution typing**: objects JSON-stringify into string
   params (`${tool_input}` deserialized fine into the tool's `tool_input:
   String`); **booleans stringify too** — `${stop_hook_active}` arrived as
   the string `"false"` and the tool's `continuation: bool` rejected it
   (`MCP error -32602 ... invalid type: string "false", expected a
   boolean`, surfaced to the user as a non-blocking Stop-hook error =
   harness-level fail-open confirmed). Fixed: `continuation` now accepts
   boolean or `"true"`/`"false"` (lenient deserializer; contract updated).
5. **Result -> hook-control mapping (the decisive question)**: per the
   hooks reference, *"the tool's text content is treated like command-hook
   stdout: if it parses as valid JSON output it is processed as a
   decision."* Confirmed live: an induced repetition flag fired
   server-side (4x identical command -> flag row with named evidence) but
   did NOT reach the model — `CheckpointResult` carried no hook-output
   fields, so the decision-less JSON was a no-op. **Fix (keeps `mcp_tool`,
   no CLI fallback)**: flag results now carry `decision: "block"` +
   `reason`; hold results carry `hookSpecificOutput: { hookEventName:
   "PreToolUse", permissionDecision: "ask", permissionDecisionReason }`;
   silence carries neither (observed: decision-less JSON is perfectly
   quiet). Contracts updated in the same change.
6. **Self-trigger exemption: confirmed.** Hook-invoked checkpoint calls
   produced no further hook rows (no cascade, counts exactly one
   action row per matched tool call, one batch row per batch); the
   `PreToolUse` matcher (`Bash|Write|Edit`) doesn't match MCP tools
   anyway.
7. **Stop field names confirmed**: `stop_hook_active` exists and
   substitutes; `last_assistant_message` produced no missing-field error.
8. **Detector precision finding (fixed)**: the model varies the Bash
   `description` between retries of the SAME command, which made
   normalized inputs differ and blinded the detectors — with identical
   inputs the repetition flag fired exactly at threshold. `description`
   is now a dropped (narrative) key in normalization.
9. **Transcript write-lag observed**: the batch hook can run before the
   latest tool_result lines are flushed, so failure marks may trail by
   one batch; detection then fires on the next checkpoint. Acceptable —
   named, not fixed (the signal arrives one batch late at worst).

### Round 3 (2026-06-12, rebuilt binary with the round-2 fixes) — ALL PASS

- [x] **Flag delivery**: an induced loop produced
      `PostToolBatch hook stopped continuation: Trajectory checkpoint
      (automated): ...` — the flag message reached the model as blocking
      feedback and the model course-corrected. Bonus: with `description`
      normalization fixed, the round-2 probes (same command, varied
      descriptions) fired retroactively — two `repeated_failure` signals
      named in one flag, both keys in `delivered_keys`.
- [x] **Hold delivery**: seeded constraint + conflicting (harmless) command
      -> the user received a real permission prompt and approved; the
      command then ran unmodified (US2-AS4 live). `hookSpecificOutput.
      permissionDecision: "ask"` from the mcp_tool result is honored.
- [x] **Stop boundary**: turn row with `review_ran = 1` (3.1 s) — the
      stringified `"false"` accepted by the lenient deserializer, real
      transcript mined, candidates found, the live review hop ran and
      correctly cleared them (decline bias held; no false alarm).
- [x] **Cooldown in anger**: 18 suppressed batch rows while the old probe
      signals stayed in the window — zero noise reached the model.
- [x] **SC-004 (layer unreachable)**: during the mid-session server
      restart, both hooks errored `MCP server 'parallax' not connected`
      as NON-BLOCKING errors and the session proceeded unimpeded.
- [x] **SC-006 (inertness)**: round 1 is the evidence — an entire working
      session with the hooks absent produced zero checkpoint rows.

Session totals at spike close: 33 action evaluations (2 holds, both
seeded tests), 37 batch (3 flags, 18 suppressed), 2 turn (1 with the
review hop), 0 fail-open rows.

### Round 2 — open questions (answered above; kept for the protocol record)

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

---

## S2 spike: UserPromptSubmit → surface plumbing (016, T011)

**Status: COMPLETE (2026-07-23, two rounds, live).** Verdict: the
`mcp_tool` `UserPromptSubmit` design stands — no adapter needed, no STOP
branch taken.

### Verified live

1. **Payload shape**: `${session_id}` and `${prompt}` substitute exactly as
   documented — `surface` deserialized both on the first firing (a mismatch
   would have been a loud `-32602`; the S1 round-2 stringification issue
   does not recur here since both fields are strings).
2. **Silence path**: an unrelated prompt produced one `push_records` row
   (`surfaced_ids: []`, 227 ms, `fail_open: 0`, 2 embed tokens) and a clean
   `success` invocation attributed to `voyage-4` — and injected nothing.
3. **`additionalContext` round-trip**: a seeded first-hand marker fact +
   a topically-related prompt surfaced end-to-end — the model received the
   full advisory template verbatim (label, `[fact, first_hand, memory
   <id>]`, content) in its context and quoted it back. Audit row:
   `surfaced_ids: ["<seed id>"]`, 176 ms.
4. Both evaluations sat comfortably inside the 500 ms budget; the seed was
   `forget`-cleaned afterward.

The `integrations/claude-code/hooks.json` `UserPromptSubmit` entry is
finalized from the verified shape; the same entry dogfoods in this repo's
`.claude/settings.json`.
