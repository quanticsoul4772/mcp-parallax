# Research: Provider rejections recorded as their own outcome

## D1 — A new error variant and outcome, not a reused one

- **Decision**: add `AppError::ProviderRejected { status: u16, detail: String }`,
  displayed as `provider rejected the request: HTTP {status}: {detail}`, mapped
  to a new `Outcome::ProviderRejected` with the stable string
  `provider_rejected`.
- **Rationale**: the defect is that two different failures share one outcome.
  Only a distinct value separates them in the counts. The status is kept in the
  error text, so a reader can still tell a credit-balance 400 from a 401.
- **Alternatives considered**: map provider 4xx to `config_error` (rejected:
  `config_error` is documented as never appearing on a record, and a 400 caused
  by a server bug would be filed as operator error); split by status and match
  billing text (rejected: depends on provider wording, and leaves a remainder in
  `validation_failure`). Scored in-session: new outcome 82, split 55,
  `config_error` 45.

## D2 — Exactly two construction sites change

- **Decision**: `src/client/anthropic.rs` (the `!status.is_success()` branch
  after the 429/5xx retry check) and `src/client/openai_compat.rs` (the terminal
  branch after the strategy-ladder check) build `ProviderRejected` instead of
  `Client`. The Anthropic effort-remedy hint stays appended to `detail`.
- **Rationale**: these are the only places an HTTP 4xx from the model provider
  becomes an error. `Client` keeps its other meanings: transport failures and
  out-of-contract successful responses (unexpected `stop_reason`, unparseable
  body), which stay `validation_failure`.
- **Alternatives considered**: reclassify every `AppError::Client` (rejected: it
  would move out-of-contract 200 responses, which are model-output failures).

## D3 — Bind the outcome list to both published lists by test

- **Decision**: add `Outcome::ALL`. A test reads the `outcome.enum` array from
  `specs/001-core-layer/contracts/invocation-record.schema.json` and the
  backticked values in the `parallax.outcome` row of
  `specs/007-observability-layer/contracts/telemetry.md`, and asserts each equals
  the set of recorded outcomes (`ALL` minus `config_error`). It replaces
  `outcome_strings_match_the_record_contract`, whose comment says "must stay in
  sync" with the schema but which compares against literals and never opens it.
  A second test asserts `parse(as_str(o)) == Some(o)` for every `o` in `ALL`.
- **Rationale**: a hand-pinned list passes when the contract drifts — the
  pattern CLAUDE.md records for 040–045. Reading the files makes the published
  lists the thing under test. `ALL` with an exhaustive `match` in a test makes a
  new variant fail to compile until it is listed.
- **Alternatives considered**: add `provider_rejected` to the existing literals
  (rejected: keeps a check that cannot see the contract).

## D4 — Telemetry needs no code change

- **Decision**: telemetry emits `outcome.as_str()` for `parallax.outcome` and
  `error.type`, so the new value is exported as soon as it exists. Only the list
  in `telemetry.md` changes.
- **Rationale**: one taxonomy feeds both surfaces by design (`src/error.rs`
  module doc).

## D5 — Storage needs no migration

- **Decision**: `invocation_records.outcome` is a text column with no check
  constraint; `Outcome::parse` gains `provider_rejected` so records read back.
- **Rationale**: past records stay as written (spec assumption); new records
  carry the new value.

## D6 — Where the design amendment goes

- **Decision**: a dated 2026-11-20 amendment to `specs/001-core-layer/data-model.md`
  §6, where the taxonomy is defined.
- **Rationale**: `src/error.rs` cites that section as the taxonomy's home.
