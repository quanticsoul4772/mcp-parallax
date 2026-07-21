# Data Model: Preference Enforcement at the Checkpoint

No storage schema changes. All additions are in-process types; the existing
`checkpoint_records` columns (`signals_evaluated`, `signals_fired` as JSON)
absorb the new signal kind unchanged.

## 1. `SignalKind::PreferenceViolation` (extends `src/checkpoint/mod.rs`)

| Property | Value |
|---|---|
| Wire/column form | `"preference_violation"` (snake_case, `as_str` + serde) |
| Boundary | Turn only |
| Verdict subset | Silence or Flag — never Hold (spec FR-003) |
| Cooldown identity | `preference_violation:fnv1a64(memory.id)` (research D4) |
| Evidence string | quotes the preference + memory id + trust standing (spec FR-002) |

## 2. `PreferenceCandidate` (new, `src/checkpoint/preference.rs`)

One recalled enforceable memory, mined deterministically before the hop.

| Field | Type | Source / rule |
|---|---|---|
| `memory_id` | `String` | `Memory.id` — provenance + cooldown identity |
| `content` | `String` | `Memory.content`, quoted verbatim in the flag |
| `trust` | `Trust` | `FirstHand` or `Verified` only — `Untrusted` is structurally excluded (spec FR-005) |
| `score` | `f32` | cosine vs the final-message query embedding |

**Population rule** (research D2): `rank_recall` hits with
`score >= REVIEW_RECALL_FLOOR` AND `gate::is_constraint(memory)`. Capped (most
relevant first) alongside the contradiction candidates so the hop input stays
bounded.

## 3. `ReviewOut` extension (in `src/checkpoint/review.rs`)

The hop's constrained output — stays **flat + closed** (Constitution II).

| Field | Type | New? | Meaning |
|---|---|---|---|
| `contradicts` | `bool` | existing | a real, explicit, material contradiction exists |
| `statement_a` | `String` | existing | earlier statement, verbatim ("" when none) |
| `statement_b` | `String` | existing | final statement, verbatim ("" when none) |
| `basis` | `String` | existing | one sentence of grounds |
| `violates` | `bool` | NEW | the turn violates a listed preference (uncertain ⇒ `false` — decline bias, spec FR-004) |
| `violated_preference` | `String` | NEW | verbatim echo of the violated preference ("" when none) — used ONLY for server-side map-back (research D5) |
| `violation_basis` | `String` | NEW | one sentence: what in the turn violates it ("" when none) |

**Judged evidence in the prompt** (research D3): numbered preference contents +
final message (fixed char cap) + the deterministic window activity summary.

## 4. Violation flag (server-assembled, fixed template)

Parameterized ONLY by server-held evidence (mined candidate) + the hop's
basis sentence — never by free model wording (research D5):

```text
End-of-turn review: this turn appears to violate a stored preference:
"<candidate.content>" (memory <candidate.memory_id>, <trust> provenance).
Basis: <violation_basis> Revise the response to honor it, or state explicitly
why it does not apply here.
```

The closing clause implements spec FR-002's "fix or push back" and the spec
edge case where a live instruction overrides a stored preference.

## 5. State/flow deltas (`run_turn`)

```text
continuation?          → silence (unchanged; enforcement skipped — FR-009)
empty final_message?   → silence (unchanged — research D9)
read window            → unchanged (bounded)
turn_recall            → unchanged (one embed; None when memory off)
mine                   → contradiction candidates (unchanged)
                         + preference candidates (NEW, memory on only)
no candidates at all   → silence, no hop (unchanged shape)
one hop                → ReviewOut with both judgments (FR-010)
assemble               → contradiction flag, violation flag, both, or silence
                         (both ⇒ one concatenated message, both signals — D6)
cooldown               → per signal_key via existing `unsuppressed`
record                 → one row; signals_evaluated includes
                         preference_violation iff memory configured (D7)
```

## 6. `CheckpointRecord` / storage

Unchanged. The new kind serializes through the existing JSON columns;
`review_ran`, `cost_usd`, `delivered_keys`, `suppressed`, `fail_open` all keep
their current semantics. Audit queries for spec SC-005:
enforcement-evaluated ≡ `"preference_violation" ∈ signals_evaluated`;
enforcement-fired ≡ a `signals_fired` entry with that kind (its evidence names
the memory id).

## 7. Observability

`emit_checkpoint` mirrors records to OTLP at the same exit point (007) — the
new kind flows through with no telemetry contract change; the
`specs/007-observability-layer/contracts/telemetry.md` surface lists signal
kinds by value, so the exported attribute simply gains the new value.
