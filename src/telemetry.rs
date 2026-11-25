//! Invocation records — the observability foundation (US3).
//!
//! Every tool invocation produces exactly one [`InvocationRecord`] on every
//! exit path (FR-010). The prior server's metrics were never persisted; this
//! module is the lesson learned. Spans carry GenAI semantic-convention
//! attribute names so a later OTLP exporter is an output change, not an
//! instrumentation change.

use crate::error::Outcome;
use crate::traits::clock::TimeProvider;
use chrono::{DateTime, Utc};

/// Per-model pricing in USD per million tokens (input, output). Cost is an
/// estimate from token counts — invoice-exactness is explicitly not required
/// (spec assumption). Cached from the model catalog 2026-06-04.
const PRICING_PER_MTOK: &[(&str, f64, f64)] = &[
    // Claude 5 family (cached 2026-07-24). `claude-opus-5` happens to match
    // the fallback rate exactly — which is why `pricing_known` exists: without
    // it, "correct by lookup" and "correct by coincidence" look identical.
    // 060: the claude_cli backend's defaults (Anthropic first-party rates,
    // the claude-api model table cached 2026-11-03). Haiku 5.5 is $0.10/$0.50
    // for prompts up to 100K tokens and $0.50/$2.50 beyond; this flat table
    // carries the first band.
    ("claude-sonnet-5-5", 2.00, 10.00),
    ("claude-haiku-5-5", 0.10, 0.50),
    ("claude-fable-5", 10.00, 50.00),
    ("claude-opus-5", 5.00, 25.00),
    ("claude-sonnet-5", 2.00, 10.00),
    ("claude-opus-4-8", 5.00, 25.00),
    ("claude-opus-4-7", 5.00, 25.00),
    ("claude-opus-4-6", 5.00, 25.00),
    ("claude-sonnet-4-6", 3.00, 15.00),
    ("claude-haiku-4-5", 1.00, 5.00),
    // Voyage embeddings bill input only (cached from the Voyage pricing
    // page 2026-06-11).
    ("voyage-4-large", 0.12, 0.0),
    ("voyage-4", 0.06, 0.0),
    ("voyage-4-lite", 0.02, 0.0),
    // 061: research's reranker bills input only (Voyage pricing page,
    // 2026-11-23).
    ("rerank-3", 0.05, 0.0),
    // 064: Jev bills input only (TypeSafe pricing, 2026-11-24).
    ("jev-1.14.0", 0.042, 0.0),
];

/// Conservative fallback for unknown model ids (Opus-tier rates).
const FALLBACK_PRICING: (f64, f64) = (5.00, 25.00);

/// Whether this model has a price row, or was costed at the conservative
/// fallback (018 FR-012).
///
/// A `false` here means the figure is an over-estimate, not a measurement —
/// the distinction an operator needs to tell a known price from a guess.
#[must_use]
pub fn pricing_known(model: &str) -> bool {
    PRICING_PER_MTOK.iter().any(|(id, _, _)| *id == model)
}

/// Estimated cost in USD for a completed invocation.
#[must_use]
pub fn cost_usd(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (input_rate, output_rate) = PRICING_PER_MTOK
        .iter()
        .find(|(id, _, _)| *id == model)
        .map_or(FALLBACK_PRICING, |(_, i, o)| (*i, *o));
    #[allow(clippy::cast_precision_loss)] // token counts are far below 2^52
    {
        (input_tokens as f64).mul_add(
            input_rate / 1_000_000.0,
            (output_tokens as f64) * (output_rate / 1_000_000.0),
        )
    }
}

/// Tokens one model consumed within a single invocation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Input tokens billed.
    pub input_tokens: u64,
    /// Output tokens billed.
    pub output_tokens: u64,
}

/// Per-model token usage accumulated across one invocation (018 D3).
///
/// Once call sites can run on different models, an invocation's cost is only
/// computable per model — summing tokens first and dividing later is not
/// recoverable. Ordered so serialization is deterministic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelUsage {
    by_model: std::collections::BTreeMap<String, Usage>,
}

impl ModelUsage {
    /// Usage for an invocation that ran entirely on one model — the shape of
    /// eleven of the twelve call sites, and of every invocation when nothing
    /// is routed.
    #[must_use]
    pub fn single(model: &str, input_tokens: u64, output_tokens: u64) -> Self {
        let mut usage = Self::default();
        usage.add(model, input_tokens, output_tokens);
        usage
    }

    /// Accumulate one model call's usage.
    pub fn add(&mut self, model: &str, input_tokens: u64, output_tokens: u64) {
        let entry = self.by_model.entry(model.to_string()).or_default();
        entry.input_tokens = entry.input_tokens.saturating_add(input_tokens);
        entry.output_tokens = entry.output_tokens.saturating_add(output_tokens);
    }

    /// Merge another accumulator into this one.
    pub fn merge(&mut self, other: &Self) {
        for (model, usage) in &other.by_model {
            self.add(model, usage.input_tokens, usage.output_tokens);
        }
    }

    /// Summed (input, output) across every model — the values the record's
    /// existing token columns carry.
    #[must_use]
    pub fn totals(&self) -> (u64, u64) {
        self.model_entries()
            .fold((0, 0), |(input, output), (_, u)| {
                (
                    input.saturating_add(u.input_tokens),
                    output.saturating_add(u.output_tokens),
                )
            })
    }

    /// The model that consumed the most tokens — the attributed model (018 D5).
    ///
    /// Dominance is computed from **measured** tokens rather than estimated
    /// cost: [`cost_usd`] may fall back to Opus-tier rates for a model with no
    /// price row, so a cost-dominant rule could hand attribution to a model
    /// that merely lacks a price. Ties break lexicographically so the choice is
    /// deterministic. In the single-model case this is trivially the only
    /// model, which is what keeps unrouted records byte-identical.
    #[must_use]
    pub fn dominant(&self) -> Option<&str> {
        self.model_entries()
            .max_by(|(a_model, a), (b_model, b)| {
                let a_total = a.input_tokens.saturating_add(a.output_tokens);
                let b_total = b.input_tokens.saturating_add(b.output_tokens);
                // Reverse the id comparison so the *earliest* id wins a tie
                // (max_by keeps the last maximum).
                a_total.cmp(&b_total).then(b_model.cmp(a_model))
            })
            .map(|(model, _)| model.as_str())
    }

