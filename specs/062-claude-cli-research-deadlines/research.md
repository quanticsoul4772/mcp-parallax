# Research: Research deadlines that fit the claude_cli backend

## D1: Where the backend reaches the deadline choice

- **Decision**: add `backend: Backend` to `ResearchDeps`; `validate_params`
  (which already takes `deps`) picks the default deadline from it.
- **Rationale**: the deadline is resolved once per run in `settings.rs`, which
  already receives `deps`. One field, set where `ResearchDeps` is built.
- **Alternatives**: a `deadline_scale` number on the deps (hides the per-depth
  values the user chose); changing `Depth::tier()` to take a backend (touches
  every tier caller for one field).

## D2: The values

- **Decision**: `claude_cli` quick 120 000 ms, standard 660 000 ms, deep
  1 980 000 ms; other backends keep 120 000 / 240 000 / 480 000 ms.
- **Rationale**: the user chose fixed per-depth values (2026-11-23). Measured
  on `claude_cli` at concurrency 4, full runs with no cutoff: quick 93 s,
  standard 538 s, deep 1632 s; the defaults are about 1.2x those. The first
  values (360 / 720 s) failed live: they were estimated from runs that had
  themselves been cut off at 240 / 480 s, and a standard run stopped at 360 s
  with 46 of 177 claims unverified.
- **Alternatives**: a multiplier (inflates quick, which already fits); an
  operator setting (no fix by default).

## D3: Tests

- **Decision**: unit tests on `validate_params` for each backend × depth, with
  and without a caller deadline; one pipeline test with a test clock showing a
  `claude_cli` standard run is not stopped at 300 s; the existing tier-table
  test stays as is (it pins the other backends' values).
- **Rationale**: SC-003 is shown by the unchanged tier test plus the
  non-claude_cli cases.
