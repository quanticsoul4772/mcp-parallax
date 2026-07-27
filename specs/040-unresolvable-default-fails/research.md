# Phase 0 Research: An Unresolvable Default Fails Instead of Being Skipped

**Feature**: 040 | **Date**: 2026-07-27

Four decisions. D2 and D3 were settled during `/speckit-clarify` and are
recorded here with the reasoning that produced them; D1 and D4 are new.

## D1 — One shared resolver, not two patched scans

**Decision**: extract a single resolver into `src/config_facts.rs` (test-only)
that both document checks call. Do not fix the two existing scans in place.

**Rationale**:

The two scans are already near-duplicates that have **drifted apart**:

| | `--help` scan | README scan |
| --- | --- | --- |
| Guards that the char after `parse_env(` is a quote | no | yes |
| Excludes the test fixture variable | no | yes |
| Coverage floor | `checked >= 8` | none |
| Line-ending normalisation | not needed | required, and initially missing |

That divergence is not incidental — it is the transmission mechanism for the
defect this feature exists to fix. 039 was written by copying 036's extraction,
so it inherited the silent skip along with the code, and then acquired two
guards 036 still lacks. Patching both would leave two things to keep in step
and a third copy waiting to be made the next time a document is added.

The corpus rule (§10) says to move a check closer to what it checks, and at the
top rung to make the failure class unrepresentable. Two copies of a scan cannot
be made unable to disagree. One resolver can.

**Alternatives considered**: patch both scans (rejected — leaves the duplication
that caused this); generate the documents from source instead of checking them
(rejected — the README's Purpose column and `--help`'s descriptions are
hand-written reasons, and §10 says derive facts while reasons stay written).

## D2 — Constants resolve from an enumerated file set

**Decision**: the resolver reads a named list of source files. A constant not
declared in that set is unresolvable and fails under FR-001.

**Rationale**: settled in `/speckit-clarify`. Searching the whole crate scored
15 against the alternatives: Rust permits one constant name in several modules,
so an unrestricted search resolves to whichever declaration it reaches first and
compares a document against the wrong value — a wrong answer that looks like
success, which is precisely this feature's own failure mode. Enumerating the
search space makes that unrepresentable rather than merely detectable
(whole-crate-with-ambiguity-failure scored 55 for catching it after the fact).

`decide` preferred a variant that also named "the file that would need adding"
(90 v 82) and the confirmation `verify` refuted it 3/3: a resolver that reads
only its declared set has no idea where a missing constant lives, so naming the
file requires exactly the unrestricted search the design rejects. **The message
states what the check can know** — the constant, the variable, and the set that
was searched.

**Alternatives considered**: whole-crate search (above); whole-crate with
ambiguity failure (above); requiring every default to be a literal (rejected —
it would mean editing `config.rs` to suit its test, and the constants exist for
good reasons, e.g. `DEFAULT_MODEL` is referenced from several places).

## D3 — The reverse direction reads structured markers only

**Decision**: the reverse check — every default a document states belongs to a
variable that has one — reads the README table's Default column and `--help`'s
`(default: X)` marker. It never reads surrounding prose.

**Rationale**: settled in `/speckit-clarify`, 85 against 62 for equal
strictness. Prose carries numbers that are not defaults: ranges like `1..=20`,
ceilings quoted inside explanations, version strings. A reverse scan over prose
produces false positives, and a check that cries wolf gets silenced rather than
corrected — this feature's failure mode aimed the other way. It also follows
§10 exactly: the Default column is a fact, the Purpose column is a reason.

**Alternatives considered**: equal strictness over the whole document
(rejected, above); forward direction only (rejected at 32 — leaves a stale row
for a removed variable invisible, since the forward scan iterates variables the
source still contains); reverse as warn-only (rejected at 38 — Principle III
forbids a path that reports a problem without failing).

## D4 — Exclusions as a typed list in the resolver

**Decision**: exclusions are a `const` array of `(variable, reason)` pairs in
`config_facts.rs`, not a data file.

**Rationale**: the list is expected to hold zero or one entries. A data file
adds a parse step, a path, and a failure mode of its own for something that fits
on two lines. Keeping it in the resolver also puts the reason next to the code
that acts on it, so a reader hitting the failure sees why a sibling variable was
excused without opening another file.

FR-003 requires a stale exclusion to fail. That is cheap here: after resolution,
assert every excluded name still appears as a variable carrying a default. An
exclusion outliving its subject is how a suppression list quietly becomes
permanent.

**Alternatives considered**: a TOML or JSON file (rejected — more machinery than
the data justifies); an attribute or comment marker in `config.rs` itself
(rejected — it would put test concerns into production source, and `config.rs`
is the subject under examination, not a participant in its own examination).

## Cross-cutting: what stays as it is

- **`config.rs` is not modified.** The resolver adapts to how configuration is
  written, not the reverse. A test that requires its subject to be reshaped for
  its convenience measures the reshaping.
- **Variables read with no default stay out of scope** for value comparison and
  in scope for the existing presence assertions.
- **The production path is untouched.** Nothing here ships in the binary beyond
  the help text it already prints.