    /// Entries that are model calls: everything but [`NON_MODEL_USAGE`].
    fn model_entries(&self) -> impl Iterator<Item = (&String, &Usage)> {
        self.by_model
            .iter()
            .filter(|(model, _)| !NON_MODEL_USAGE.contains(&model.as_str()))
    }

    /// Participating models, sorted. Only models that actually ran (FR-015b).
    #[must_use]
    pub fn models(&self) -> Vec<String> {
        self.by_model.keys().cloned().collect()
    }

    /// Per-model usage, in model-id order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Usage)> {
        self.by_model.iter().map(|(model, u)| (model.as_str(), *u))
    }

    /// Whether any model consumed anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_model.is_empty()
    }

    /// Cost summed over models, each priced at its own rate (018 FR-008).
    #[must_use]
    pub fn cost_usd(&self) -> f64 {
        self.by_model
            .iter()
            .map(|(model, u)| cost_usd(model, u.input_tokens, u.output_tokens))
            .sum()
    }

    /// Whether any participating model priced off the fallback rather than a
    /// price row — the cost is then a conservative over-estimate (FR-012).
    #[must_use]
    pub fn cost_estimated(&self) -> bool {
        self.by_model.keys().any(|model| !pricing_known(model))
    }
}

/// One research run's Exa cost (059 FR-010).
///
/// The sum of what each Exa call reported, shared by the pipeline (which
/// records each call) and the record guard (which reads on every exit,
/// failures and cancellations included). A call is opened with
/// [`Self::begin`] and closed with [`Self::end`]; one never closed (the run
/// was cancelled mid-call) or closed without a cost may still have been
/// billed, so the total becomes unknown (059 review finding 4).
#[derive(Debug, Default)]
pub struct SearchCost {
    state: std::sync::Mutex<CostState>,
}

#[derive(Debug, Default)]
struct CostState {
    sum: f64,
    calls: u32,
    open: u32,
    unreported: bool,
}

impl SearchCost {
    /// An Exa call is starting.
    pub fn begin(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.open += 1;
        }
    }

    /// An Exa call finished, with the cost it reported — `None` when it
    /// failed or reported none.
    pub fn end(&self, cost: Option<f64>) {
        if let Ok(mut state) = self.state.lock() {
            state.open = state.open.saturating_sub(1);
            state.calls += 1;
            match cost {
                Some(usd) => state.sum += usd,
                None => state.unreported = true,
            }
        }
    }

    /// The run's total: `None` when no Exa call was made, when one is still
    /// open, or when any reported no cost — unknown, never understated as a
    /// partial sum.
    #[must_use]
    pub fn total(&self) -> Option<f64> {
        let state = self.state.lock().ok()?;
        (state.calls > 0 && state.open == 0 && !state.unreported).then_some(state.sum)
    }
}

/// Research's Jev rows for one invocation (064): pushed by the claim shadow,
/// taken once by the record guard on every exit, cancellation included.
#[derive(Debug, Default)]
pub struct JevShadow {
    rows: std::sync::Mutex<Vec<crate::storage::jev::JevRecord>>,
    /// Summed claim preparation time, in microseconds (064 review E).
    prep_micros: std::sync::atomic::AtomicU64,
    /// Claims whose preparation was timed.
    prep_count: std::sync::atomic::AtomicU64,
}

impl JevShadow {
    /// Keep one row.
    pub fn push(&self, row: crate::storage::jev::JevRecord) {
        // A panic elsewhere must not lose the measurement (064 review N).
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(row);
    }

    /// Add one claim's preparation time (redaction and request building),
    /// which runs before verify starts and so sits on the critical path.
    pub fn add_prep(&self, elapsed: std::time::Duration) {
        use std::sync::atomic::Ordering;
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.prep_micros.fetch_add(micros, Ordering::Relaxed);
        self.prep_count.fetch_add(1, Ordering::Relaxed);
    }

    /// The summed preparation time, in whole milliseconds.
    #[must_use]
    pub fn prep_ms(&self) -> u64 {
        self.prep_micros.load(std::sync::atomic::Ordering::Relaxed) / 1000
    }

