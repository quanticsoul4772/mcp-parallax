# Deep-dive — The Watchdog Layer

**Status:** Deep-dive / proposal, **amended 2026-06-12 (MCP reality)** — the
amendment below supersedes the architecture where they conflict; the purpose,
signal catalog, intervention model, and hard problems stand. **Parent:**
[`OFFLOAD_LANDSCAPE.md`](OFFLOAD_LANDSCAPE.md) §C (runtime monitoring) and §K
(safety). **One line:** *the model won't call a corrective when it's the one
failing — because it can't tell — so the watchdog runs beside it, watches for the
failures it's blind to, and fires the help unprompted.*

## 2026-06-12 amendment — MCP reality: the mechanism, re-grounded

This doc was written assuming the watchdog runs in-process with the model loop,
on an activity/event bus. Parallax is an external MCP stdio server; that
premise does not survive, and the protocol offers no substitute. Re-grounded
against the MCP 2025-11-25 spec, the 2026-07-28 release candidate, and Claude
Code's current hook system (web-verified 2026-06).

### What did not survive (verified)

- **The event-stream substrate.** The "activity bus we already built"
  ([`DASHBOARD.md`](DASHBOARD.md)) was the previous server's in-process bus. An
  MCP stdio server observes only calls made to itself. There is no protocol
  mechanism — current or planned — by which a server sees the calling model's
  output stream, its other tool calls, or its turns: `includeContext` (the one
  trajectory-context parameter sampling ever had) is deprecated
  ([SEP-2596](https://modelcontextprotocol.io/specification/draft/changelog)).
- **Server-initiated intervention.** Sampling — the one server-initiated
  completion mechanism — was never implemented by Claude Code
  ([claude-code#31893](https://github.com/anthropics/claude-code/issues/31893),
  closed not-planned 2026-03) and is deprecated in the 2026-07-28 spec RC
  (SEP-2577); the RC also forbids server requests outside an in-flight client
  request (SEP-2260). Standard server→client notifications are not surfaced to
  the model in Claude Code. The "async, non-blocking, intervening in
  milliseconds" channel has no transport.
- **Token-stream supervision.** No transport exposes generation in progress.
  Checkpoint is not the cheap option; it is the *only* option — and this doc
  already conceded checkpoint is "enough for most signals."

### What replaces it: harness hooks as sensor/actuator, this server as brain

Claude Code's hook system (~30 lifecycle events as of mid-2026,
[docs](https://code.claude.com/docs/en/hooks)) *is* the assumed event stream,
productized — delivered as per-event invocations at checkpoint boundaries
instead of an in-process bus. The concept map:

| Original concept | MCP-reality mechanism |
|---|---|
| Activity/event stream | Hook events — `PreToolUse`, `PostToolUse`/`PostToolBatch`, `Stop`, `UserPromptSubmit` — each carrying `transcript_path` (the full session trajectory on disk); `Stop` carries the model's final message text |
| Watchdog consumes the stream | The `mcp_tool` hook handler: the harness calls a Parallax checkpoint tool directly with the event payload — no shell shim |
| **Feedback** (passive, the default) | Hook response `decision: "block"` + reason, or `additionalContext` — model-visible flag-and-let-it-fix |
| **Gate / approve** | `PreToolUse` → `permissionDecision: "deny"/"ask"` + reason on a pending tool call |
| **Interrupt** (active) | Lost mid-stream; nearest is an `async` hook with `asyncRewake` — a slow background check that re-enters the loop after the fact |
| Never rewrites | Held by abstention: the hook surface *can* rewrite (`updatedInput`, `updatedToolOutput`); this layer never uses those fields |
| Independent context + budget | Unchanged — the server judges the bare trajectory blind, in its own context, against memory/world-state |
| Cheap signals gate the expensive judge | Unchanged and now load-bearing: hooks fire on every tool call, so the cheap path must be pure and fast (loop/repeat detection over the call sequence, entity diff, contradiction-vs-memory via the existing recall path); one constrained model hop only on a cheap-signal fire |
| Trace event per trigger | Unchanged — one invocation record per checkpoint; catch-rate vs noise measurable from day one |

Deliverable shape: hook configuration shipped alongside the server
(`integrations/claude-code/`; plugin packaging is a named deferral until the
hook plumbing is live-verified), **off by default** — installing the hooks is
the explicit opt-in this layer's authority requires — plus the checkpoint
tool(s) on the server. The gating model is deliberately
*catalog-resident-but-uninvoked*: the checkpoint tools stay in the catalog
like every corrective (their one new capability, a validated bounded
transcript read, is constrained in the reader, not env-gated), and the layer
is "off" because nothing fires the tools until the sensor plane is installed
— the same opt-in posture as Constitution VI, enforced at the harness instead
of an env var. The watchdog + memory pairing survives whole: memory holds
what should be true; the checkpoint checks the live trajectory against it.

### Named costs

- **Checkpoint granularity is the floor.** No mid-generation interrupt, and
  extended thinking is redacted from every external surface (hooks, transcript,
  OTel) — some failures leave no observable trace (hard problem 5, worse here).
- **The sensor plane is client-specific.** Claude Code and the Agent SDK expose
  these hooks; other MCP clients do not. The server stays portable; the
  watchdog plane does not.
- **`PreToolUse` sits in the tool-call critical path** — a hard latency budget
  on the cheap heuristics; the model-hop escalation belongs at `Stop`-time
  review, not per-call gating.
- First Parallax component shipped outside the server binary.

### Future upgrade path, not foundation

Claude Code **channels** (`claude/channel`,
[reference](https://code.claude.com/docs/en/channels-reference)): server-pushed
content injected into the model's context over plain stdio, plus a permission
relay where the server adjudicates the model's tool calls. That is the true
push channel this doc wanted — but it is a research preview behind an
Anthropic allowlist (custom channels need a development flag). Revisit when GA.

### Consequence for naming

"Watchdog" implied a concurrent supervisor. What survives MCP is a
**checkpoint layer**: the same failure-mode catalog, the same tiered
interventions, the same blind judging — triggered by the harness, never by the
model. The self-diagnosis dependency stays removed, which is the layer's whole
point; prompt-level rituals ("always call X after Y" in client instructions)
are documented-unreliable and are not a substitute for harness triggering.

---

*The original proposal follows, unamended, as the design record. Read its
architecture through the table above.*

## Why this is the answer to the deepest problem in the design

Every callable corrective ([`NEXT_REASONING_SERVER.md`](NEXT_REASONING_SERVER.md))
assumes the model **recognizes it needs help and calls the tool.** But the worst
failures — anchoring, sycophancy, drift, unfaithful reasoning — are *failures of the
frame*, invisible from inside (the model that's convinced it's right won't ask to be
challenged; the one rationalizing post-hoc believes its own explanation). A callable
tool cannot fix a failure the model can't perceive.

The watchdog removes the **self-diagnosis dependency.** It is automatic metacognition:
an independent process that watches the model's trajectory and surfaces problems the
model would never have flagged. This is exactly what runtime verification (RV) is
for — on-line assurance when the model can't self-assess, since LLMs are stochastic
and opaque, making pre-deployment guarantees impractical
([RvLLM](https://arxiv.org/pdf/2505.18585),
[Watchdogs & Oracles](https://arxiv.org/pdf/2511.14435)). It is the single
highest-leverage layer *precisely for the failures that matter most.*

## Architecture: an independent supervisor beside the loop

The grounded pattern is the **Large Supervisor Model** — a lightweight model running
*concurrently* with the main model, monitoring its output stream in real time and
issuing structured intervention signals (**abstain / feedback / interrupt**) **without
rewriting outputs**; it interrupts and notifies via a structured payload
([LSM](https://www.researchgate.net/publication/401283765)). Adapted here:

- **Independent context + budget.** The watchdog is *not anchored where the model is*
  — it judges the bare trajectory, blind to the model's self-justification. (The
  judge-bias contract from the value model applies: blind it, diversify it.)
- **It runs on the activity/event stream we already built.** The dashboard's
  in-memory activity bus ([`DASHBOARD.md`](DASHBOARD.md)) — Client→Registry→Mode→…
  events with tool/model/duration — *is* the substrate a watchdog consumes. Build the
  observability once; the dashboard *shows* the stream, the watchdog *acts* on it.
- **Async, non-blocking**, intervening in milliseconds–seconds; it never sits in the
  stdio critical path.

## What it watches for

| Signal | How detected | Failure it catches |
|---|---|---|
| **Self-contradiction** | NLI / consistency vs earlier turns & stored decisions; inter-sample checks ([SelfCheckGPT]) | drift, flip-flopping |
| **Sycophantic flip** | answer changed after pushback with *no new evidence* | sycophancy |
| **Drift from goal/constraints** | current step diverges from the stated goal / constraints / world-state | drift, lost-the-thread |
| **Hallucination / ungrounded claim** | claim not supported by provided context; **entity-level** diffs (dates/names) are most prone | unverified assertion |
| **Overclaiming** | high assertiveness + low calibrated confidence | miscalibration |
| **Unsafe / irreversible action** | risk score on a pending tool call | consequential mistakes |
| **Injection in input / tool stream** | pattern + provenance checks (§K) | adversarial compromise |

## How it detects (mechanics)

- **Checkpoint vs streaming.** Checkpoint (between steps/tool-calls) is cheaper and
  enough for most signals; reserve token-stream supervision for *interrupt-before-bad-output*.
- **Cheap signals gate the expensive judge.** Heuristics first — entity diff,
  contradiction-with-stored-decision, action risk score — and escalate to an LLM (or
  deterministic, §B) check only when a cheap signal fires. This is cost-aware routing
  (§I) applied internally; a critic on *every* token is unaffordable.
- **It must read state.** A watchdog with nothing to check against is useless — it
  needs the goal, the constraints, and prior decisions, which live in the memory /
  world-state layer ([`MEMORY_LAYER.md`](MEMORY_LAYER.md)). **Watchdog + memory are a
  pair:** memory holds what *should* be true; the watchdog checks the live trajectory
  against it.

## The intervention model

Borrow the LSM's three signals and add a gate, tiered by stakes:

- **Feedback (passive)** — annotate/flag: "you contradicted decision X." The model
  *can* correct once an external pass points the error out, even though it couldn't
  find it itself (the faithfulness/self-correction research) — so flag-and-let-it-fix
  is the right default.
- **Interrupt (active)** — stop the stream and notify with a structured signal before
  a bad output ships.
- **Gate / approve** — block a consequential or irreversible action pending
  verify-before-commit or human approval; tiered autonomy (low-risk autonomous,
  high-risk requires verification/human) ([guardrails + HITL](https://developers.openai.com/api/docs/guides/agents/guardrails-approvals)).

The watchdog **never rewrites** the model's output — it surfaces or gates; the model
fixes. That keeps it honest about authority and keeps the model in the loop.

## The watchdog is the meta-layer that fires the correctives

This is the connective insight. The correctives are the *what*; the watchdog is the
*when*. It **invokes Verify / abstention / reframe on the model's behalf** when it
detects the matching signal — closing the loop the model can't close itself ("I'm
convinced I'm right" → watchdog sees the contradiction → fires Challenge). Three ties
make it the spine of the whole server:

1. runs on the **observability** stream (dashboard infra);
2. reads the **memory/world-state** to know what to check against;
3. triggers the **correctives** + gates **actions** (§K).

## The hard problems (honest)

1. **Alarm fatigue — the make-or-break.** Too many flags and every flag is ignored.
   Mitigate: fire only on real signals, tier by risk, threshold on confidence, and
   emit a **trace event per trigger** so you can measure *catch rate vs noise* and
   whether the flag actually led to a correction. If precision is low, the layer is
   worse than nothing.
2. **Cost / latency.** Cheap heuristics gate the expensive judge; checkpoint over
   token-stream where possible; prefer deterministic checks (§B).
3. **The watchdog is still an LLM** — own blind spots and biases. Keep monitors
   *narrow and specialized per signal* (a contradiction-checker, an action-risk
   scorer) rather than one omniscient critic; use deterministic checks where the
   signal allows; *who watches the watchdog* is a real question.
4. **Authority calibration.** Flag vs interrupt vs require-human is policy; mis-set =
   runaway or constant interruption. Tier by risk domain.
5. **Limited observability.** It sees the trajectory, not the model's internals; some
   failures leave no external trace and slip through.

## Open questions

- Checkpoint vs streaming as the default?
- One general supervisor vs a panel of narrow specialized monitors?
- The precision/fatigue tuning — the same calibration problem the Verify spike hit,
  now in real time and higher-stakes.
- Trust/authority model: what may it block autonomously vs escalate?
- Keeping the watchdog itself honest (judge-bias, deterministic where possible).
