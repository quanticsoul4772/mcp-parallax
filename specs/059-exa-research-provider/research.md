# Research: Exa replaces Brave for research

Reference implementation: an earlier private Exa integration (0.9.1),
read 2026-11-21: `src/tools/webSearch.ts`, `src/tools/webFetch.ts`,
`src/tools/shared.ts`, `src/utils/exaClient.ts`, `src/types.ts`,
`docs/TOOL_REFERENCE.md`. Exa API facts below come from it.

## D1 — Search for URLs, then one contents request for the kept sources

**Decision** (amended 2026-11-22 by analyze finding A8): each scoped angle
makes one `POST {base}/search` with header `x-api-key`, body `{ query, type,
numResults, includeDomains?, excludeDomains? }` and no contents request; each
result carries `url` and `title`. After URL dedup, the local domain filter and
the `max_sources` cut, the run makes one `POST {base}/contents` with `{ urls,
text: { maxCharacters }, maxAgeHours }` for the kept URLs (at most 60, within
Exa's 100 per request). Each result carries `url`, `title` and `text`;
`statuses[]` reports a per-URL `error`, which drops and counts that source.
Both responses carry `costDollars.total`. If the contents request fails
after retries, the run fails with that error (023); tokens already spent
stay metered (020).

**Rationale**: Exa bills text separately (`costDollars.contents.text`) on top
of a per-request price ($0.007-0.015 for both endpoints, Exa's OpenAPI spec).
Research searches for about twice the results it keeps, so text returned with
the search would be paid for on every dropped candidate. One extra request per
run buys text for only the sources used.

**Alternatives considered**: text inline with the search (one request fewer,
pays for dropped candidates); inline text with fewer results per angle (less
dedup headroom, worse coverage).

## D2 — Depth maps to an Exa search type

**Decision** (amended 2026-11-22 by analyze finding A7): `quick` → `auto`,
`standard` → `auto`, `deep` → `deep-lite`.

**Rationale**: Exa's OpenAPI spec marks `fast` and `instant` legacy (replaced
by `lite`), and `lite` rejects parameters outside an unlisted restricted set,
which may include the domain filters. `quick` stays quick through its tier (3
angles, 8 sources, 1 vote). `deep` and `deep-reasoning` are avoided: the
reference marks them multi-step and able to run for minutes per search, and a
deep run makes 8 searches. `deep-lite`'s cost and latency are measured in the live acceptance
run (quickstart); if they are out of line with `auto`, changing the mapping is
a plan change, not a silent swap.

**Alternatives considered**: `lite` for `quick`, checked live (undocumented
restrictions); keeping legacy `fast`; `deep` for `deep` (latency risk under
the run deadline).

## D3 — Freshness: `EXA_MAX_AGE_HOURS`, default 168

**Decision**: new operator variable `EXA_MAX_AGE_HOURS` (integer -1..=720;
default 168, one week), sent as `maxAgeHours` on the contents request. Out of range or
unparseable is a startup error naming it.

**Rationale**: FR-006 needs a stated limit. Exa's own default (omit the field)
serves cached text of any age; a week keeps research answers current while
still using Exa's cache for most pages. The range is Exa's (0 = always live,
-1 = cache only).

**Alternatives considered**: no variable, a fixed constant (operators could
not force fresh crawls); omit the field (no limit at all).

## D4 — Per-source text cap and truncation

**Decision**: the contents request asks for `text.maxCharacters` = `EXTRACT_TEXT_MAX_CHARS`
(16,000, the cap extraction already applies). A source whose text length is at
least 99% of the cap is marked truncated (the reference's rule; Exa does not
report truncation). The mark is returned as a new `truncated` field on each
entry of the result's `sources` list.

**Rationale**: today the 16,000-character cut happens silently; FR-007 asks
for it to be visible. Exa doing the cut means no bytes beyond the cap cross
the wire.

**Spec impact**: the spec's assumption "the tool's input and output shapes do
not change" is contradicted by this additive field; the spec is amended to say
the output gains one field (`sources[].truncated`) and nothing else changes.

## D5 — Readability step removed

**Decision**: Exa's `text` is used as the source text directly (trimmed). A
result with no text, or only whitespace, is dropped and counted as an
unreadable source is today. The title is Exa's `title`, else the URL.
`extract::readable_text` and the `rs-trafilatura` dependency are removed.

**Rationale**: Exa returns cleaned page text; running an HTML extractor over
it is meaningless. Dropping empty text keeps the existing "no main text"
accounting.

## D6 — The page fetcher is deleted (clarification Q2)

**Decision**: remove `research/fetch.rs` (`HygieneFetcher`, robots.txt,
domain spacing, the private-address guard, byte caps), the `Fetcher` seam
(`traits/fetcher.rs`), `FETCH_ALLOW_PRIVATE`, `FETCH_TIMEOUT_MS`, and the
`robotstxt` dependency. `reqwest`'s `stream` feature is dropped if nothing
else uses it (only `fetch.rs` calls `bytes_stream` today).

**Rationale**: no caller remains; Parallax makes no page requests (SC-002), so
the private-address guard has nothing to guard. Exa crawls on its side.

## D7 — Removed variables at startup (clarification Q1 as revised, Q2)

**Decision**: `FETCH_ALLOW_PRIVATE` or `FETCH_TIMEOUT_MS` present is a
`ConfigError` naming it and saying Parallax no longer fetches pages.
`BRAVE_API_KEY` present with `EXA_API_KEY` absent is a `ConfigError` naming
both and saying Brave is removed; with `EXA_API_KEY` present, `BRAVE_API_KEY`
is ignored (amended 2026-11-22: other tools share `BRAVE_API_KEY`, and the
operator's environment sets it, so a blanket error would stop Parallax on every
launch).

**Rationale**: the project's loud-configuration rule; a silent loss of the
research tool after upgrade is what it prevents.

## D8 — Domain constraints go to Exa and stay local (clarification Q3)

**Decision**: `domains_allow` → `includeDomains`, `domains_deny` →
`excludeDomains`, sent on every angle's search; the existing local filter
(registrable-domain match) still runs and is the authority. The search seam
takes them per call (they are per-run constraints).

**Rationale**: excluded domains stop consuming result slots; the local filter
keeps "never fetch these domains" exact even if Exa's matching differs.
Exa's OpenAPI spec (Context7 /exa-labs/openapi-spec, read 2026-11-22) lists
includeDomains and excludeDomains as independent optional fields of up to
1,200 entries each (hostname, path prefix or *.example.com), with no rule
against sending both.

## D9 — Retries

**Decision**: the Exa client retries network errors, 429 and 5xx up to
`MAX_RETRIES`, waiting `Retry-After` (seconds or HTTP date) when present,
otherwise exponential backoff from the existing base; other 4xx are not
retried and map to the existing outcome taxonomy (`provider_rejected` for
4xx other than 429, per 057). Both endpoints used (`/search`, `/contents`)
are read-only, so every request is safe to retry.

**Rationale**: the reference's rule; Brave's client did not honor
`Retry-After`, which SC-004 now requires.

## D10 — Exa cost recorded separately from model cost

**Decision**: each seam call (search and contents) returns its request's reported cost
(`costDollars.total`, `None` when absent). The run meter sums it and notes any
request without a cost. The invocation record gains `search_cost_usd`
(nullable): the run's sum when every Exa request reported a cost, null when
any did not, and null for every other tool. A new nullable SQLite column,
added by the existing additive `ALTER TABLE` migration pattern. The research invocation span carries it as `parallax.search.cost_usd` when
present (analyze A3: span only; a dollar value is not used as a metric
attribute). `cost_usd` stays the model cost. The cost is recorded on failed runs
too, as model tokens are (020).

**Rationale**: FR-010 and SC-003 (record and telemetry agree because both
derive from the record at the one exit point, 007 FR-009). A separate field
keeps `cost_usd`'s documented meaning (summed model cost) intact for anyone
already reading it.

**Alternatives considered**: add Exa's cost into `cost_usd` (changes the
meaning of an existing column and its telemetry attribute).

