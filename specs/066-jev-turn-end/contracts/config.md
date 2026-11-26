# Contract: configuration additions (066)

| Variable | Values | Default | Effect |
|---|---|---|---|
| `PARALLAX_JEV_MODE_TURN_SIGNALS` | `off` \| `shadow` | `off` | `shadow`: at each turn end, cheap detectors run and Jev is asked only what they leave open; results recorded only. Sends redacted assistant text and claims; never user text or tool output. Requires the key. |
| `PARALLAX_JEV_MODE_TURN_REVIEW` | `off` \| `shadow` \| `on` | `off` | `on` skips the end-of-turn review when Jev is confidently "no"; refused at startup unless the configured model has a pinned threshold entry and a recorded order check. |

Unknown `PARALLAX_JEV_*` names and unparseable values are startup errors naming the variable. Both variables are operator-owned (`NEW_SERVER_DESIGN.md` §10).
