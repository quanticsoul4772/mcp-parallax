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
    /// The invocation id, made up front so the run's Jev rows can name it
    /// (064).
    id: String,
    /// The research run's Jev rows (064), or `None`.
    jev: Option<Arc<crate::telemetry::JevShadow>>,
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
            id: uuid::Uuid::new_v4().to_string(),
            jev: dims.jev,
            cli_cost: Arc::new(crate::telemetry::CliCost::default()),
            caller,
            started_at,
            done: false,
        }
    }

    /// This invocation's record id (064; tests check it is made up front).
    #[cfg(test)]
    pub(super) fn id(&self) -> &str {
        &self.id
    }

    /// The run's Jev rows, stamped with this invocation's id, and their
    /// summed cost (`None` when there are no rows).
    fn take_jev_rows(&self) -> (Vec<crate::storage::jev::JevRecord>, Option<f64>) {
        let mut rows = self
            .jev
            .as_ref()
            .map(|shadow| shadow.take())
            .unwrap_or_default();
        for row in &mut rows {
            row.invocation_id = Some(self.id.clone());
        }
        let cost = if rows.is_empty() {
            None
        } else {
            Some(rows.iter().filter_map(|row| row.cost_usd).sum())
        };
        (rows, cost)
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
        let (jev_rows, jev_cost) = self.take_jev_rows();
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
        })
        .with_id(self.id.clone())
        .with_jev_cost(jev_cost);
        // One measurement, two sinks (007 FR-009): tracing + telemetry, both
        // derived from this record value via the single publish() door.
        let jev_summary = self.jev.as_ref().map(|shadow| {
            crate::observability::JevSpanSummary::from_rows(
                &jev_rows,
                shadow.prep_ms(),
                shadow.mode(),
            )
        });
        record.publish_with_jev(self.caller.as_ref(), jev_summary.as_ref());
        if let Err(e) = self.storage.record_invocation(&record).await {
            // The record write itself failed — surface loudly on the
            // diagnostic stream; never on the protocol channel.
            tracing::error!(error = %e, "invocation record write failed");
        }
        if !jev_rows.is_empty() {
            if let Err(e) = self.storage.record_jev(&jev_rows).await {
                tracing::error!(error = %e, "jev rows write failed");
            }
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
        let (jev_rows, jev_cost) = self.take_jev_rows();
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
        .with_pages(self.pages.as_ref().and_then(|p| p.get()))
        .with_id(self.id.clone())
        .with_jev_cost(jev_cost);
        let jev_summary = self.jev.as_ref().map(|shadow| {
            crate::observability::JevSpanSummary::from_rows(
                &jev_rows,
                shadow.prep_ms(),
                shadow.mode(),
            )
        });
        record.publish_with_jev(self.caller.as_ref(), jev_summary.as_ref());
        let storage = Arc::clone(&self.storage);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(e) = storage.record_invocation(&record).await {
                    tracing::error!(error = %e, "cancelled-invocation record write failed");
                }
                if !jev_rows.is_empty() {
                    if let Err(e) = storage.record_jev(&jev_rows).await {
                        tracing::error!(error = %e, "cancelled-invocation jev rows write failed");
                    }
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

    // ---- 064 T023: research Jev rows on the invocation record ---------------

    fn jev_row(id: &str, cost: Option<f64>) -> crate::storage::jev::JevRecord {
        crate::storage::jev::JevRecord {
            id: id.into(),
            created_at: chrono::Utc::now(),
            server_session_id: "s".into(),
            harness_session_id: None,
            site: crate::traits::classifier::JevSite::ResearchClaims,
            mode: crate::traits::classifier::JevMode::Shadow,
            questions_json: serde_json::json!([]),
            model_requested: "jev-1.14.0".into(),
            model_answered: None,
            outcome: crate::storage::jev::JevOutcome::Call(Outcome::Success),
            error_class: None,
            request_id: None,
            latency_ms: None,
            input_tokens: None,
            cost_usd: cost,
            eval_id: None,
            invocation_id: None,
            claim_key: Some(format!("claim {id}")),
            source_ids: None,
            label_json: None,
            label_source: None,
            label_cost_usd: None,
            state_json: None,
            state_hash: None,
            transcript_ref: None,
            redaction_count: 0,
            truncated: false,
            applied: false,
        }
    }

    type Captured = (
        Arc<std::sync::Mutex<Vec<InvocationRecord>>>,
        Arc<std::sync::Mutex<Vec<Vec<crate::storage::jev::JevRecord>>>>,
    );

    /// A guard whose storage keeps every invocation record and every Jev
    /// batch it is handed.
    fn jev_guard(shadow: Option<Arc<crate::telemetry::JevShadow>>) -> (RecordGuard, Captured) {
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let batches = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (r, b) = (Arc::clone(&records), Arc::clone(&batches));
        let mut storage = MockStorage::new();
        storage.expect_record_invocation().returning(move |rec| {
            r.lock().unwrap().push(rec.clone());
            Ok(())
        });
        storage.expect_record_jev().returning(move |rows| {
            b.lock().unwrap().push(rows.to_vec());
            Ok(())
        });
        let guard = RecordGuard::new(
            Arc::new(storage),
            Arc::new(SystemClock),
            "s".into(),
            "research".into(),
            "claude-sonnet-5-5".into(),
            super::super::RecordDims {
                jev: shadow,
                ..super::super::RecordDims::default()
            },
            None,
        );
        (guard, (records, batches))
    }

    #[tokio::test]
    async fn the_invocation_id_is_made_when_the_guard_is_and_the_record_uses_it() {
        let (guard, (records, _)) = jev_guard(None);
        let id = guard.id().to_string();
        assert!(!id.is_empty());
        guard.finish(&ModelUsage::default(), Outcome::Success).await;
        assert_eq!(records.lock().unwrap()[0].id, id);
    }

    #[tokio::test]
    async fn finish_writes_the_rows_in_one_batch_linked_and_summed() {
        let shadow = Arc::new(crate::telemetry::JevShadow::default());
        shadow.push(jev_row("a", Some(0.001)));
        shadow.push(jev_row("b", Some(0.002)));
        shadow.push(jev_row("c", None));
        let (guard, (records, batches)) = jev_guard(Some(shadow));
        guard.finish(&ModelUsage::default(), Outcome::Success).await;
        let record = records.lock().unwrap()[0].clone();
        let batches = batches.lock().unwrap().clone();
        assert_eq!(batches.len(), 1, "one transaction");
        assert_eq!(batches[0].len(), 3);
        assert!(batches[0]
            .iter()
            .all(|row| row.invocation_id.as_deref() == Some(record.id.as_str())));
        let sum = record.jev_cost_usd.unwrap();
        assert!((sum - 0.003).abs() < 1e-12, "{sum}");
    }

    #[tokio::test]
    async fn a_cancelled_run_still_writes_its_rows() {
        let shadow = Arc::new(crate::telemetry::JevShadow::default());
        shadow.push(jev_row("a", Some(0.001)));
        let (guard, (records, batches)) = jev_guard(Some(shadow));
        drop(guard);
        for _ in 0..50 {
            if !batches.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let record = records.lock().unwrap()[0].clone();
        assert_eq!(record.outcome, Outcome::Cancelled);
        let batches = batches.lock().unwrap().clone();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0][0].invocation_id.as_deref(),
            Some(record.id.as_str())
        );
        assert_eq!(record.jev_cost_usd, Some(0.001));
    }

    #[tokio::test]
    async fn no_rows_means_no_jev_cost_and_no_batch() {
        let (guard, (records, batches)) =
            jev_guard(Some(Arc::new(crate::telemetry::JevShadow::default())));
        guard.finish(&ModelUsage::default(), Outcome::Success).await;
        assert_eq!(records.lock().unwrap()[0].jev_cost_usd, None);
        assert!(batches.lock().unwrap().is_empty());
    }

    /// 064 T027: the span's Jev cost is the record's, and Jev never enters the
    /// record's model usage (so never the token-usage metric).
    #[tokio::test]
    async fn the_span_jev_cost_matches_the_record_and_tokens_exclude_jev() {
        let (spans, _, otel) = crate::observability::test_exporters();
        let shadow = Arc::new(crate::telemetry::JevShadow::default());
        shadow.push(jev_row("a", Some(0.001)));
        shadow.push(jev_row("b", Some(0.002)));
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let r = Arc::clone(&records);
        let mut storage = MockStorage::new();
        storage.expect_record_invocation().returning(move |rec| {
            r.lock().unwrap().push(rec.clone());
            Ok(())
        });
        storage.expect_record_jev().returning(|_| Ok(()));
        let guard = RecordGuard::new(
            Arc::new(storage),
            Arc::new(SystemClock),
            "s".into(),
            "jev_parity_probe".into(),
            "claude-sonnet-5-5".into(),
            super::super::RecordDims {
                jev: Some(shadow),
                ..super::super::RecordDims::default()
            },
            None,
        );
        guard
            .finish(
                &ModelUsage::single("claude-sonnet-5-5", 10, 5),
                Outcome::Success,
            )
            .await;
        otel.flush();
        let record = records.lock().unwrap()[0].clone();
        assert!(!record.models.iter().any(|m| m.starts_with("jev")));
        assert_eq!((record.input_tokens, record.output_tokens), (10, 5));
        let exported = spans.get_finished_spans().unwrap();
        let span = exported
            .iter()
            .find(|s| s.name == "parallax.jev_parity_probe")
            .unwrap();
        let cost = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "parallax.jev.cost_usd")
            .map(|kv| kv.value.to_string());
        assert_eq!(cost, record.jev_cost_usd.map(|c| c.to_string()));
    }

    /// 064 review E: the invocation span's critical path is the measured
    /// preparation time, not a constant 0.
    #[tokio::test]
    async fn the_span_critical_path_is_the_measured_preparation_time() {
        let (spans, _, otel) = crate::observability::test_exporters();
        let shadow = Arc::new(crate::telemetry::JevShadow::default());
        shadow.push(jev_row("a", Some(0.001)));
        shadow.add_prep(std::time::Duration::from_millis(7));
        let mut storage = MockStorage::new();
        storage.expect_record_invocation().returning(|_| Ok(()));
        storage.expect_record_jev().returning(|_| Ok(()));
        let guard = RecordGuard::new(
            Arc::new(storage),
            Arc::new(SystemClock),
            "s".into(),
            "jev_prep_probe".into(),
            "claude-sonnet-5-5".into(),
            super::super::RecordDims {
                jev: Some(shadow),
                ..super::super::RecordDims::default()
            },
            None,
        );
        guard.finish(&ModelUsage::default(), Outcome::Success).await;
        otel.flush();
        let exported = spans.get_finished_spans().unwrap();
        let span = exported
            .iter()
            .find(|s| s.name == "parallax.jev_prep_probe")
            .unwrap();
        let critical = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "parallax.jev.critical_path_ms")
            .map(|kv| kv.value.to_string());
        assert_eq!(critical.as_deref(), Some("7"));
    }
}