## D11 — Test seam, no base-URL variable

**Decision**: `ExaClient::with_base_url` and `with_backoff_base_ms` for tests,
as `BraveClient` had; no `EXA_API_BASE` variable.

**Rationale**: the integration tests construct the client directly; no
operator need for another base URL is known (YAGNI).

## D12 — Exa features not used (FR-011)

| Feature | Reason |
|---|---|
| `category` filters | Research questions are open-ended; a category would silently narrow sources the scope step chose. |
| `outputSchema` (synthesized answer) | Parallax's own model writes the answer from verified claims; an Exa answer would bypass verification. |
| `summary` | Claims are extracted from page text; a summary is a second model's paraphrase. |
| `highlights` | Excerpts of ~500 characters are too short for claim extraction and grounding. |
| `subpages` | Multiplies cost per result; sources are single pages. |
| `deep`, `deep-reasoning`, `lite`, legacy `fast`/`instant` types | D2: latency per search under the run deadline; `lite`'s undocumented restrictions; legacy values. |
| `userLocation`, `moderation`, `livecrawlTimeout`, `includeText`/`excludeText` | No research requirement; the last two are deprecated by Exa. |
| `/answer`, `/findSimilar`, `/context`, agent runs, monitors, websets | Not part of the research pipeline. |

## D13 — Design corpus amendments

**Decision**: dated amendments (2026-11-22) to `docs/design/SDK_LANDSCAPE.md`
(the research-search row and pick, the sequencing line, the cost line),
`docs/design/RESEARCH_PRIMITIVE.md` (fetch and fetch-hygiene: Exa supplies
text, Parallax fetches nothing), and `docs/design/NEW_SERVER_DESIGN.md` §10's
credentials row (`EXA_API_KEY` for `BRAVE_API_KEY`). The 007 telemetry
contract gains `parallax.search.cost_usd`.

**Rationale**: Constitution I: the corpus is the source of truth and names
Brave and the hygiene fetcher.

## D14 — Tests and acceptance

**Decision**: wiremock doubles for Exa in unit and integration tests (request
body asserted: type per depth, `maxAgeHours`, `maxCharacters`, domain lists;
response with and without `costDollars`; 429 with `Retry-After`; 400; empty
text). Pipeline tests drop the fetcher mock and feed text through the search
double. `examples/acceptance_research.rs` switches to Exa and is run three
times live before the PR (SC-001), reporting runs, findings and Exa cost.

**Rationale**: Constitution IV; the memory note on live acceptance for
changes to what the model reads (the page text format changes from
trafilatura output to Exa's text).
