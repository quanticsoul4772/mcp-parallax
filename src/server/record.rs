//! Per-invocation recording plumbing shared by every tool: the drop-guard
//! around one invocation and the failure-class → MCP error mapping.

use crate::error::{AppError, Outcome};
use crate::observability::CallerTrace;
use crate::telemetry::{InvocationRecord, ModelUsage};
use crate::traits::clock::TimeProvider;
use crate::traits::storage::Storage;
use chrono::{DateTime, Utc};
use rmcp::model::ErrorData;
use std::sync::Arc;

/// Map every failure class to a distinct, descriptive MCP error (FR-007 /
/// SC-005): the class is identifiable from the message alone, via the
/// outcome-taxonomy prefix plus the `AppError` Display text.
pub(super) fn to_error_data(error: &AppError) -> ErrorData {
    let message = format!("[{}] {error}", error.outcome().as_str());
    // Match the root: attached usage (020) is orthogonal to the failure class,
    // and a wrapper must not silently reclassify an invalid-params error as an
    // internal one.
    match error.root() {
        AppError::InvalidInput(_) => ErrorData::invalid_params(message, None),
        _ => ErrorData::internal_error(message, None),
    }
}

/// Drop-guard around one invocation: `finish()` writes the real record;
/// dropping unfinished (the request future was abandoned) records `cancelled`.
pub(super) struct RecordGuard {
    storage: Arc<dyn Storage>,
    clock: Arc<dyn TimeProvider>,
    session_id: String,
    tool: String,
    model: String,
    /// The research rigor tier, when the invocation has one (019). Captured at
    /// construction from the request, so it is stamped on every exit — a run
    /// that fails or is cancelled still records which ceiling it ran under,
    /// which is precisely the case worth knowing about when sizing budgets.
    depth: Option<String>,
    /// The caller's effort override (028), or `None` when configuration
    /// alone decided. Only the override: see `InvocationRecord::with_effort`.
    effort: Option<crate::routing::Effort>,
    /// The caller's pass-count override (028), or `None`.
    passes: Option<u32>,
    /// The research run's Exa cost accumulator (059), or `None`. Read on
    /// every exit, so a failed or cancelled run still records its cost.
    search_cost: Option<Arc<crate::telemetry::SearchCost>>,
    /// The research run's page counts (061), or `None`.
    pages: Option<Arc<crate::telemetry::PageCounts>>,
    /// The research run's non-model usage (061 review finding 3), or `None`.
    non_model: Option<Arc<crate::telemetry::NonModelUsage>>,
    /// The invocation's `claude_cli` cost meter (060 D11), scoped around its
    /// work by the server; read when the invocation finishes with usage.
    cli_cost: Arc<crate::telemetry::CliCost>,
    /// The trace context the request carried (058), or `None`. Held until the
    /// record is published so every exit, cancellation included, exports its
    /// span under the caller's trace. Never stored.
    caller: Option<CallerTrace>,
    started_at: DateTime<Utc>,
    done: bool,
}

impl RecordGuard {
    pub(super) fn new(
        storage: Arc<dyn Storage>,
        clock: Arc<dyn TimeProvider>,
        session_id: String,
        tool: String,
        model: String,
        dims: super::RecordDims,
        caller: Option<CallerTrace>,
    ) -> Self {
        let started_at = clock.now();
        Self {
            storage,
            clock,
            session_id,
            tool,
            model,
            depth: dims.depth.map(|d| d.as_str().to_string()),
            effort: dims.effort,
            passes: dims.passes,
            search_cost: dims.search_cost,
            pages: dims.pages,
            non_model: dims.non_model,
            cli_cost: Arc::new(crate::telemetry::CliCost::default()),
            caller,
            started_at,
            done: false,
        }
    }

    /// The meter the invocation's work runs inside (060 D11).
    pub(super) fn cli_cost(&self) -> Arc<crate::telemetry::CliCost> {
        Arc::clone(&self.cli_cost)
    }

    /// A run that did not succeed returns no usage of its own beyond the
    /// tokens its error carried; its non-model usage (the rerank) is added
    /// here. A success's usage already holds it (061 review finding 3).
    fn with_non_model(&self, usage: &ModelUsage, outcome: Outcome) -> ModelUsage {
        let mut usage = usage.clone();
        if outcome != Outcome::Success {
            if let Some(side) = &self.non_model {
                usage.merge(&side.snapshot());
            }
        }
        usage
    }

