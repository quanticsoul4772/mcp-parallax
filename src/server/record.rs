//! Per-invocation recording plumbing shared by every tool: the drop-guard
//! around one invocation and the failure-class → MCP error mapping.

use crate::error::{AppError, Outcome};
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
            started_at,
            done: false,
        }
    }

    pub(super) async fn finish(mut self, usage: &ModelUsage, outcome: Outcome) {
        self.done = true;
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
        .with_passes(self.passes);
        // One measurement, two sinks (007 FR-009): tracing + telemetry, both
        // derived from this record value via the single publish() door.
        record.publish();
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
            &ModelUsage::default(),
            Outcome::Cancelled,
            self.started_at,
        )
        .with_depth(self.depth.as_deref())
        .with_effort(self.effort)
        .with_passes(self.passes);
        record.publish();
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
