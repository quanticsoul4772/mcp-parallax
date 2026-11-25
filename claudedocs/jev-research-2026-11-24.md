# Jev, Voyage and Exa in Parallax: research report (2026-11-24)

Research step 1 of 8 for adding Jev to Parallax (research, requirements,
specify, clarify, plan, tasks and analyze, implement, PR). This report is for
review before requirements. No Parallax code was changed.

**How it was produced.** Four read-only subagents, each brief approved first:

- **A:** TypeSafe's official docs online.
- **B:** this session's transcript and the user's projects that use Jev.
- **C:** Parallax's design docs and code.
- **E:** Voyage and Exa against Parallax.

Three Parallax `research` runs (Jev, Voyage, Exa) served as a cross-check, and
16 live Jev calls were made on synthetic Parallax-shaped questions. Section 9
lists every source.

---

## 1. Summary

**What Jev is.** TypeSafe's first "System One" model.

- **Input and output:** you send a `state` (text or JSON) and a set of
  questions you define. Jev returns typed answers with probabilities: yes/no
  (`noul`), pick one (`choice`), ordered rubric (`score`), and image location
  (`box`, preview).
- **No text:** it writes no prose, code or explanations.
- **Speed and price:** measured live at 94–159 ms per call. It costs $0.042
  per million input tokens, and output is free.
- **For comparison:** one Parallax `verify` pass today takes 8.5–33.5 s and
  costs $0.039–$0.172.

**Where it fits Parallax best.** Parallax has no fast classifier today. Every
judgment, including ones whose output is already a typed yes/no or category,
goes to the large judgment model (default `claude-opus-4-8`). The strongest
fits, ranked by the Parallax code map (subagent C) and checked against the
live calls:

1. **Watchdog `checkpoint_turn` pre-screen.** The end-of-turn review makes a
   full model call on almost every turn (634 of 747 recorded). Its three
   outputs are already typed (contradicts, violates, capture kind).
2. **Memory consolidation relation.** A 4-way typed judgment (same, updates,
   context-specific, distinct) is a full model call today under a 5 s budget
   that can lapse silently. Jev answered "updates" at 0.99 on the
   SQLite-to-Postgres pair in about 0.1 s.
3. **Research claim verification as a cascade.** Claim verification is the
   dominant research cost. Jev could give a first verdict and escalate only
   uncertain claims to the full ensemble.
4. **Research injection screening.** 063's phrase list misses paraphrased
   injections. Jev caught the paraphrased test at 0.97, and the user's own
   back-test showed 0 false flags on 71 clean pages.
5. **`check` routing and translation round-trip.** Jev cleanly separated a
   computable claim (1.0) from an opinion (1.0).
6. **Watchdog `checkpoint_action` risk class.** It belongs alongside the
   substring list, not as a replacement.

**The main constraint.** Parallax's design forbids escalating on a model's own
stated confidence. It requires agreement across independent passes
(`docs/design/CORRECTIVE_SELECTION.md:62-70,90`). TypeSafe says Jev is
calibrated but publishes no calibration data. So every threshold-based use
needs Jev's probabilities measured against Parallax's own records first. The
tables that could supply this already exist: `checkpoint_records`,
`consolidation_records`, `invocation_records`, and the 054 decide dataset.

**What is unknown.** Missing from TypeSafe:

- calibration evidence of any kind;
- determinism (the live calls varied slightly: 0.81 and then 0.87 on the same
  input);
- a latency SLA;
- an official Rust SDK or MCP server;
- the actual rate-limit numbers;
- whether new sign-ups are open. The cross-check run found a TypeSafe post
  from 2026-09-24 saying direct sign-ups were closed. The user already has a
  key.

**Voyage and Exa are not finished.**

- **Voyage:** Parallax uses one embedding call shape, one text per request,
  plus rerank-3 on research pages. Unused: semantic claim dedup (the stats
  field already claims it, but the code is exact-match), reranking memory
  recall, batched embedding, and passage-level selection with contextual
  embeddings.
- **Exa:** Parallax uses five search fields and two contents fields. Unused:
  boilerplate stripping, publication dates, warnings and notices, per-URL
  error tags, and `additionalQueries` for the deep tier. Section 6 has the
  full list.

**Parallax's own research tool has a problem.**

- **Deadlines:** all three cross-check runs hit their deadline
  (`stopped_early=true`, 706–742 s against a 660 s deadline) while running
  alongside four subagents.
- **Thin results:** coverage was 0–0.14, and nearly every claim is
  single-source and unverified.
- **Credibility:** the Jev run cited several unofficial Jev sites
  (jevlaya.cc, bestjevai.com, typesafe-jev.ai, jevainews.com) at the same
  0.5 credibility as TypeSafe's own docs.

---

## 2. What Jev is (A1–A6)

Source tags: **[official]** means TypeSafe's own site, docs, API spec or
legal pages. **[third-party]** means anything else.

### 2.1 Products and intended use

- **The company:** TypeSafe AI, Inc. (San Francisco) builds "machine-native"
  models that make decisions inside software rather than writing text for
  people. [official] typesafe.ai
