# Quickstart: Claude Code CLI backend

## Prerequisites

- Claude Code installed and signed in (`claude`, then `/login`) on a
  subscription that allows Claude Code.
- No Anthropic API key needed.

## Configure

In the MCP server entry for Parallax:

```json
"env": {
  "PARALLAX_BACKEND": "claude_cli"
}
```

Optional: `CLAUDE_CLI_PATH` (if `claude` is not on the path),
`CLAUDE_CLI_MODEL` (default `claude-sonnet-5-5`), `CLAUDE_CLI_CONCURRENCY`
(default 4), and the usual `PARALLAX_MODEL_*` / `PARALLAX_EFFORT_*` routing.

## Check it works

1. Restart the session; the server log shows `backend=claude_cli` and the
   CLI version.
2. Call `decide` with two options: a scored result comes back.
3. Look at the invocation record: `cost_basis = cli_estimate`.

## Live acceptance (T-live)

`cargo run --example acceptance_claude_cli` with `ANTHROPIC_API_KEY` unset.
It runs every always-on tool once, a quick `research` question (needs
`EXA_API_KEY`), and 20 `decide` calls, and reports per call: time, schema
pass on first attempt, tokens and estimate. Pass: SC-001 to SC-005, with
Parallax itself configured as an MCP server in the operator's Claude Code to
prove isolation (no recursive call appears in the records).

Results are recorded below after the run.
