//! Per-invocation recording plumbing shared by every tool: the drop-guard
//! around one invocation and the failure-class → MCP error mapping.

use crate::error::{AppError, Outcome};
use crate::telemetry::InvocationRecord;
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
    match error {
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
    ) -> Self {
        let started_at = clock.now();
        Self {
            storage,
            clock,
            session_id,
            tool,
            model,
            started_at,
            done: false,
        }
    }

    pub(super) async fn finish(mut self, input_tokens: u64, output_tokens: u64, outcome: Outcome) {
        self.done = true;
        let record = InvocationRecord::create(
            self.clock.as_ref(),
            &self.session_id,
            &self.tool,
            &self.model,
            input_tokens,
            output_tokens,
            outcome,
            self.started_at,
        );
        record.emit();
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
            0,
            0,
            Outcome::Cancelled,
            self.started_at,
        );
        record.emit();
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
