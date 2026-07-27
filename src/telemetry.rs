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
    ("claude-fable-5", 10.00, 50.00),
    ("claude-opus-5", 5.00, 25.00),
    ("claude-sonnet-5", 3.00, 15.00),
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
        self.by_model.values().fold((0, 0), |(input, output), u| {
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
        self.by_model
            .iter()
            .max_by(|(a_model, a), (b_model, b)| {
                let a_total = a.input_tokens.saturating_add(a.output_tokens);
                let b_total = b.input_tokens.saturating_add(b.output_tokens);
                // Reverse the id comparison so the *earliest* id wins a tie
                // (max_by keeps the last maximum).
                a_total.cmp(&b_total).then(b_model.cmp(a_model))
            })
            .map(|(model, _)| model.as_str())
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
            latency_ms,
            outcome,
            created_at,
        }
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
    /// exactly this reason: `publish` is the only door.
    pub fn publish(&self) {
        self.emit();
        crate::observability::emit_invocation(self);
    }

    /// Emit the record as a structured tracing event with GenAI
    /// semantic-convention attribute names. Private: every exit point goes
    /// through [`Self::publish`] so tracing and telemetry cannot diverge.
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