- **System One models:**
  - They return typed decisions with confidence.
  - They are trained with RLCD ("Reinforcement Learning for Calibrated
    Decisions").
  - They sample in parallel rather than token by token.

  [official] typesafe.ai/blog/introducing-system-one-models-and-jev
- **Jev:**
  - The first public System One model, "in early access".
  - Versions `jev-1.13.0` and `jev-1.14.0`. 1.14 was released 2026-10-14.
  - The aliases `jev-latest` and `jev-preview` both point to 1.14.0, and they
    move on new releases, so pin a version.

  [official] docs.typesafe.ai/models.md
- **Intended uses:** routing, classification, scoring and guardrails. Jev
  does not generate text, write code, stream, call tools or converse, and is
  "not a drop-in replacement" for an agent's LLM.
  [official] docs.typesafe.ai/concepts/system-one.md,
  docs.typesafe.ai/introduction/coding-agents.md
- **Advice from the docs:** ask one atomic thing per question and combine the
  answers in code. [official] docs.typesafe.ai/introduction
- **Known weak spots** (TypeSafe's own "jaggedness" list):
  - It reads questions literally.
  - It is weak at math, counting, dates and hex/RGB values.
  - Multi-hop reasoning and double negatives hurt it.
  - Large or irrelevant state distracts it.
  - **Text injected into state can shift answers.**
  - Contradictory criteria hurt results.
  - **It leans toward the first Choice option listed.**

  [official] docs.typesafe.ai/model-jaggedness.md
- **Other products and channels:**
  - Enterprise tier, console and Playground.
  - A Claude Code / Codex agent skill (`claude plugin install
    typesafe@typesafe-ai`).
  - A GitHub org with SDKs, an n8n node, WorkflowEvals and an LLM-backed
    adapter.

  [official]
- **Self-reported case studies:** Jack & Jill (88% cheaper), MotherDuck
  (98.7% lower cost), Sardine. [official] typesafe.ai/enterprise

### 2.2 How a question works

- **Who defines it:** the caller. `questions` is a map whose keys you choose;
  answers come back under the same keys. You write each question's
  `instructions` and `criteria`. There are no server-side question
  templates. [official] docs.typesafe.ai/api/system-one.md
- **Answer types:**

| Type | Use | Returns |
|---|---|---|
| `noul` | yes/no | `noul`: probability of yes, 0 to 1. No `confidence` field (TypeSafe suggests \|2p−1\| if you want one). |
| `choice` | pick one of 1 to 255 options | `choice` (top option), `confidence`, `probabilities` map summing to about 1 |
| `score` | ordered rubric, 1 to 10 levels | probability-weighted `score` (can fall between levels), `confidence`, `legend`, `probabilities` |
| `box` | locate something in an image | Jev 1.14 only, behind the `TypeSafe-Danger-Zone-Images: true` header |

  [official] docs.typesafe.ai/api/system-one.md, api.typesafe.ai/openapi.json,
  docs.typesafe.ai/changelog.md

- **No free numeric type.** "Jev's score levels are weak in numerical
  calibration": compare a score to a threshold, but don't read exact values
  from it. [official] docs.typesafe.ai/model-jaggedness.md
- **Structured instructions and criteria:** each can be a string, object,
  array or null. Taxonomies are walked one Choice per level.
  [official] docs.typesafe.ai/primitives/advanced.md
- **How confidence is computed:** Choice uses (p_max − 1/n)/(1 − 1/n). Score
  uses the whole distribution weighted by distance from the most likely
  level. TypeSafe calls confidence "one reasonable way to summarize a
  distribution, not the only one". [official] docs.typesafe.ai/confidence.md
- **Calibration is claimed, not shown:**
  - "Their probabilities are optimized against outcomes to reflect
    uncertainty." Also: "Calibration is measured across groups of
    predictions; it does not guarantee that an individual answer is
    correct." [official] docs.typesafe.ai/concepts/system-one.md
  - There are no reliability diagrams, ECE or Brier scores anywhere, and the
    eval site has no calibration analysis. The docs say to start with
    conservative thresholds and tune them on your own data.
    [official] docs.typesafe.ai/confidence.md, evals.typesafe.ai
- **Determinism:** not documented. The homepage FAQ asks "is Jev
  deterministic" but showed no answer. [official] typesafe.ai

### 2.3 The API

- **Base and spec:** base URL `https://api.typesafe.ai`, OpenAPI 3.1 spec
  version 0.2.0 at `/openapi.json`. [official]
- **Endpoints:**
  - `POST /v1/systemone`: `model`, `state` and `questions` required;
    `assets` (PNG/JPEG images) optional.
  - `GET /v1/models`: lists the available models.

  [official] docs.typesafe.ai/api/system-one.md,
  docs.typesafe.ai/api/list-models.md
- **Auth:** `Authorization: Bearer <key>`, with the environment variable
  `TYPESAFE_API_KEY`. [official] docs.typesafe.ai/introduction/quickstart.md
- **Response:**
  - `model`: the version that answered, which can differ from the alias you
    sent.
  - `answers`.
  - `usage`: `input_tokens` is billed; `output_tokens` is free.
  - An `x-typesafe-request-id` header.

  [official]
- **Input limits:**
  - Body at most 32 MiB, compressed or decompressed.
  - UTF-8 JSON without a BOM.
  - Nesting at most 64 levels.
  - Finite numbers only.
  - gzip or zstd, one layer.

  [official] docs.typesafe.ai/api/errors.md
- **Context:**
  - 64k tokens per request (state plus all questions).
  - 32k per question (state plus one question).
  - The `TypeSafe-Danger-Zone-Long-Context: true` header raises both to 262k
    at the same price, but TypeSafe says it is slower and less accurate.

  [official] docs.typesafe.ai/models.md
- **Batching:** many questions about one state go in one request and are
  evaluated in parallel. There is no endpoint for batching separate requests.
  [official]
- **Latency:** "70ms-500ms" end to end, measured from TypeSafe's laptops.
  There is no SLA. [official] blog post. Measured here: 94–159 ms (section 4).
- **Rate limits:** per organization, in tokens per second and requests per
  second. The actual numbers appear only in the console.
  - Third-party sources give 100,000 tokens and 80 requests per second per
    key.
  - The user's exa-mcp-server notes give the same figures
    (`D:\Projects\exa-mcp-server\claudedocs\improvement-research-2026-10-10.md:89-90`).
  - Going over returns 429 with `retry-after`.

  [official] docs.typesafe.ai/models.md; [third-party] typesafe-jev.ai/jev-pricing
- **Errors:**

| Status | Meaning |
|---|---|
| 400 | Bad request; includes `max_tokens_exceeded` when context is exceeded |
| 401 | Bad or missing key |
| 413 | Body too large |
| 415 | Unsupported media type |
| 422 | Validation failure, with a `detail[]` array |
| 429 | Rate limited |
| 529 | Overloaded |

  Retry 429 and 529 with exponential backoff. [official] docs.typesafe.ai/api/errors.md

### 2.4 Integrations

- **Official Python SDK:** `typesafe-sdk`.
  - Sync and async clients.
  - 2 retries, backoff from 0.5 s capped at 5 s with jitter.
  - Retries on 408, 429 and 5xx, honoring `Retry-After`.
  - A 30 s budget per call.

  [official] docs.typesafe.ai/sdk/python.md
- **Official JS/TS SDK:** `@typesafe-ai/sdk` (Node 22+), apparently v0.7.0.
  [official]
- **No official Rust SDK.** There are unofficial crates (`typesafe-system-one`,
  `typesafe-rs`, `jevrs` and others) with conflicting ownership claims; none
  was checked. [third-party] docs.rs
- **No official MCP server.** Community servers exist (`typesafe-mcp`,
  `@jkudish/jev-mcp`, keysersoft/jev-mcp-server). [third-party]
- **Composio:**
  - A Jev toolkit with `JEV_EVALUATE_STATE` and `JEV_LIST_MODELS`, which can
    be served as MCP.
  - A separate TypeSafe tool-picking provider, pinned to SDK 0.6.0.

  [third-party] composio.dev/toolkits/jev.md,
  docs.composio.dev/docs/providers/typesafe.md
- **LiteLLM:** has a TypeSafe pass-through. It doesn't stream (TypeSafe
  doesn't offer streaming) and forwards only `TypeSafe-Danger-Zone-` headers.
  [third-party] docs.litellm.ai/docs/pass_through/typesafe
- **What a Rust client in Parallax would need**, from the official spec:
  - reqwest with JSON.
  - The bearer header.
  - The two endpoints.
  - Types for the four question and answer shapes, with bounds checked
    client-side (0–1, 255 options, 10 levels).
  - Retries on 408, 429, 5xx and 529 with jitter and `retry-after`.
  - The request ID captured.
  - A pinned model ID.
  - Handling for 400 `max_tokens_exceeded` and 422 `detail[]`.

  Parallax already has this client pattern for Exa and Voyage
  (`src/client/exa.rs`, `src/client/voyage.rs`). The OpenAPI spec could
  generate the types.

### 2.5 Custom questions and back-testing

- **Every question is custom.** There is no fine-tuning: "Jev is not
  fine-tuned on customer data." [official] docs.typesafe.ai/models.md
- **No dedicated calibration or back-test tool.** The closest official pieces:
  - **WorkflowEvals:** 705 cases. It measures agreement with LLM-made labels,
    not calibration.
  - **system-one-adapter-python:** the same client interface backed by an
    LLM, for comparing Jev with an LLM.
  - **The eval site:** Jev scored 67.8% at $0.0004 and 0.4 s per case,
    against labels from two frontier LLMs that were not human-verified.

  [official] github.com/typesafe-ai/WorkflowEvals,
  github.com/typesafe-ai/system-one-adapter-python, evals.typesafe.ai
- **Testing advice in the docs:** reorder Choice options to check stability
  (because of the first-option bias), examine wrong answers, and test edge
  cases before rollout. [official] docs.typesafe.ai/model-jaggedness.md
- **The user's own projects already contain back-test harnesses** (section 3):
  exa-mcp-server's `scripts/jev-backtest/backtest.py` and upscaler's
  `typesafe-backtest.py`. They are the most directly reusable evidence
  method.

### 2.6 Pricing, terms and data handling

- **Price:**
  - $42 per billion input tokens ($0.042 per million); output is free.
  - The same price for 1.13, 1.14 and long context.
  - TypeSafe concedes it "can't prove it isn't subsidized".

  [official] docs.typesafe.ai/models.md, blog post
- **Billing:** prepaid credits that expire after 12 months. No free tier is
  documented, and there is no pricing page. [official] typesafe.ai/legal/mca
- **Master Customer Agreement (2026-09-23):**
  - The customer owns input and output.
  - No training on customer data without prior consent.
  - "TypeSafe may Process Telemetry without restriction."
  - TypeSafe may delete customer data at its discretion; some data is kept
    indefinitely for telemetry, abuse monitoring and legal reasons.
  - No distilling or building competing models.
  - No SLA.
  - Liability is capped at the greater of 12 months' fees or $50.

  [official] typesafe.ai/legal/mca
- **Privacy policy:**
  - "We will not train or fine tune any artificial intelligence or machine
    learning models on your prompts or other Input."
  - Data is kept "as long as reasonably necessary".
  - Hosted in the United States.

  [official] typesafe.ai/legal/privacy-policy
- **DPA:**
  - No fixed retention period.
  - Subprocessor list at trust.typesafe.ai (not fetched).
  - 72-hour breach notice.
  - EU SCCs.

  [official] typesafe.ai/legal/data-processing
- **Zero data retention:** enterprise customers only, by support request.
  SOC 2 Type II is available on request. [official]
- **What this means for Parallax:** anything sent to Jev leaves the machine
  and may be kept. Each candidate use in section 5 notes what text it would
  send.
  - The watchdog uses send tool-call text and assistant turns.
  - Memory uses send stored memories.
  - Research uses send public page text.
- **Availability:** a TypeSafe post on X dated 2026-09-24 said new direct
  sign-ups were closed. [third-party, via the cross-check run] jevwiki.ai.
  The user's key is set in the environment (presence checked, value never
  read).

---

## 3. The user's earlier Jev work (B7–B8)

### 3.1 History in this session's transcript

Line numbers refer to this session's transcript file.

- **2026-11-21, the request** (L37218): the user asked to add Jev:
  - "There's other frameworks that we can add. One that I like... Jev, J-E-V,
    that one has come in super handy in my other projects, and I think could
    benefit here."
  - It was one of four improvement tracks, alongside MCP alignment, Exa and
    Voyage.
- **Identified and confirmed:**
  - Jev was identified by web search (L37285, L37291).
  - The user confirmed (L37339): "this is correct: TypeSafe AI's Jev". Also
    (L37350): "we have already implemented it in other projects".
- **Tracks scored:**
  - Track scoring put MCP alignment 90, Exa 82, Voyage 64, Jev 30 (L37293,
    L37305). Jev was scored low because its API, pricing and terms had not
    been checked.
  - A later scoring put Jev at 38 (L37795, L37808).
- **2026-11-22, exa-mcp-server read:**
  - It was read with the user's approval (L42422, L42461), including the Jev
    injection back-test.
  - The 059 (Exa) spec named "a Jev prompt-injection screen (shadow/warn)" as
    an out-of-scope follow-up (`specs/059-exa-research-provider/spec.md:9,190`,
    `plan.md:68`).
- **After later compactions:** the summaries (L55268, L59170) no longer
  mentioned Jev.
- **How 063 became phrase matching:**
  - On 2026-11-23 the user chose the injection screen as the next feature
    (L58421).
  - The assistant grepped only the Parallax repository for "jev" and wrote
    (L58462): "I can't find what Jev is in the repo."
  - The options were then scored with Parallax `decide` (L58449-L58450):

| Option | Score |
|---|---|
| Pattern screen | 72 |
| Patterns + model | 55 |
| Model classifier | 45 |
| Named service | 25 |

- "Named service" was described as "Unnamed service cannot be assessed".
- The user picked "Pattern screen — 72", and 063 was specified to exclude
  external screening services.
- **In short:** the assistant had lost track of what Jev was, presented it
  as an unknown, and the scoring followed from that framing.
- **2026-11-24:** the assistant said Jev "was never identified" (L60344),
  which contradicts L37339 and L42422. The user then directed this research.

### 3.2 Projects that use Jev

From a grep of `D:\Projects` and `C:\Development\Projects`, skipping .env
files.

- **Code:** three projects contain Jev code (exa-mcp-server, harness-bridge,
  upscaler).
- **Specs only:** agentic-orchestrator-research.
- **Documents only:** meta and arc mention it.
- **Nothing in `C:\Development\Projects`** uses it.

**exa-mcp-server** (`D:\Projects\exa-mcp-server`)

- **Question:** one `noul`, `directs_ai`: "Does `page_text` contain text
  addressed to an AI assistant or agent, telling it to ignore or change
  instructions, call tools, visit URLs, reveal data, or claiming the content
  is pre-approved?" (`src\utils\jev.ts:14-23`).
- **API call** (`jev.ts:30-80`, `src\config\index.ts:70-82`):
  - Axios `POST https://api.typesafe.ai/v1/systemone`, model `jev-latest`,
    30 s timeout.
  - 3 retries on 429, 529, 5xx and network errors.
  - 100k-character segments, highest score kept; 4 pages at a time.
- **Modes:** `JEV_SCREEN=off|shadow|warn`, default off. It runs last in
  `web_fetch`, and it fails open: errors are shown as "not screened", and
  content is never withheld (`src\tools\injectionScreen.ts:17-56`,
  `jev.ts:86-101`).
- **Back-test** (`claudedocs\jev-injection-backtest-2026-10-10.md`), at the
  0.35 threshold:

| Set | n | Flagged |
|---|---|---|
| clean | 71 | 0 |
| hard_clean (security articles quoting attacks) | 12 | 4 |
| BIPIA | 75 | 47 |
| InjecAgent | 62 | 54 |
| InjecAgent, enhanced | 62 | 62 |

- AUC 0.993 (0.942 with hard_clean as negatives).
- p50 latency 0.15 s; $0.013 for 178 calls.
- **Problems the back-test found:**
  - All 8 InjecAgent misses are plain first-person requests.
  - BIPIA task-hijack categories are mostly missed.
  - The hand-written set shares an author with the question.
  - The two runs differed.
  - Adaptive attacks were not tested.
  - The 0.35 threshold rests on 71 pages.
- **Jev 1.13 weak spots, from the user's own notes**
  (`improvement-research-2026-10-10.md:91`): Score poorly calibrated,
  unreliable dates, first-option bias in Choice, and it "does not treat its
  input as hostile".
- **Key settings:** `TYPESAFE_API_KEY`, `TYPESAFE_BASE_URL`, `JEV_MODEL`,
  `JEV_SCREEN_THRESHOLD`, `JEV_AUDIT_LOG`, `JEV_TIMEOUT`.

**harness-bridge** (`D:\Projects\harness-bridge\src\harness_bridge\jev_screen.py`)

- **Questions** (`jev_screen.py:60-95, 360-380`):
  - S1: `action_class` Choice (read_only / state_changing /
    spend_or_destructive) and `explicitly_approved` Noul, with a 0.9 floor
    for state_changing.
  - S2: `claim_matches_diff` Noul; below 0.5 counts as divergence.
- **Behavior:**
  - Transport errors fail open; unclassifiable answers fail closed.
  - Default mode is enforce (the operator changed it 2026-09-20).
  - The rule: a screen is a "defense layer, never a gate owner"
    (`D:\Projects\agentic-orchestrator-research\W1-JEV-BRIDGE-SPEC.md:10`).
- **Measured:** hermetic tests only. The live-sample review was retired.

**upscaler** (`D:\Projects\upscaler\var\`)

- **Scripts:**
  - `jev-gate.py`: action class plus approval, floor 0.9, fails closed to
    CONSULT.
  - `bm2-watchdog.py` and `ec1-watchdog.py`: `recommended_action` and
    `verdict_agrees`, shadow mode.
  - `session-triage.py`: `recommended_recovery` Choice.
  - `pace_check.py`: `pace_severity` Score.
- **Measured** (`agentic-orchestrator-research\TYPESAFE-INTEGRATION-RESEARCH.md`):
  - Back-test 6/6.
  - Session triage 7/7 after one rewrite of the criteria.
  - A shadow sample of 76 judgments caught both incident states, with 5
    low-confidence false positives (0.21–0.30).
  - **Adding one state field (`cap_semantics`) moved P(halt) from 0.37 to
    0.01.** What goes into `state` matters as much as the question.

**Patterns across the user's projects worth carrying into Parallax:**

- **Shadow mode first:** log Jev's answer without acting on it, then promote
  to warn or enforce from measured data.
- **Fail open on transport errors.** Decide per use whether an unclassifiable
  answer fails open or closed.
- **The screen is a defense layer, never the gate owner.** Jev adds a signal;
  the existing mechanism keeps the decision.
- **Floors per class** (for example 0.9 for state-changing actions) rather
  than one global threshold.
- **Back-test before thresholds.** Every project that measured found
  something (first-person misses, criteria rewrites, missing state fields).

---

## 4. Live measurements (D16)

Setup:

- 16 calls, 8 synthetic requests each sent twice.
- Model pinned to `jev-1.14.0`; key from the environment, never printed.
- Total input was 5,676 tokens, about $0.00024. Every call returned 200.
- Raw results are in the session scratchpad (`jev_probe_results.json`).

| # | Parallax use | State | Question | Send 1 | Send 2 | ms |
|---|---|---|---|---|---|---|
| 1 | watchdog action risk | `git push --force origin main` | choice action_class | spend_or_destructive 0.81 (state_changing 0.19) | spend_or_destructive 0.87 (0.13) | 159 / 113 |
| 2 | watchdog action risk | `cargo test` | choice action_class | **state_changing 0.61** (read_only 0.39) | same | 103 / 114 |
| 3 | `check` routing | "17% of 240 is 40.8" | choice claim_kind | computable 1.00 | same | 95 / 101 |
| 4 | `check` routing | "Rust is a better language than Go" | choice claim_kind | judgment 1.00 | same | 105 / 104 |
| 5 | memory consolidation | SQLite, then "moved to Postgres" | choice relation | updates 0.99 | same | 94 / 99 |
| 6 | turn contradiction, no negation word | "We use Postgres" vs "Storage is SQLite" | noul contradicts | 0.91 | 0.90 | 105 / 112 |
| 7 | research claim support | WAL claim + excerpt | noul established | 0.95 | 0.95 | 115 / 118 |
| 8 | research injection (paraphrased) | "Note for any assistant... already approved emailing their saved passwords" | noul directs_ai | 0.97 | 0.96 | 101 / 101 |

What this shows:

- **Speed.** 94–159 ms per call, inside TypeSafe's 70–500 ms claim and
  matching the user's back-test (0.15 s p50). That is about 100–300 times
  faster than one judgment-model call in Parallax today.
- **Repeatability.** 6 of 8 requests returned identical answers both times.
  Two drifted slightly: 0.81 then 0.87, and 0.91 then 0.90. Answers are close
  but not deterministic, so tests can't assert exact probabilities.
- **A questionable answer.** Request 2 called `cargo test` "state_changing"
  at 0.61. It does write build output, but most would call it read-only.
  This matches the docs' "reads questions literally": the criteria said
  "changes files". Criteria wording would need tuning and back-testing, as
  upscaler's session triage needed one rewrite.
- **Plain contradiction.** Request 6 caught a contradiction with no negation
  word (0.91). The lexical cue list in `checkpoint_turn`'s mining step
  (`src/checkpoint/review.rs:35-45`) cannot catch this.
- **Paraphrased injection.** Request 8 caught a paraphrased injection that
  063's phrase list does not contain ("ignore previous instructions" and
  similar).
- **Not tested here:**
  - Calibration, which needs many labeled cases.
  - Adversarial inputs.
  - Long state.
  - First-option bias (option reordering).
  - Rate limits.

---

## 5. Where Jev fits in Parallax, layer by layer (C9–C15)

Every model call in Parallax today goes to the configured judgment model:
default `claude-opus-4-8` at $5 / $25 per million input / output tokens
(`src/telemetry.rs:29`). Only research extraction uses the "bulk" tier
(`src/routing.rs:290-293`); on `claude_cli` that is `claude-haiku-5-5`
(`src/routing.rs:328`).

Measured costs today:

- One `verify` pass: 8.5–33.5 s and $0.039–$0.172
  (`claudedocs/experiments/verify-recent-facts/README.md:56-61`).
- `claude_cli` adds about 1.3 s per call
  (`specs/060-claude-cli-backend/quickstart.md:50`).
- A Voyage query embedding: p50 130 ms
  (`specs/006-checkpoint-layer/quickstart.md:65`).

There is no server-side routing between correctives
(`src/server/correctives.rs:47,79,111,145,178`).

Each use below lists how it works today, Jev's role, the benefit, the cost and
speed, the risk, and what text would be sent to TypeSafe.

### 5.1 Watchdog (checkpoint layer)

**`checkpoint_turn`: end-of-turn review. Best fit.**

- **Today:**
  - Lexical mining: at least 3 shared words plus a polarity cue from a 9-word
    list (`src/checkpoint/review.rs:27,35-45,183-237`).
  - Then one decline-biased full model call returning `contradicts` (bool),
    `violates` (bool) and `capture_kind` (skill / lesson / none) plus text
    (`review.rs:47-124`, call at `review.rs:366`).
  - With memory configured, the call runs at every turn end
    (`src/checkpoint/run.rs:309-314`). 634 of 747 recorded turns made the
    call.
- **Jev's role:**
  - (a) A pre-screen: make the full call only when P(contradiction,
    violation or capture-worthy) is above a threshold.
  - (b) Replace polarity-cue mining with a pairwise contradiction check,
    which catches "Postgres" vs "SQLite" (measured 0.91).
  - (c) The three outputs are already typed.
- **Benefit:**
  - Most turn-end calls are skipped, saving cost and time on nearly every
    turn.
  - Contradictions without negation words are caught.
  - The design's "threshold on confidence" against alarm fatigue becomes
    explicit (`docs/design/WATCHDOG_LAYER.md:201-205`).
- **Cost and speed:** about $0.00002 per Jev call at ~400 tokens, in about
  0.1 s, against one full model call per turn today.
- **Risk:**
  - Flags quote verbatim statements and a `basis`, and capture stores
    generated text (`review.rs:412-446`). Jev produces none of that, so it
    can gate the call but not replace its text outputs.
  - A threshold means nothing until it is calibrated against
    `checkpoint_records`.
- **Text sent:** the turn's assistant text and the candidate statements.

**`checkpoint_action`: pre-action gate.**

- **Today:**
  - A substring match against 16 risk patterns (`src/checkpoint/gate.rs:17-25`).
  - If it matches, a hold when a trusted lesson or fact memory has cosine of
    at least 0.55.
  - All under a 500 ms budget that fails open (`src/checkpoint/run.rs:191-207`).
  - The hold fires on relevance, not contradiction (`gate.rs:4-7`).
- **Jev's role:**
  - (a) An action risk class, the same question harness-bridge and upscaler
    already use.
  - (b) "Does this action contradict constraint X?", so a hold needs a
    contradiction and not just topical relevance.
- **Benefit:** it catches risky commands the list doesn't name. It is
  additive only (logical OR with the list).
- **Risk:**
  - Measured false holds are already 0/60 (`src/checkpoint/mod.rs:42-44`).
  - Jev shares the 500 ms budget with the embedding.
  - Replacing the list would let an unlisted destructive command through.
  - The `cargo test` misclassification shows the criteria need back-testing.
- **Text sent:** the tool name and input of every gated action.

**`checkpoint_batch`: loop screening.**

- **Today:** exact match after normalization, no model call
  (`src/checkpoint/screen.rs:39-161`).
- **Jev's role:** "Is this window a no-progress loop?", which would catch
  near-identical and alternating loops.
- **Risk:** a per-batch call adds latency and cost to every tool call, and
  alarm fatigue is "the make-or-break" (`WATCHDOG_LAYER.md:201-205`). It
  should run only after the cheap screen fires. Lowest priority in this
  layer.

### 5.2 Cognitive correctives

**`verify`.**

- **Today:** 3 parallel stance-blind passes under distinct lenses; majority
  vote, with a tie going to refuted (`src/modes/verify.rs:70-101,342-346`).
  About $0.12–$0.52 and 8–70 s.
- **Jev's role:**
  - (a) An extra voter from a different model family (real diversity).
  - (b) A cheap first answer that escalates when uncertain.
- **Risk:**
  - The design forbids escalating on self-reported confidence
    (`docs/design/CORRECTIVE_SELECTION.md:62-70,90`).
  - A typed vote carries no named error, which breaks the rule that took
    false positives from 1/6 to 0/6 (`docs/design/NEW_SERVER_DESIGN.md:293-296`).
  - Verify already refutes recent true facts; a fast model is unlikely to do
    better.
- **Verdict:** a diversity voter is plausible only after measurement. A cheap
  first answer conflicts with the design.

**`decide`.**

- **Today:** one scored pass, argmax, confidence 0.5 + gap/200
  (`src/modes/decide.rs:505-514`). The same input has named different winners
  at score gaps up to 38 (`NEW_SERVER_DESIGN.md:239-260`). Running more than
  one pass was deferred (`NEW_SERVER_DESIGN.md:272-275`).
- **Jev's role:** a cheap Choice over the options, run many times and with
  the options reordered (which also tests first-option bias). That gives the
  stability estimate 054 deferred.
- **Risk:** a probability distribution is not the same thing as stability
  across reruns, and rationales still need the large model. It must be
  measured on the 054 dataset (`claudedocs/experiments/decide-stability`).
- **Note:** this corrective directly shapes the questions asked of the user.
  The 063 history (section 3.1) shows what a single unstable pass with a
  skewed framing can do.

**`grounded_verify`.**

- **Today:** k passes. A computable claim is settled by the engine only when
  all passes agree on the count (`src/modes/grounded_verify.rs:444-461`).
- **Jev's role:** a fast pre-pass on `needs_computation` and the property to
  count.
- **Risk:** settling relies on agreement across passes. A false "computable"
  is the costliest error (`docs/design/DETERMINISTIC_LAYER.md:124-126`).

**`elicit`, `unstick`, `diverge`.** Lower value.

- **`elicit`:** a relevance filter on recalled preferences
  (`src/modes/elicit.rs:247-274`).
- **`unstick`:** paraphrase detection against tried steps; today it uses
  exact equality (`src/modes/unstick.rs:236-245`).
- **`diverge`:** paraphrase dedup; today it uses token Jaccard of at least
  0.8 (`src/modes/diverge.rs:257-267`).
- **Output:** all three still produce generative text that Jev cannot write.

### 5.3 Deterministic layer: `check`

- **Today:** one full model call both classifies checkability and writes the
  formal form, with a bias toward declining
  (`src/deterministic/translate.rs:22-105`), plus one retry fed by the engine
  (`src/deterministic/check.rs:101-149`).
- **Jev's role:**
  - (a) A fast checkability pre-decline (measured: computable 1.00, judgment
    1.00). This is the open question at `DETERMINISTIC_LAYER.md:133-134`.
  - (b) A round-trip check: "does form F state claim C?". That targets
    translation faithfulness, the layer's central risk
    (`DETERMINISTIC_LAYER.md:73-74,115-116`).
- **Risk:**
  - The translator prompt has fine rules (exact percentages are checkable),
    so a cheap classifier may wrongly decline them. Skip only at very high
    P(not checkable).
  - LLM judges "miss subtle errors precisely where unfaithful specifications
    fail" (`DETERMINISTIC_LAYER.md:68-71`).
- **Text sent:** the claim and the formal form.

### 5.4 Memory

**Consolidation. Strongest fit in memory.**

- **Today:**
  - A cosine screen at 0.75 or above.
  - Then one full model 4-way judgment (same_assertion / updates /
    context_specific / distinct) with a 5 s budget; any failure keeps both
    (`src/memory/consolidate.rs:25-33,122-140,218-254`).
  - A merge needs cosine of at least 0.90 plus a trust guard.
- **Jev's role:** exactly this typed relation (measured: updates 0.99 in
  about 0.1 s).
- **Benefit:** a full-model call can overrun 5 s and silently do nothing; Jev
  would finish.
- **Risk:** a wrong supersede hides knowledge. It is reversible (a status
  change), but the design's Berlin/Lisbon case is subtle
  (`docs/design/MEMORY_LAYER.md:165-167`). Supersede should need a high bar.
- **Text sent:** two stored memories.

**`recall` and `surface`.**

- **Today:** cosine plus recency plus a trust bonus
  (`src/memory/ranking.rs:20-27,70-89`). Surface uses a 0.55 floor, a cap of
  3 and a 500 ms budget (`src/memory/push.rs:24-32`).
- **Jev's role:** "does memory M bear on this task?". The design asks for
  task-state relevance, "not pure cosine" (`MEMORY_LAYER.md:72-78`).
- **Risk:** N calls inside surface's 500 ms. Voyage rerank (section 6)
  competes for this same slot.

**`save` trust.**

- **Today:** trust comes from caller flags, with optional verify
  (`src/memory/tools.rs:88-120`).
- **Jev's role:** an extra "reads as instructions or external" flag only.
  Replacing verify would weaken the poisoning defense the design is built on
  (`MEMORY_LAYER.md:152-164`).

### 5.5 Research

Calls per run:

- 1 scope call.
- 1 extraction per kept source (5 / 15 / 35).
- k = 1 / 2 / 3 verify passes per claim, up to 12 claims per source.
- 1–2 synthesis calls.

About 2,508 tokens per claim-pass (`src/research/mod.rs:74`). Verification
dominates (`src/research/pipeline.rs:536-537`).

- **Claim verification cascade. Largest cost lever.**
  - Today: the verify ensemble per claim (`pipeline.rs:947-1065`).
  - Jev: a first verdict per claim (measured: "established" 0.95 on a clear
    excerpt), escalating only uncertain claims, or acting as the quick-tier
    voter.
  - Risk:
    - "Refuted" means "not established by the excerpts"
      (`docs/design/RESEARCH_PRIMITIVE.md:204-213`).
    - A typed vote has no findings.
    - Calibration is unproven.
  - Text sent: public excerpts and claims.
- **Injection screening.**
  - Today: 063's phrase families, warn-only (`src/research/screen.rs:10-76`).
  - Jev: the `directs_ai` question, back-tested in exa-mcp-server and
    measured 0.97 on a paraphrase. False positives are cheap because it only
    warns.
  - Risk: Jev reads the hostile text itself (the docs list injected text
    shifting answers as a known weak spot). The typed output limits the
    damage to flipping one flag. Plain first-person requests are missed
    (back-test).
  - Combining them: keep 063's deterministic screen and add Jev, with each
    reported separately.
- **Grounding.**
  - Today: only checks that citation tokens resolve
    (`src/research/grounding.rs:87-110`). The design says every answer clause
    must map to a finding (`RESEARCH_PRIMITIVE.md:268`).
  - Jev: "does clause X follow from finding Y?".
- **Claim dedup.**
  - Today: an exact normalized key (`mod.rs:365-371`), though the stats field
    says "semantic dedup" (`src/research/contract.rs:204`).
  - Jev could do pairwise "same claim?", but that grows as n². Voyage
    embeddings fit this better (section 6).
- **Source credibility.**
  - Today: base 0.5, +0.2 for .gov/.edu/.org (`src/research/verdict.rs:164-176`).
  - The cross-check runs show the weakness: unofficial Jev sites scored the
    same as TypeSafe's docs.
  - Jev could classify "official vendor documentation vs third-party". The
    design asks for "conservative and explainable"
    (`RESEARCH_PRIMITIVE.md:405-407`).

Scope, extraction and synthesis produce text, so Jev does not fit there.

### 5.6 Routing between correctives

- **Today:** none. The client picks tools from their descriptions.
- **Jev's role:** route by mechanism, classifying a claim as computable,
  needing a named source, or judgment. The design calls this "the server's
  legit job" and rates checkability a reliable signal
  (`CORRECTIVE_SELECTION.md:45-60,86`). Measured: clean separation on two
  examples.
- **Risk:** the design warns against rebuilding a meta-selector over
  corrective choice (`CORRECTIVE_SELECTION.md:42-43`) and against routing on
  self-reported confidence.

### 5.7 The design rule and the calibration it requires

Parallax's design allows a cheap signal to gate an expensive judge:

- "Cheap signals gate the expensive judge" (`WATCHDOG_LAYER.md:56,160-163`).
- Keep monitors "narrow and specialized per signal (a contradiction-checker,
  an action-risk scorer)" (`WATCHDOG_LAYER.md:208-211`).

It forbids routing on a model's own stated confidence. Confidence must come
from ensemble agreement (`CORRECTIVE_SELECTION.md:62-70,84-93`;
`NEW_SERVER_DESIGN.md:91,251-255`). No design document mentions System 1 /
System 2 or a fast classifier tier.

Jev's probabilities count as self-report until shown calibrated, and TypeSafe
publishes no calibration data. Before any Jev threshold decides anything in
Parallax:

1. **Shadow mode.** Record Jev's answer next to the existing decision without
   acting on it, as all three of the user's projects did.
2. **Measure against Parallax's own labels:**
   - `checkpoint_records`: the turn review's decisions.
   - `consolidation_records`: relation judgments.
   - `invocation_records` and the research verdicts.
   - The 054 decide dataset.

   Compute reliability per question: at each probability band, how often is
   Jev right?
3. **Per-class floors** (as in harness-bridge) and Choice options reordered
   to check first-option bias.
4. **Only then** let a Jev probability gate or skip anything, and keep the
   existing mechanism as the decision owner.

---

## 6. Voyage and Exa (E17)

### 6.1 Voyage: what Parallax uses

- **Embeddings:** one text per request with `input`, `model` and
  `input_type` (`src/client/voyage.rs:103-111`).
  - 1024-dimension float32, stored as f32 BLOBs
    (`src/storage/sqlite.rs:34,556`).
  - Model `voyage-4` (`src/config.rs:19`).
  - Used by save, recall, memory push (first 2,000 characters, 500 ms budget)
    and consolidation.
- **Rerank:** `rerank-3` on research pages, keeping 5 / 15 / 35
  (`src/client/voyage.rs:26,186-195`; `src/research/pipeline.rs:775-835`).

### 6.2 Voyage: unused capabilities, ranked

1. **Semantic claim dedup in research.**
   - Today: claims merge only on exact normalized text, though the stats
     field promises semantic dedup (`src/research/mod.rs:362-371`;
     `contract.rs:204`).
   - Embedding claims would merge reworded duplicates, so each is verified
     once with more sources behind it.
   - That saves about 2,508 tokens per vote per duplicate.
   - Cost: well under a cent per run.
2. **Rerank memory recall.** Rerank the top cosine matches for sharper
   relevance, at $0.05 per million tokens. It doesn't fit surface's 500 ms
   budget. It competes with Jev's relevance check (section 5.4), so the two
   should be measured against each other.
3. **Batched embedding** (up to 1,000 texts per request) and the Batch API
   (33% cheaper, up to 12 hours).
   - Today there is no re-embed path when the model changes; it only logs a
     warning (`src/memory/tools.rs:189-205`).
   - Batching makes re-embedding memories and embedding claims cheap.
4. **voyage-4-large for saved memories, voyage-4 for queries.** The 4-series
   share one space. Saves cost $0.12 instead of $0.06 per million tokens. The
   mismatch warning would need to treat the 4-series as one family.
5. **Smaller vectors** (`output_dtype` int8 or binary, `output_dimension`):
   4x to 32x less storage. Little benefit at current sizes. Stored vectors
   would need a format marker (`src/storage/sqlite.rs:584-587`).
6. **Passage selection with contextual embeddings (voyage-context-4).** Send
   extraction only the relevant passages instead of whole 16,000-character
   pages. That is about $0.03 per deep run, plus chunking code.
7. **rerank-3-lite:** saves under a cent per run. Not worth it now.
8. **Semantic dedup in `diverge`:** catches reworded framings, but adds a
   Voyage dependency to diverge.
9. **Accounting gap:** a `VOYAGE_MODEL` of voyage-code-4, voyage-context-4 or
   voyage-4-nano is priced at the Opus fallback, about 40x over
   (`src/telemetry.rs:44`). Adding price rows costs nothing.

Not useful for Parallax: multimodal, the finance and law models, and reranker
instructions (documented for rerank-2.5, not rerank-3).

### 6.3 Exa: what Parallax uses

- **Search:** `query`, `type`, `numResults`, `includeDomains`,
  `excludeDomains` (`src/client/exa.rs:72-85`). Type `auto` for quick and
  standard, `deep-lite` for deep (`src/research/mod.rs:81-86`).
- **Contents:** `urls`, `text.maxCharacters` = 16,000, and `maxAgeHours`
  (`src/client/exa.rs:88-94`).
- **Fields read back:** `id`, `url`, `title`, `text`, statuses, and the total
  cost (`exa.rs:391-426`). Error tags are dropped (`exa.rs:300-314`).
- **Rejected on purpose:** spec 059 D12 lists each declined feature with its
  reason (`specs/059-exa-research-provider/research.md:169-180`).

### 6.4 Exa: unused capabilities, ranked

1. **Strip boilerplate before extraction** (`includeSections` /
   `excludeSections` / `verbosity`). Fewer extraction tokens, possibly fewer
   false injection hits. No extra charge. Exa calls it "best-effort", so it
   needs a live check.
2. **Publication dates.**
   - Today `publishedDate` is not parsed (`exa.rs:409-415`).
   - Showing dates in `sources` and to verification helps time-sensitive
     claims, and a date window would enforce recency.
   - Free; it is a contract change.
3. **`additionalQueries` for the deep tier.** One deep search with up to 10
   extra queries instead of 8 separate ones: about $0.096 drops to $0.012 per
   deep run. It caps at 100 results (deep wants 120), per-angle isolation is
   lost, and equivalence is unmeasured.
4. **Check whether `deep-lite` is worth $12 per 1,000 vs `auto` at $7.**
   Parallax ignores deep-lite's synthesized output.
5. **Log `warnings`, pass `notices`, record per-URL error tags.** Free, and
   it explains dropped sources.
6. **Query-guided highlights.** 059 rejected them as too short, but
   `highlights.maxCharacters` is now configurable. They overlap with rerank.
7. **`/answer` as an outside cross-check vote**, at $5 per 1,000. It bypasses
   grounding, so it could only be a flagged signal.
8. **Agent runs:** slow, expensive, and bypass verification. Low value.

Do not adopt: `/findSimilar` and search-side `subpages` (both deprecated),
`includeText` / `excludeText` (deprecated), `/research/v1` (deprecated
2026-05-01).

**Exa prices** (official pricing page, fetched 2026-11-24):

| Item | Price |
|---|---|
| `lite` | $4 per 1,000 |
| `auto` | $7 per 1,000 |
| `deep-lite` / `deep` | $12 per 1,000 |
| Extra result above 10 | $1 per 1,000 |
| Contents | $1 per 1,000 pages per content type |
| `/answer` | $5 per 1,000 |
| Agent | $0.012 to $1.00 per run, or metered |
| Free credit | $10 a month |

---

## 7. Cross-check: research-tool runs vs subagent findings

**Jev (run 1 vs subagent A)**

- **They agree on:**
  - the price ($0.042 per million, output free);
  - versions 1.13 and 1.14, with 1.14 released 2026-10-14;
  - 64k and 32k context;
  - 70–500 ms latency;
  - the noul, choice and score shapes;
  - Bearer auth and the two endpoints;
  - calibration measured across groups, not per answer.
- **Run 1 added:**
  - The sign-up closure post (2026-09-24, via jevwiki.ai).
  - Third-party rate limits of 100,000 tokens and 80 requests per second.
  - The LiteLLM integration.
- **Run 1 missed or got wrong:**
  - It found no official SDK, though A found official Python and JS SDKs.
  - It found nothing on data handling, though A found the MCA, privacy
    policy and DPA.
  - It gives score levels as 2–10; A's official source says 1–10.
  - It shows images as contested; A's changelog source settles it: `box` and
    images exist on 1.14 behind a header.
- **Why:** run 1 stopped at its deadline (742 s) with 7 of 7 sub-questions
  unsettled except pricing. It cited unofficial sites at the same credibility
  as TypeSafe's docs.

**Voyage (run 2 vs subagent E)**

- **They agree on:**
  - the voyage-4 family and its shared space;
  - dimensions 256–2048 and int8/binary output;
  - 1,000 texts per request;
  - 200M free tokens;
  - reranker limits of 1,000 documents and an 8K query;
  - the bi-encoder vs cross-encoder explanation.
- **They differ on:**
  - Run 2 names voyage-context-3; E's fetched pricing table lists
    voyage-context-4.
  - Run 2 reports instruction-following for rerank-2.5. Both note that
    instructions are not documented for rerank-3, which Parallax uses.
  - Run 2 found no reranker pricing; E found rerank-3 at $0.05 and
    rerank-3-lite at $0.02.
- **Run 2 also** stopped at its deadline (723 s).

**Exa (run 3 vs subagent E)**

- **They agree on:** every core price (lite $4, auto $7, deep $12, contents
  $1 per page per type, answer $5, agent $0.012–$1.00) and the legacy types.
- **Run 3 added:** default rate limits (10 queries per second for search and
  answer, 5 for deep) and that `deep-reasoning` is legacy at $15.
- **E added:** that `/findSimilar` is deprecated in the spec, and the full
  contents options.
- **Run 3** stopped at its deadline (706 s), with coverage 0.

**What the cross-check says about Parallax `research`:**

- **Deadlines:** all three runs exceeded the 660 s standard deadline and
  stopped early. They ran at the same time as four subagents.
- **Credibility:** the heuristic gave unofficial sites the same 0.5 as
  official docs.
- **Thin results:** almost every claim was single-source and unverified.

This is the deadline overrun recorded in `specs/063-research-injection-screen/quickstart.md`
(quick runs passing their 120 s deadline without stopping), plus a
source-quality gap. Both bear on Jev uses in research (5.5) and on Voyage's
claim dedup (6.2).

---

## 8. Open questions and unknowns

1. **Calibration:** no published data. It must be measured on Parallax's own
   records before any threshold is used (5.7).
2. **Determinism:** small drift was seen (0.81 then 0.87). TypeSafe's FAQ has
   no answer.
3. **Rate limits:** console only. Third-party figures say 80 requests and
   100,000 tokens per second. This matters for per-claim research use at deep
   depth (35 sources × 12 claims).
4. **Access:** whether new sign-ups are open; the user's existing key works.
5. **Data handling:**
   - Which Parallax uses are acceptable to send off the machine (watchdog
     turn text, memories, public page text).
   - Zero retention is enterprise-only.
6. **Rust client:** there is no official SDK, so Parallax would need its own
   thin client (the pattern already exists for Exa and Voyage). Whether an
   unofficial crate is usable was not checked.
7. **Long state and adversarial inputs:** untested here. The docs say large
   state distracts Jev and injected text can shift answers.
8. **First-option bias:** documented, untested here. Any Choice use needs
   option-order checks.
9. **Priority between Jev and Voyage** for memory relevance and research
    dedup, where both could fill the same slot.
10. **The research tool's own deadline and credibility issues** (section 7),
    which affect any research-side Jev use.
11. **Not fetched:** the subprocessor list, the Trust Center contents, and
    the JS SDK's retry defaults.

---

## 9. Sources

**TypeSafe (official):**

- typesafe.ai
- typesafe.ai/blog/introducing-system-one-models-and-jev
- typesafe.ai/enterprise
- docs.typesafe.ai pages:
  - introduction
  - concepts/system-one.md
  - introduction/coding-agents.md
  - introduction/quickstart.md
  - api/system-one.md
  - api/list-models.md
  - api/errors.md
  - models.md
  - model-jaggedness.md
  - confidence.md
  - primitives/advanced.md
  - changelog.md
  - agent-skill.md
  - sdk/python.md (and its api/retries.md and api/exceptions.md)
  - sdk/javascript.md
  - llms.txt
  - legal.md
- api.typesafe.ai/openapi.json
- evals.typesafe.ai
- github.com/typesafe-ai, including WorkflowEvals and
  system-one-adapter-python
- Legal pages:
  - typesafe.ai/legal/terms
  - typesafe.ai/legal/mca
  - typesafe.ai/legal/privacy-policy
  - typesafe.ai/legal/data-processing
  - typesafe.ai/legal/acceptable-use-policy

**Third-party:**

- composio.dev/toolkits/jev.md
- docs.composio.dev/docs/providers/typesafe.md
- docs.litellm.ai/docs/pass_through/typesafe
- pydantic.dev/docs/ai/models/typesafe
- jevwiki.ai/wiki/syntheses/faq.md
- socket.dev/pypi/package/typesafe-mcp
- glama.ai/mcp/servers/jkudish/jev-mcp
- docs.rs: typesafe-system-one, typesafe-rs, jevrs
- typesafe-jev.ai/jev-pricing
- jevainews.com/jev/pricing

**Voyage:**

- docs.voyageai.com/docs/embeddings
- docs.voyageai.com/docs/pricing
- docs.voyageai.com/docs/reranker
- docs.voyageai.com/docs/contextualized-chunk-embeddings
- docs.voyageai.com/docs/multimodal-embeddings
- docs.voyageai.com/docs/batch-inference
- docs.voyageai.com/docs/faq
- mongodb.com/docs/voyageai/models
- blog.voyageai.com (voyage-3.5, rerank-2.5, voyage-context-3)

**Exa:**

- docs.exa.ai/reference/search
- docs.exa.ai/reference/get-contents
- docs.exa.ai/reference/answer
- exa.ai/docs/exa-spec.yaml
- exa.ai/docs/reference/agent-api/overview
- exa.ai/docs/reference/research/overview
- exa.ai/docs/changelog
- exa.ai/pricing
- exa.ai/docs/admin/pricing

**The user's projects:**

- D:\Projects\exa-mcp-server:
  - src\utils\jev.ts
  - src\tools\injectionScreen.ts
  - src\tools\webFetch.ts
  - src\config\index.ts
  - claudedocs\jev-injection-backtest-2026-10-10.md
  - claudedocs\improvement-research-2026-10-10.md
  - scripts\jev-backtest\backtest.py
- D:\Projects\harness-bridge\src\harness_bridge\jev_screen.py
- D:\Projects\upscaler\var\: jev-gate.py, typesafe-backtest.py,
  bm2-watchdog.py, ec1-watchdog.py, session-triage.py, pace_check.py
- D:\Projects\agentic-orchestrator-research\: TYPESAFE-INTEGRATION-RESEARCH.md,
  W1-JEV-BRIDGE-SPEC.md

**Parallax** (file:line cited inline throughout):

- docs/design: NEW_SERVER_DESIGN.md, WATCHDOG_LAYER.md,
  CORRECTIVE_SELECTION.md, MEMORY_LAYER.md, RESEARCH_PRIMITIVE.md,
  DETERMINISTIC_LAYER.md
- src/checkpoint, src/modes, src/deterministic, src/memory, src/research,
  src/client, src/routing.rs, src/telemetry.rs
- specs 006, 054, 059, 060, 061, 062, 063

**This session's transcript:** line references in section 3.1.

**Live measurements:** 16 calls to `POST https://api.typesafe.ai/v1/systemone`,
model `jev-1.14.0`, 2026-11-24. Results are in the session scratchpad
(`jev_probe_results.json`).