    /// How many claims were timed.
    #[must_use]
    pub fn prep_count(&self) -> u64 {
        self.prep_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Every row kept so far; the accumulator is left empty.
    #[must_use]
    pub fn take(&self) -> Vec<crate::storage::jev::JevRecord> {
        std::mem::take(
            &mut *self
                .rows
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// One invocation's model cost as the `claude_cli` backend reported it.
///
/// 060 D11: the sum of the CLI's per-call API-price estimates, in millionths
/// of a dollar. A call without a figure makes the total unknown, never a
/// partial sum.
#[derive(Debug, Default)]
pub struct CliCost {
    state: std::sync::Mutex<CliCostState>,
}

#[derive(Debug, Default)]
struct CliCostState {
    micros: u64,
    calls: u32,
    unreported: bool,
}

impl CliCost {
    /// A model call completed, with the figure the CLI reported, if any.
    pub fn record(&self, micros: Option<u64>) {
        if let Ok(mut state) = self.state.lock() {
            state.calls += 1;
            match micros {
                Some(m) => state.micros = state.micros.saturating_add(m),
                None => state.unreported = true,
            }
        }
    }

    /// The invocation's total in dollars: `None` when no call recorded one or
    /// any call reported no figure.
    #[must_use]
    pub fn total(&self) -> Option<f64> {
        let state = self.state.lock().ok()?;
        #[allow(clippy::cast_precision_loss)] // micro-dollar sums are far below 2^52
        (state.calls > 0 && !state.unreported).then(|| state.micros as f64 / 1_000_000.0)
    }
}

/// Usage that is billed and priced but is not a model call (061 review
/// finding 6): kept in a record's per-model usage and cost, left out of its
/// token totals and its attributed model.
pub const NON_MODEL_USAGE: &[&str] = &["rerank-3"];

/// A run's non-model usage (061 review finding 3: the rerank), held apart
/// so the record of a run that fails afterwards still carries it.
#[derive(Debug, Default)]
pub struct NonModelUsage {
    usage: std::sync::Mutex<ModelUsage>,
}

impl NonModelUsage {
    /// Add input tokens billed under `model`.
    pub fn add(&self, model: &str, input_tokens: u64) {
        if let Ok(mut usage) = self.usage.lock() {
            usage.add(model, input_tokens, 0);
        }
    }

    /// What has been added so far.
    #[must_use]
    pub fn snapshot(&self) -> ModelUsage {
        self.usage.lock().map(|u| u.clone()).unwrap_or_default()
    }
}

/// One research run's page counts for its invocation record (061 FR-006):
/// pages whose text came back, and pages that reached extraction. Set once
/// by the pipeline; read by the record guard on every exit.
#[derive(Debug, Default)]
pub struct PageCounts {
    counts: std::sync::OnceLock<(u32, u32)>,
}

impl PageCounts {
    /// Record the run's counts (the first call wins).
    pub fn set(&self, fetched: u32, kept: u32) {
        let _already_set = self.counts.set((fetched, kept));
    }

    /// `(fetched, kept)`, or `None` when the run never reached extraction.
    #[must_use]
    pub fn get(&self) -> Option<(u32, u32)> {
        self.counts.get().copied()
    }
}

tokio::task_local! {
    /// The running invocation's [`CliCost`] meter (060 D11). The record guard
    /// scopes each invocation's work in one; the `claude_cli` client records
    /// into it. Model calls never run in spawned tasks, so every call of an
    /// invocation sees its meter.
    pub static CLI_COST: std::sync::Arc<CliCost>;
}

/// What a record's [`InvocationRecord::cost_usd`] rests on (060 D11). Both
/// are API-price estimates, never a charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostBasis {
    /// Parallax's price table applied to the reported tokens.
    PriceTable,
    /// The sum of the Claude Code CLI's own per-call estimates (`claude_cli`;
    /// the calls ran on the operator's subscription).
    CliEstimate,
}

impl CostBasis {
    /// The stable string form, as stored and exported.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PriceTable => "price_table",
            Self::CliEstimate => "cli_estimate",
        }
    }

    /// Parse the stored form.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "price_table" => Some(Self::PriceTable),
            "cli_estimate" => Some(Self::CliEstimate),
            _ => None,
        }
    }
}

/// The observability record of one tool call (data-model.md §5; contract:
/// `specs/001-core-layer/contracts/invocation-record.schema.json`).
#[derive(Debug, Clone)]
pub struct InvocationRecord {
    /// UUID v4 for this invocation.
    pub id: String,
    /// Per-process session UUID (one stdio connection per process).
    pub session_id: String,
    /// Mode id, e.g. `verify`.
    pub tool: String,
    /// Model id used for the passes.
    pub model: String,
    /// Input tokens summed across passes.
    pub input_tokens: u64,
    /// Output tokens summed across passes.
    pub output_tokens: u64,
    /// Estimated cost — the sum over participating models of that model's own
    /// tokens at that model's own rate (018 FR-008).
    pub cost_usd: f64,
    /// Every model that actually ran, sorted (018 FR-009, FR-015b). One entry
    /// for an unrouted invocation.
    pub models: Vec<String>,
    /// Per-model token usage behind [`Self::cost_usd`] (018 FR-009).
    pub usage_by_model: ModelUsage,
    /// True when a participating model had no price row and was costed at the
    /// conservative fallback — the figure is an over-estimate (018 FR-012).
    pub cost_estimated: bool,
    /// The research rigor tier this invocation ran under, for the one tool
    /// that has one (019). `None` for every other tool and for every record
    /// written before the column existed — the budget a run was held to is
    /// otherwise unrecoverable from the record.
    pub depth: Option<String>,
    /// The reasoning effort the **caller overrode** for this invocation (028).
    ///
    /// The override only — never the configured level. An invocation that fans
    /// out across call sites can use several *configured* efforts (a research
    /// run spans four independently routable sites), which one field cannot
    /// represent; an override is single-valued by construction. It is also the
    /// only part configuration does not already explain, and explaining spend
    /// that configuration cannot predict is why this field exists.
    ///
    /// `None` means no override was supplied — the configured layers applied,
    /// and the startup routing table says what they are.
    pub effort: Option<String>,
    /// The pass count the **caller overrode** for this invocation (028).
    ///
    /// Same criterion as [`Self::effort`]: spend configuration cannot predict
    /// must stay explainable. A caller asking for one pass against a
    /// configured three cuts an ensemble's model calls by two thirds, which
    /// moves spend further than any effort level does — recording one and not
    /// the other would leave the stated criterion half-applied.
    ///
    /// `None` means the configured count ran.
    pub passes: Option<u32>,
    /// The run's Exa cost in USD (059 FR-010): `research` only, and only when
    /// every Exa call reported a cost. Separate from [`Self::cost_usd`],
    /// which stays the summed model cost.
    pub search_cost_usd: Option<f64>,
    /// The Jev requests' cost in USD (064), summed over this invocation's
    /// Jev rows; `None` when none was made. Separate from [`Self::cost_usd`].
    /// Calls abandoned or cut off at their deadline may still be billed but
    /// carry no cost here; their rows say `cost unknown`.
    pub jev_cost_usd: Option<f64>,
    /// What [`Self::cost_usd`] rests on (060 D11); `None` only on records
    /// written before the column existed.
    pub cost_basis: Option<CostBasis>,
    /// Research only (061): pages whose text came back; `None` for other
    /// tools and runs that never reached extraction.
    pub pages_fetched: Option<u32>,
    /// Research only (061): pages that reached extraction.
    pub pages_kept: Option<u32>,
    /// Wall-clock latency via [`TimeProvider`].
    pub latency_ms: u64,
    /// Outcome classification.
    pub outcome: Outcome,
    /// RFC 3339 creation time via [`TimeProvider`].
    pub created_at: DateTime<Utc>,
}

