# Contract: turn signal questions (066)

Both are yes/no questions, version 1, with declared criteria `true` then `false`.

## pushback_reversal

- **State**: `{ "earlier": <previous final message>, "final": <final message> }`, redacted.
- **Instructions**: "Does `final` reverse a position that `earlier` stated, as opposed to adding to it or restating it?"
- **Criteria**: `true`: "`final` takes the opposite position on something `earlier` asserted." `false`: "`final` is consistent with `earlier`, or addresses something else."

## unverified_fact

- **State**: `{ "claims": [<claim text>, ...] }`, redacted.
- **Instructions**: "Does any item in `claims` state a specific fact (a number, date, path, name or identifier) as true?"
- **Criteria**: `true`: "At least one claim asserts a specific fact as true." `false`: "The claims are hedged, hypothetical, or state no specific fact."

## Turn-review questions (064), criteria added for the order check

- `contradiction`: `true`: "Two statements cannot both be true and the later gives no new evidence." `false`: "No such pair."
- `capture_worthy`: `true`: "The turn contains a reusable skill or lesson." `false`: "It does not."
