# Contract: configuration additions (066)

| Variable | Values | Default | Effect |
|---|---|---|---|
| `CHECKPOINT_TURN_SIGNALS` | `off` \| `on` | `off` | `on`: at each turn end, the pushback-reversal and unverified-fact detectors run in code and one `turn_signal_records` row is written. Nothing is sent off the machine. |
| `PARALLAX_JEV_MODE_TURN_REVIEW` | `off` \| `shadow` \| `on` | `off` | `on` is refused at startup in 066. |

Unparseable values and `PARALLAX_JEV_*` names outside the known set are startup errors naming the variable. `CHECKPOINT_TURN_SIGNALS` is operator-owned (`NEW_SERVER_DESIGN.md` §10).
