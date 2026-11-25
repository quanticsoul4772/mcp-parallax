# Contract: configuration additions

| Variable | Values | Default | Errors at startup |
|---|---|---|---|
| `PARALLAX_JEV_MODE_RESEARCH_INJECTION` | `off`, `shadow`, `on` | `off` | unparseable value; non-off without `TYPESAFE_API_KEY`; `on` when `PARALLAX_JEV_MODEL` has no pinned injection threshold (names both variables) |
| `PARALLAX_JEV_MODE_TURN_REVIEW`, `PARALLAX_JEV_MODE_RESEARCH_CLAIMS` | `off`, `shadow` | `off` | unchanged from 064; `on` remains a startup error |

- The suffix set is `JevSite::ALL`: `TURN_REVIEW`, `RESEARCH_CLAIMS`, `RESEARCH_INJECTION`; any other suffix is a startup error.
- With the site enabled, one startup line names the destination host (`api.typesafe.ai`) and the data class: "kept research pages (URL and capped text, redacted)".
- `--help`, the README env table and the `PARALLAX_JEV_MODE_` prefix help test list the new site and the `on` value.