    pub(super) async fn finish(mut self, model_usage: &ModelUsage, outcome: Outcome) {
        self.done = true;
        let usage = &self.with_non_model(model_usage, outcome);
        let record = InvocationRecord::create(
            self.clock.as_ref(),
            &self.session_id,
            &self.tool,
            // The fallback attribution: a failed or cancelled invocation
            // records no tokens, so there is no dominant model to derive.
            &self.model,
            usage,
            outcome,
            self.started_at,
        )
        .with_depth(self.depth.as_deref())
        .with_effort(self.effort)
        .with_passes(self.passes)
        .with_search_cost(self.search_cost.as_ref().and_then(|c| c.total()))
        .with_pages(self.pages.as_ref().and_then(|p| p.get()))
        // The CLI's figure only beside the tokens it priced: an exit that
        // records no usage records no cost either.
        .with_cli_cost(if model_usage.is_empty() {
            None
        } else {
            self.cli_cost.total()
        });
        // One measurement, two sinks (007 FR-009): tracing + telemetry, both
        // derived from this record value via the single publish() door.
        record.publish(self.caller.as_ref());
        if let Err(e) = self.storage.record_invocation(&record).await {
            // The record write itself failed — surface loudly on the
            // diagnostic stream; never on the protocol channel.
            tracing::error!(error = %e, "invocation record write failed");
        }
    }
}

impl Drop for RecordGuard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Abandoned mid-flight: the edge case "client disconnects
        // mid-invocation" — record `cancelled` (spec edge case 4).
        let record = InvocationRecord::create(
            self.clock.as_ref(),
            &self.session_id,
            &self.tool,
            &self.model,
            &self.with_non_model(&ModelUsage::default(), Outcome::Cancelled),
            Outcome::Cancelled,
            self.started_at,
        )
        .with_depth(self.depth.as_deref())
        .with_effort(self.effort)
        .with_passes(self.passes)
        .with_search_cost(self.search_cost.as_ref().and_then(|c| c.total()))
        .with_pages(self.pages.as_ref().and_then(|p| p.get()));
        record.publish(self.caller.as_ref());
        let storage = Arc::clone(&self.storage);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(e) = storage.record_invocation(&record).await {
                    tracing::error!(error = %e, "cancelled-invocation record write failed");
                }
            });
        } else {
            // No runtime to persist on — say so loudly rather than silently
            // dropping the record (FR-010).
            tracing::error!("cancelled-invocation record not persisted: no tokio runtime");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::traits::clock::SystemClock;
    use crate::traits::storage::MockStorage;

    fn guard_capturing(
        side: Arc<crate::telemetry::NonModelUsage>,
    ) -> (RecordGuard, Arc<std::sync::Mutex<Vec<InvocationRecord>>>) {
        let written = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&written);
        let mut storage = MockStorage::new();
        storage.expect_record_invocation().returning(move |r| {
            sink.lock().unwrap().push(r.clone());
            Ok(())
        });
        let guard = RecordGuard::new(
            Arc::new(storage),
            Arc::new(SystemClock),
            "s".into(),
            "research".into(),
            "claude-sonnet-5-5".into(),
            super::super::RecordDims {
                non_model: Some(side),
                ..super::super::RecordDims::default()
            },
            None,
        );
        (guard, written)
    }

    /// 061 review finding 3: a failed run's record carries the non-model
    /// usage (the rerank) beside the model tokens the error carried.
    #[tokio::test]
    async fn a_failed_record_carries_the_non_model_usage() {
        let side = Arc::new(crate::telemetry::NonModelUsage::default());
        side.add("rerank-3", 100);
        let (guard, written) = guard_capturing(Arc::clone(&side));
        guard
            .finish(
                &ModelUsage::single("claude-sonnet-5-5", 10, 5),
                Outcome::ValidationFailure,
            )
            .await;
        let records = written.lock().unwrap().clone();
        assert!(records[0].models.contains(&"rerank-3".to_string()));
        assert_eq!((records[0].input_tokens, records[0].output_tokens), (10, 5));
    }

    /// On success the run's own usage already holds the rerank: no double count.
    #[tokio::test]
    async fn a_successful_record_does_not_count_the_rerank_twice() {
        let side = Arc::new(crate::telemetry::NonModelUsage::default());
        side.add("rerank-3", 100);
        let (guard, written) = guard_capturing(Arc::clone(&side));
        let mut usage = ModelUsage::single("claude-sonnet-5-5", 10, 5);
        usage.add("rerank-3", 100, 0);
        guard.finish(&usage, Outcome::Success).await;
        let records = written.lock().unwrap().clone();
        let rerank: Vec<_> = records[0]
            .usage_by_model
            .iter()
            .filter(|(m, _)| *m == "rerank-3")
            .map(|(_, u)| u.input_tokens)
            .collect();
        assert_eq!(rerank, vec![100]);
    }
}