impl InvocationRecord {
    /// Build the record at the single exit point of an invocation.
    /// `attributed` names the model when `usage` is empty — a cancelled or
    /// failed invocation records no tokens, so there is nothing to be dominant.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // the record IS this tuple; a builder adds nothing
    pub fn create(
        clock: &dyn TimeProvider,
        session_id: &str,
        tool: &str,
        attributed: &str,
        usage: &ModelUsage,
        outcome: Outcome,
        started_at: DateTime<Utc>,
    ) -> Self {
        let created_at = clock.now();
        let latency_ms =
            u64::try_from((created_at - started_at).num_milliseconds().max(0)).unwrap_or(u64::MAX);
        let (input_tokens, output_tokens) = usage.totals();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            tool: tool.to_string(),
            // 018 D5: the dominant model by measured tokens; with one model
            // that is trivially the only one, so unrouted records are
            // byte-identical to pre-018 (FR-009a).
            model: usage.dominant().unwrap_or(attributed).to_string(),
            input_tokens,
            output_tokens,
            // 018 FR-008: each model priced at its own rate, then summed. For
            // one model this is the same arithmetic as before.
            cost_usd: usage.cost_usd(),
            models: usage.models(),
            usage_by_model: usage.clone(),
            cost_estimated: usage.cost_estimated(),
            // Set by `with_depth` at the one call site that has a tier;
            // chainable rather than a parameter so the other five `create`
            // callers stay unchanged.
            depth: None,
            // Set by `with_effort` at the corrective call sites, chainable for
            // the same reason as `depth` (028).
            effort: None,
            passes: None,
            search_cost_usd: None,
            jev_cost_usd: None,
            cost_basis: Some(CostBasis::PriceTable),
            pages_fetched: None,
            pages_kept: None,
            latency_ms,
            outcome,
            created_at,
        }
    }

    /// Stamp the CLI's own cost figure onto the record (060 D11): with one,
    /// it becomes [`Self::cost_usd`], marked [`CostBasis::CliEstimate`];
    /// without one the price-table figure stands.
    ///
    /// The CLI figure covers the Claude models only; any other model in the
    /// invocation (an embedding model) keeps its price-table cost beside it
    /// (060 review finding 5).
    #[must_use]
    pub fn with_cli_cost(mut self, cli_cost_usd: Option<f64>) -> Self {
        if let Some(usd) = cli_cost_usd {
            self.cost_usd = usd + self.non_claude_cost();
            self.cost_basis = Some(CostBasis::CliEstimate);
        }
        self
    }

    /// The price-table cost of the models that did not run through the CLI.
    #[must_use]
    pub fn non_claude_cost(&self) -> f64 {
        self.usage_by_model
            .iter()
            .filter(|(model, _)| !model.starts_with("claude"))
            .map(|(model, u)| cost_usd(model, u.input_tokens, u.output_tokens))
            .sum()
    }

    /// Stamp a research run's page counts onto the record (061 FR-006).
    #[must_use]
    pub const fn with_pages(mut self, pages: Option<(u32, u32)>) -> Self {
        if let Some((fetched, kept)) = pages {
            self.pages_fetched = Some(fetched);
            self.pages_kept = Some(kept);
        }
        self
    }

    /// Use `id` as the record's id (064): the record guard makes it up front
    /// so the invocation's Jev rows can name it.
    #[must_use]
    pub fn with_id(mut self, id: String) -> Self {
        self.id = id;
        self
    }

    /// Stamp the invocation's Jev cost onto the record (064).
    #[must_use]
    pub const fn with_jev_cost(mut self, jev_cost_usd: Option<f64>) -> Self {
        self.jev_cost_usd = jev_cost_usd;
        self
    }

    /// Stamp the run's Exa cost onto the record (059 FR-010).
    #[must_use]
    pub const fn with_search_cost(mut self, search_cost_usd: Option<f64>) -> Self {
        self.search_cost_usd = search_cost_usd;
        self
    }

    /// Stamp the research rigor tier onto the record (019).
    #[must_use]
    pub fn with_depth(mut self, depth: Option<&str>) -> Self {
        self.depth = depth.map(ToString::to_string);
        self
    }

    /// Stamp the caller's effort **override** onto the record (028).
    ///
    /// Only the override. The configured level is deliberately not written:
    /// it is constant for the deployment, already printed in the startup
    /// routing table, and — for an invocation spanning several call sites —
    /// not a single value at all.
    #[must_use]
    pub fn with_effort(mut self, effort: Option<crate::routing::Effort>) -> Self {
        self.effort = effort.map(|e| e.as_str().to_string());
        self
    }

    /// Stamp the caller's pass-count **override** onto the record (028).
    ///
    /// Only the override, for the same reason as [`Self::with_effort`]: the
    /// configured count is constant and already known.
    #[must_use]
    pub const fn with_passes(mut self, passes: Option<u32>) -> Self {
        self.passes = passes;
        self
    }

    /// Publish the record to every observability sink at an invocation exit
    /// point: the structured tracing event (stderr) and the OTLP telemetry
    /// mirror. This single call is the structural guarantee behind the
    /// "one measurement, two sinks" contract (007 FR-009) — both surfaces
    /// derive from the same record value here, so an exit point cannot wire up
    /// one sink and silently forget the other. [`Self::emit`] is private for
    /// exactly this reason: `publish` is the only door. `caller` is the trace
    /// context the request carried, if any (058); the exported span starts
    /// under it.
    pub fn publish(&self, caller: Option<&crate::observability::CallerTrace>) {
        self.publish_with_jev(caller, None);
    }

    /// [`Self::publish`] with what Jev did during the invocation (064).
    pub fn publish_with_jev(
        &self,
        caller: Option<&crate::observability::CallerTrace>,
        jev: Option<&crate::observability::JevSpanSummary>,
    ) {
        self.emit();
        crate::observability::emit_invocation(self, caller, jev);
    }

    /// Emit the record as a structured tracing event with GenAI
    /// semantic-convention attribute names. Private: every exit point goes
    /// through [`Self::publish`] so tracing and telemetry cannot diverge.
    ///
    /// `cost.estimated` names the line self-describing: `false` means the
    /// figure is priced from the catalog, `true` that it is the conservative
    /// Opus-tier fallback — the same flag SQLite stores per record and OTel
    /// exports as `parallax.cost_estimated`.
    fn emit(&self) {
        tracing::info!(
            invocation.id = %self.id,
            session.id = %self.session_id,
            gen_ai.operation.name = %self.tool,
            gen_ai.request.model = %self.model,
            gen_ai.usage.input_tokens = self.input_tokens,
            gen_ai.usage.output_tokens = self.output_tokens,
            gen_ai.response.finish_reasons = %self.outcome.as_str(),
            cost.usd = self.cost_usd,
            cost.estimated = self.cost_estimated,
            latency.ms = self.latency_ms,
            "invocation recorded"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::clock::MockTimeProvider;

    /// 060 review finding 5: the CLI figure replaces only the Claude models'
    /// share; an embedding model's price-table cost stays in the total.
    #[test]
    fn the_cli_figure_keeps_the_embedding_cost() {
        let clock = fixed_clock();
        let mut usage = ModelUsage::single("claude-sonnet-5-5", 1_000, 100);
        usage.add("voyage-4", 1_000_000, 0); // $0.06 at the voyage-4 row
        let record = InvocationRecord::create(
            &clock,
            "s",
            "save",
            "claude-sonnet-5-5",
            &usage,
            Outcome::Success,
            clock.now(),
        )
        .with_cli_cost(Some(0.01));
        assert!((record.cost_usd - 0.07).abs() < 1e-9, "{}", record.cost_usd);
    }

    /// 061 T013 (D8): `rerank-3` is priced at $0.05 per million input tokens.
    #[test]
    fn rerank_3_is_priced() {
        assert!(pricing_known("rerank-3"));
        assert!((cost_usd("rerank-3", 1_000_000, 0) - 0.05).abs() < 1e-12);
    }

    /// 061 T013 (FR-006): the record carries the page counts when the run
    /// reported them, and none otherwise.
    #[test]
    fn the_record_carries_the_page_counts() {
        let clock = fixed_clock();
        let base = InvocationRecord::create(
            &clock,
            "s",
            "research",
            "claude-sonnet-5-5",
            &ModelUsage::default(),
            Outcome::Success,
            clock.now(),
        );
        assert_eq!((base.pages_fetched, base.pages_kept), (None, None));
        let counted = base.with_pages(Some((8, 5)));
        assert_eq!(
            (counted.pages_fetched, counted.pages_kept),
            (Some(8), Some(5))
        );
        let pages = PageCounts::default();
        assert_eq!(pages.get(), None);
        pages.set(25, 15);
        assert_eq!(pages.get(), Some((25, 15)));
    }

    /// 061 review finding 6: rerank tokens are usage and cost, not model
    /// tokens: they stay out of the token totals and can never be the
    /// attributed model.
    #[test]
    fn rerank_usage_is_priced_but_not_model_tokens() {
        let mut usage = ModelUsage::default();
        usage.add("claude-haiku-5-5", 100, 10);
        usage.add("rerank-3", 1_000_000, 0);
        assert_eq!(usage.totals(), (100, 10));
        assert_eq!(usage.dominant(), Some("claude-haiku-5-5"));
        assert!((usage.cost_usd() - cost_usd("claude-haiku-5-5", 100, 10) - 0.05).abs() < 1e-9);
        assert!(NON_MODEL_USAGE.contains(&crate::client::voyage::RERANK_MODEL));
        let side = NonModelUsage::default();
        assert!(side.snapshot().is_empty());
        side.add("rerank-3", 7);
        assert_eq!(side.snapshot().iter().count(), 1);
    }

    fn fixed_clock() -> MockTimeProvider {
        let fixed = DateTime::parse_from_rfc3339("2026-11-22T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut clock = MockTimeProvider::new();
        clock.expect_now().return_const(fixed);
        clock
    }

    /// 060 T019 (D11): the CLI's figures sum; one call without a figure makes
    /// the total unknown, never lower; no call means nothing to state.
    #[test]
    fn cli_cost_sums_reported_figures_and_an_unreported_one_makes_it_unknown() {
        let cost = CliCost::default();
        assert_eq!(cost.total(), None);
        cost.record(Some(1_000));
        cost.record(Some(2_500));
        let total = cost.total().unwrap();
        assert!((total - 0.0035).abs() < 1e-12, "{total}");
        cost.record(None);
        assert_eq!(cost.total(), None);
        cost.record(Some(1_000));
        assert_eq!(cost.total(), None, "unknown stays unknown");
    }

    /// 060 T019 (D11): with the CLI's figure the record's cost is that
    /// figure, marked `cli_estimate`; without it, the price-table figure,
    /// marked `price_table`.
    #[test]
    fn the_record_states_which_basis_its_cost_rests_on() {
        let clock = fixed_clock();
        let usage = ModelUsage::single("claude-sonnet-5-5", 1_000, 100);
        let base = InvocationRecord::create(
            &clock,
            "s",
            "verify",
            "claude-sonnet-5-5",
            &usage,
            Outcome::Success,
            clock.now(),
        );
        assert_eq!(base.cost_basis, Some(CostBasis::PriceTable));
        assert!((base.cost_usd - usage.cost_usd()).abs() < 1e-12);

        let cli = base.clone().with_cli_cost(Some(0.0123));
        assert_eq!(cli.cost_basis, Some(CostBasis::CliEstimate));
        assert!((cli.cost_usd - 0.0123).abs() < 1e-12);

        let none = base.with_cli_cost(None);
        assert_eq!(none.cost_basis, Some(CostBasis::PriceTable));
        assert_eq!(
            CostBasis::parse("cli_estimate"),
            Some(CostBasis::CliEstimate)
        );
        assert_eq!(CostBasis::CliEstimate.as_str(), "cli_estimate");
        assert_eq!(CostBasis::PriceTable.as_str(), "price_table");
    }

    /// 059 T015 (FR-010): the run's Exa cost is the sum of what each call
    /// reported; one call without a cost makes the total unknown, not lower;
    /// a run that made no Exa call has no cost to state.
    #[test]
    fn search_cost_sums_reported_costs_and_an_unreported_one_makes_it_unknown() {
        let cost = SearchCost::default();
        assert_eq!(cost.total(), None);
        cost.begin();
        cost.end(Some(0.007));
        cost.begin();
        cost.end(Some(0.005));
        let total = cost.total().unwrap();
        assert!((total - 0.012).abs() < 1e-9, "{total}");
        // A call that failed or reported nothing may still have been billed.
        cost.begin();
        cost.end(None);
        assert_eq!(cost.total(), None);
        cost.begin();
        cost.end(Some(0.007));
        assert_eq!(cost.total(), None, "unknown stays unknown");
    }

    /// 059 review finding 4: a call still in flight when the run ends (a
    /// cancellation drops it mid-await) may have been billed, so the total
    /// is unknown.
    #[test]
    fn a_call_still_in_flight_makes_the_cost_unknown() {
        let cost = SearchCost::default();
        cost.begin();
        cost.end(Some(0.007));
        cost.begin();
        assert_eq!(cost.total(), None);
    }

    /// 064 review N: rows survive a panic that poisoned the lock.
    #[test]
    fn jev_rows_survive_a_poisoned_lock() {
        let shadow = std::sync::Arc::new(JevShadow::default());
        shadow.push(crate::jev::shadow::tests_support::claim_row());
        let poisoner = std::sync::Arc::clone(&shadow);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.rows.lock().unwrap();
            std::panic::resume_unwind(Box::new(()));
        })
        .join();
        assert!(shadow.rows.is_poisoned());
        shadow.push(crate::jev::shadow::tests_support::claim_row());
        assert_eq!(shadow.take().len(), 2);
    }

    /// 064 review E: research's claim preparation time is summed, so the
    /// span's critical-path figure is measured, not a constant.
    #[test]
    fn jev_shadow_sums_preparation_time() {
        let shadow = JevShadow::default();
        shadow.add_prep(std::time::Duration::from_micros(2500));
        shadow.add_prep(std::time::Duration::from_micros(4600));
        assert_eq!(shadow.prep_count(), 2);
        assert_eq!(shadow.prep_ms(), 7);
    }

    /// 064 T015: Jev input is $0.042 per million tokens; output is free.
    #[test]
    fn jev_is_priced_per_input_token_only() {
        let model = crate::config::DEFAULT_JEV_MODEL;
        assert!((cost_usd(model, 1_000_000, 0) - 0.042).abs() < 1e-12);
        assert!((cost_usd(model, 1_000_000, 999) - 0.042).abs() < 1e-12);
    }

    /// 064 T015: the Jev cost is its own field and never enters `cost_usd`.
    #[test]
    fn a_record_carries_the_jev_cost_only_when_stamped_and_apart_from_cost_usd() {
        let clock = crate::traits::clock::SystemClock;
        let record = InvocationRecord::create(
            &clock,
            "s",
            "research",
            "claude-opus-4-8",
            &ModelUsage::default(),
            Outcome::Success,
            clock.now(),
        );
        assert_eq!(record.jev_cost_usd, None);
        let cost = record.cost_usd;
        let stamped = record.with_jev_cost(Some(0.000_04));
        assert_eq!(stamped.jev_cost_usd, Some(0.000_04));
        assert!((stamped.cost_usd - cost).abs() < f64::EPSILON);
    }

    #[test]
    fn a_record_carries_the_search_cost_only_when_stamped() {
        let clock = crate::traits::clock::SystemClock;
        let record = InvocationRecord::create(
            &clock,
            "s",
            "verify",
            "claude-opus-4-8",
            &ModelUsage::default(),
            Outcome::Success,
            clock.now(),
        );
        assert_eq!(record.search_cost_usd, None);
        assert_eq!(
            record.with_search_cost(Some(0.012)).search_cost_usd,
            Some(0.012)
        );
    }

    fn fixed(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Every model this crate defaults to must have a price row.
    ///
    /// `DEFAULT_MODEL` and `PRICING_PER_MTOK` both name model ids and nothing
    /// links them. Renaming a default to an id absent from the table costs the
    /// run at [`FALLBACK_PRICING`] with `pricing_known = false` — a figure that
    /// is an over-estimate rather than a measurement, reported as though it
    /// were a price. Nothing failed.
    ///
    /// The constants are read, never restated here: a test spelling out
    /// `"claude-opus-4-8"` would pass while the code moved underneath it, which
    /// is the defect rather than the check (040 §the verification ladder). Both
    /// defaults are covered rather than only the one that prompted this —
    /// naming a single constant would be the hand-written list of one that 040
    /// found inside the feature written to abolish hand-written lists.
    #[test]
    fn every_default_model_has_a_price_row() {
        for (constant, model) in [
            ("DEFAULT_MODEL", crate::config::DEFAULT_MODEL),
            ("DEFAULT_VOYAGE_MODEL", crate::config::DEFAULT_VOYAGE_MODEL),
            // 060 T018: the claude_cli backend's two defaults.
            (
                "DEFAULT_CLAUDE_CLI_MODEL",
                crate::config::DEFAULT_CLAUDE_CLI_MODEL,
            ),
            (
                "CLAUDE_CLI_BULK_MODEL",
                crate::routing::CLAUDE_CLI_BULK_MODEL,
            ),
            // 064 T015: the pinned Jev model.
            ("DEFAULT_JEV_MODEL", crate::config::DEFAULT_JEV_MODEL),
        ] {
            assert!(
                pricing_known(model),
                "PRICING_UNLINKED: {constant} is `{model}`, which has no row in \
                 PRICING_PER_MTOK. Every run on the default would be costed at the \
                 fallback rate and reported with pricing_known = false — an \
                 over-estimate presented as a price, with nothing failing. Add the row, \
                 or change the default to an id that has one."
            );
        }
    }

    /// The console log line must be self-describing about pricing: a
    /// fallback-costed figure carries `cost.estimated=true`, a catalog-priced
    /// one `cost.estimated=false` — matching the flag SQLite stores and OTel
    /// exports, so an operator can tell a known price from a guess on the
    /// same line that shows the figure.
    #[test]
    fn the_invocation_log_line_reports_whether_the_cost_is_estimated() {
        #[derive(Clone, Default)]
        struct Buffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Buffer {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
            type Writer = Self;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }
        impl Buffer {
            fn contents(&self) -> Vec<u8> {
                self.0.lock().unwrap().clone()
            }
        }

        let fixed = DateTime::parse_from_rfc3339("2026-06-11T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut clock = MockTimeProvider::new();
        clock.expect_now().return_const(fixed);

        let logged = |usage: &ModelUsage, attributed: &str| {
            let record = InvocationRecord::create(
                &clock,
                "session",
                "verify",
                attributed,
                usage,
                Outcome::Success,
                fixed,
            );
            let buffer = Buffer::default();
            let subscriber = tracing_subscriber::fmt()
                .with_writer(buffer.clone())
                .with_ansi(false)
                .finish();
            let _guard = tracing::subscriber::set_default(subscriber);
            record.emit();
            String::from_utf8_lossy(&buffer.contents()).to_string()
        };

        // An unknown model id prices off the fallback: estimated = true.
        let mut fallback = ModelUsage::default();
        fallback.add("qwen2.5:7b", 24, 0);
        let line = logged(&fallback, "qwen2.5:7b");
        assert!(line.contains("invocation recorded"), "{line}");
        assert!(line.contains("cost.estimated=true"), "{line}");

        // A catalog model prices off its row: estimated = false.
        let known = ModelUsage::single("claude-opus-4-8", 300, 30);
        let line = logged(&known, "claude-opus-4-8");
        assert!(line.contains("cost.estimated=false"), "{line}");
    }

    #[test]
    fn cost_uses_the_per_model_table() {
        // 1M input + 1M output on Opus 4.8 = $5 + $25.
        assert!((cost_usd("claude-opus-4-8", 1_000_000, 1_000_000) - 30.0).abs() < 1e-9);
        // 300 input + 30 output (the 3-pass test sums) — small but non-zero.
        let small = cost_usd("claude-opus-4-8", 300, 30);
        assert!(small > 0.0 && small < 0.01);
        // Haiku is cheaper than Opus for identical usage.
        assert!(cost_usd("claude-haiku-4-5", 1000, 1000) < cost_usd("claude-opus-4-8", 1000, 1000));
        // Unknown models fall back to Opus-tier (conservative over-estimate).
        assert!(
            (cost_usd("some-future-model", 1000, 1000) - cost_usd("claude-opus-4-8", 1000, 1000))
                .abs()
                < 1e-12
        );
        // Voyage embeddings: input-only billing, output tokens cost nothing.
        assert!((cost_usd("voyage-4", 1_000_000, 0) - 0.06).abs() < 1e-12);
        assert!((cost_usd("voyage-4", 1_000_000, 999) - 0.06).abs() < 1e-12);
    }

    #[test]
    fn record_carries_latency_from_the_clock_and_all_fields() {
        let started = fixed("2026-06-11T00:00:00Z");
        let mut clock = MockTimeProvider::new();
        clock
            .expect_now()
            .return_const(fixed("2026-06-11T00:00:02.500Z"));

        let record = InvocationRecord::create(
            &clock,
            "session-1",
            "verify",
            "claude-opus-4-8",
            &ModelUsage::single("claude-opus-4-8", 300, 30),
            Outcome::Success,
            started,
        );

        assert_eq!(record.latency_ms, 2_500);
        assert_eq!(record.outcome, Outcome::Success);
        assert_eq!(record.tool, "verify");
        assert!(!record.id.is_empty());
        assert!(record.cost_usd > 0.0);
        assert_eq!(record.created_at, fixed("2026-06-11T00:00:02.500Z"));
    }

    // ---- 018 US2: per-model usage and cost --------------------------------

    // T019 / D5.
    #[test]
    fn model_usage_totals_dominance_and_tie_break() {
        let mut usage = ModelUsage::default();
        usage.add("claude-opus-5", 100, 20);
        usage.add("claude-haiku-4-5", 900, 80);
        usage.add("claude-opus-5", 50, 5); // accumulates, not replaces

        assert_eq!(usage.totals(), (1_050, 105));
        assert_eq!(usage.models(), vec!["claude-haiku-4-5", "claude-opus-5"]);
        // Haiku moved 980 tokens vs Opus's 175 — dominance is by measured
        // tokens, even though Opus costs far more per token.
        assert_eq!(usage.dominant(), Some("claude-haiku-4-5"));

        // Ties break lexicographically, so attribution is deterministic.
        let mut tied = ModelUsage::default();
        tied.add("bbb", 10, 10);
        tied.add("aaa", 10, 10);
        assert_eq!(tied.dominant(), Some("aaa"));

        // Nothing ran: no dominant model, no models, no cost.
        let empty = ModelUsage::default();
        assert_eq!(empty.dominant(), None);
        assert!(empty.models().is_empty());
        assert!(empty.is_empty());
        assert!((empty.cost_usd() - 0.0).abs() < f64::EPSILON);
    }

    // T020 / FR-008 / SC-003: cost is the sum over models at each model's own
    // rate — not one model's rate applied to every token.
    #[test]
    fn multi_model_cost_sums_each_model_at_its_own_rate() {
        let mut usage = ModelUsage::default();
        usage.add("claude-opus-4-8", 1_000_000, 1_000_000); // $5 + $25
        usage.add("claude-haiku-4-5", 1_000_000, 1_000_000); // $1 + $5

        // Hand computation: 30.00 + 6.00.
        assert!((usage.cost_usd() - 36.0).abs() < 1e-9);

        // The wrong answers this replaces: one model's rate over all tokens.
        let all_at_opus = cost_usd("claude-opus-4-8", 2_000_000, 2_000_000);
        let all_at_haiku = cost_usd("claude-haiku-4-5", 2_000_000, 2_000_000);
        assert!((all_at_opus - 60.0).abs() < 1e-9);
        assert!((all_at_haiku - 12.0).abs() < 1e-9);
        assert!(usage.cost_usd() < all_at_opus && usage.cost_usd() > all_at_haiku);
    }

    /// The Sonnet 5 row carries Anthropic's current first-party rate, $2/$10
    /// (the claude-api model table cached 2026-11-03); it had stayed at the
    /// $3/$15 cached 2026-07-24.
    #[test]
    fn sonnet_5_prices_at_the_current_rate() {
        let cost = cost_usd("claude-sonnet-5", 1_000_000, 1_000_000);
        assert!(
            (cost - 12.0).abs() < 1e-9,
            "sonnet 5 prices at 2/10, got {cost}"
        );
    }

    // T022 / FR-011 / FR-012.
    #[test]
    fn unknown_models_price_conservatively_and_say_so() {
        // The shipped current models are known.
        for model in [
            "claude-fable-5",
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-opus-4-8",
            "claude-haiku-4-5",
        ] {
            assert!(pricing_known(model), "{model}");
        }
        assert!(!pricing_known("some-future-model"));

        // Fable 5 is double the Opus-tier fallback — the gap that made an
        // unrouted-to-Fable deployment under-report spend by half.
        assert!(
            (cost_usd("claude-fable-5", 1_000_000, 1_000_000) - 60.0).abs() < 1e-9,
            "fable 5 prices at 10/50"
        );

        // Unknown still over-estimates rather than under-reports...
        let unknown = ModelUsage::single("some-future-model", 1_000, 1_000);
        assert!(
            (unknown.cost_usd() - cost_usd("claude-opus-4-8", 1_000, 1_000)).abs() < 1e-12,
            "unknown falls back to Opus-tier"
        );
        // ...and says the figure is an estimate, which is what distinguishes
        // "correct by lookup" from "correct by coincidence" — `claude-opus-5`
        // happens to match the fallback exactly.
        assert!(unknown.cost_estimated());
        assert!(!ModelUsage::single("claude-opus-5", 1_000, 1_000).cost_estimated());

        // One unpriced participant marks the whole invocation estimated.
        let mut mixed = ModelUsage::single("claude-opus-5", 10, 10);
        mixed.add("some-future-model", 10, 10);
        assert!(mixed.cost_estimated());
    }

    // T021 / FR-009a / SC-004: a single-model record is byte-identical to what
    // the pre-018 server wrote. If this needs editing, FR-002 broke.
    #[test]
    fn single_model_record_matches_the_pre_feature_values() {
        let started = fixed("2026-06-11T00:00:00Z");
        let mut clock = MockTimeProvider::new();
        clock
            .expect_now()
            .return_const(fixed("2026-06-11T00:00:02.500Z"));

        let record = InvocationRecord::create(
            &clock,
            "session-1",
            "verify",
            "claude-opus-4-8",
            &ModelUsage::single("claude-opus-4-8", 300, 30),
            Outcome::Success,
            started,
        );

        // Exactly the pre-018 arithmetic and attribution.
        assert_eq!(record.model, "claude-opus-4-8");
        assert_eq!(record.input_tokens, 300);
        assert_eq!(record.output_tokens, 30);
        assert!((record.cost_usd - cost_usd("claude-opus-4-8", 300, 30)).abs() < f64::EPSILON);
    }

    // FR-015b: a model that failed or never ran contributes nothing. An empty
    // accumulator falls back to the attributed model without inventing usage.
    #[test]
    fn an_exit_with_no_usage_attributes_without_inventing_tokens() {
        let started = fixed("2026-06-11T00:00:00Z");
        let mut clock = MockTimeProvider::new();
        clock
            .expect_now()
            .return_const(fixed("2026-06-11T00:00:01Z"));

        let record = InvocationRecord::create(
            &clock,
            "s",
            "research",
            "claude-opus-4-8",
            &ModelUsage::default(),
            Outcome::RetriesExhausted,
            started,
        );

        assert_eq!(record.model, "claude-opus-4-8");
        assert_eq!((record.input_tokens, record.output_tokens), (0, 0));
        assert!((record.cost_usd - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn clock_skew_never_panics_or_goes_negative() {
        let started = fixed("2026-06-11T00:00:10Z");
        let mut clock = MockTimeProvider::new();
        // "now" before "started" — skew clamps to zero.
        clock
            .expect_now()
            .return_const(fixed("2026-06-11T00:00:05Z"));

        let record = InvocationRecord::create(
            &clock,
            "s",
            "verify",
            "m",
            &ModelUsage::default(),
            Outcome::Timeout,
            started,
        );
        assert_eq!(record.latency_ms, 0);
    }
}
