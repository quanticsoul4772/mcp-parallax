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
    /// Estimated cost (tokens × configured per-model pricing).
    pub cost_usd: f64,
    /// Wall-clock latency via [`TimeProvider`].
    pub latency_ms: u64,
    /// Outcome classification.
    pub outcome: Outcome,
    /// RFC 3339 creation time via [`TimeProvider`].
    pub created_at: DateTime<Utc>,
}

impl InvocationRecord {
    /// Build the record at the single exit point of an invocation.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // the record IS this tuple; a builder adds nothing
    pub fn create(
        clock: &dyn TimeProvider,
        session_id: &str,
        tool: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        outcome: Outcome,
        started_at: DateTime<Utc>,
    ) -> Self {
        let created_at = clock.now();
        let latency_ms =
            u64::try_from((created_at - started_at).num_milliseconds().max(0)).unwrap_or(u64::MAX);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            tool: tool.to_string(),
            model: model.to_string(),
            input_tokens,
            output_tokens,
            cost_usd: cost_usd(model, input_tokens, output_tokens),
            latency_ms,
            outcome,
            created_at,
        }
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
            300,
            30,
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

    #[test]
    fn clock_skew_never_panics_or_goes_negative() {
        let started = fixed("2026-06-11T00:00:10Z");
        let mut clock = MockTimeProvider::new();
        // "now" before "started" — skew clamps to zero.
        clock
            .expect_now()
            .return_const(fixed("2026-06-11T00:00:05Z"));

        let record =
            InvocationRecord::create(&clock, "s", "verify", "m", 0, 0, Outcome::Timeout, started);
        assert_eq!(record.latency_ms, 0);
    }
}
