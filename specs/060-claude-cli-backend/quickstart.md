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

## Live acceptance results (2026-11-22)

`cargo run --example acceptance_claude_cli` on the debug build, model
`claude-sonnet-5-5`, Claude Code 2.1.330, with `ANTHROPIC_API_KEY` and
`EXA_API_KEY` unset (subscription login only). OVERALL PASS.

| Check | Result |
|---|---|
| SC-001: decide, verify, unstick, diverge, elicit, check, grounded_verify | all PASS, 3.4–5.8 s each, every record `cost_basis = cli_estimate` |
| SC-003: CLI overhead | PASS, median 1,292 ms over 20 calls (target ≤ 3,000) |
| SC-004: schema adherence | PASS, 20/20 valid on the first attempt (target ≥ 19) |
| SC-005: isolation | PASS, 33 of 33 requests carried the isolation flags and variables |
| SC-002: research | not measured (run without `EXA_API_KEY` to save usage) |

The CLI's API-price estimates were about $0.01–0.04 per tool call; they
count against the subscription, not as a charge. A separate check showed a
`CLAUDE.md` planted in a parent folder steering answers before the
isolation fix and not after.

### Second run, with research (2026-11-22)

Same command with `EXA_API_KEY` set: OVERALL PASS. All seven tools passed
again; SC-002 PASS (a quick `research` question completed in 142 s,
`cost_basis = cli_estimate`); SC-003 median overhead 1,235 ms; SC-004 20/20;
SC-005 121 of 121 requests isolated. The research run's CLI estimate was
$1.23 on `claude-sonnet-5-5` for every step (about 70 calls), against
$0.004–0.013 per call for the other tools: research is the tool that draws
most on the subscription, and routing its bulk step to `claude-haiku-5-5`
(the backend default) lightens it.
