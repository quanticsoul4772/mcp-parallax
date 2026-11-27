# Quickstart: Jev turn signals (068)

## 1. Turn the site on (shadow)

```text
PARALLAX_JEV_MODE_TURN_SIGNALS=shadow        # needs TYPESAFE_API_KEY
PARALLAX_JEV_SEND_USER_TEXT=true             # optional: lets pushback_reversal see that turn's user text
```

`CHECKPOINT_TURN_SIGNALS` is removed; remove it from the setup (§4.1, T021).

## 2. Live example (T019, SC-005)

```text
PARALLAX_JEV_MODE_TURN_SIGNALS=shadow cargo run --release --example acceptance_jev_signals
```

Exits non-zero unless: every non-continuation turn end with a final message leaves one row per question and the checkpoint results equal the site-off run (SC-001); no request body carries user text without the opt-in (SC-002); no date- or number-bearing claim reaches `unverified_fact` (SC-003).

## 3. Data still needed (T019a)

After the live run, the plan reports the rows and yes answers so far, the data still needed for §5.4 and §7.3, and the expected date at the observed rate.
