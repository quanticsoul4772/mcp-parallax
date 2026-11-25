# Contract: configuration (064)

The new variables, as they appear in `--help` and the README environment table. A dedicated test binds this wording to both. The `PARALLAX_JEV_MODE_` prefix namespace is invisible to the `config_facts` scan, so it gets its own test, modeled on the routing prefix test (`src/main.rs:433`), with sites and values taken from `JevSite::ALL` and `JevMode::ALL` and matched as whole words.

| Variable | Values | Default | Notes |
|---|---|---|---|
| `TYPESAFE_API_KEY` | string | unset | Presence enables nothing by itself. A blank value counts as unset. Never logged. Removed from the claude_cli child environment. |
| `PARALLAX_JEV_MODE_TURN_REVIEW` | `off` \| `shadow` | `off` | `shadow`: Jev answers beside every end-of-turn review, recorded only. Sends a redacted turn summary, transcript-only statement pairs and the final assistant text. Requires the key. |
| `PARALLAX_JEV_MODE_RESEARCH_CLAIMS` | `off` \| `shadow` | `off` | `shadow`: Jev answers beside every verified research claim, recorded only. Sends the redacted claim and its source excerpts. Requires the key. |
| `PARALLAX_JEV_MODEL` | model id | `jev-1.14.0` | Pinned model. exa-mcp-server's `JEV_MODEL` is never read. |

## Errors (startup, naming the variable)

- Any other `PARALLAX_JEV_MODE_*` suffix.
- A value other than `off` or `shadow`; `on` doesn't exist in 064.
- A mode set to `shadow` while `TYPESAFE_API_KEY` is unset or blank.

## Startup log

One line per enabled site, naming the destination host (`api.typesafe.ai`) and the data class sent. The key never appears.

With no key, one line: `Jev off: TYPESAFE_API_KEY is not set, so no Jev site runs`.

## Ownership

All four variables are operator-owned. They are added to the owner table in `docs/design/NEW_SERVER_DESIGN.md` §10.
