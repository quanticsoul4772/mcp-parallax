# Contract: configuration (067, §5.1 config)

| Variable | Values | Default | Rules |
|---|---|---|---|
| `PARALLAX_JEV_MODE_TOOL_RISK` | `off`, `shadow` | `off` | `on` is a startup error; `shadow` without a non-blank `TYPESAFE_API_KEY` is a startup error; any other value is a startup error |

- Operator-owned: classified as such in `NEW_SERVER_DESIGN.md` §10 (§5.1 config).
- The suffix `TOOL_RISK` joins the closed `PARALLAX_JEV_MODE_<SITE>` set through `JevSite::ALL`.
- `--help` and the README env table gain one row each.
- When the site is `shadow`, the startup log names the destination host and the data class sent (§5.1 config).
