//! Per-boundary orchestration: validate → screen → (review) → assemble →
//! record.
//!
//! Fail-open is the spine (FR-008): any evaluation failure becomes a
//! recorded silence-with-`fail_open`, never an error that could block the
//! session. Only the audit write itself propagates — the harness's hooks
//! fail open on errors, so a broken store degrades to no-op there too.

use crate::checkpoint::contract::{
    CheckpointActionParams, CheckpointBatchParams, CheckpointResult, CheckpointTurnParams,
};
use crate::checkpoint::{
    gate, preference, review, screen, Boundary, CheckpointRecord, Signal, SignalKind,
    COOLDOWN_WINDOW_MS, GATE_BUDGET_MS,
};
use crate::error::AppError;
use crate::modes::CorrectiveMode;
use crate::observability::CallerTrace;
use crate::telemetry;
use crate::traits::client::ModelClient;
use crate::traits::clock::TimeProvider;
use crate::traits::embedder::Embedder;
use crate::traits::storage::Storage;
use crate::traits::trajectory::TrajectoryReader;
use chrono::Duration;
use std::sync::Arc;
use std::time::Instant;

/// Everything the three checkpoint boundaries need.
pub struct CheckpointDeps {
    /// Bounded, validated trajectory access (the seventh seam).
    pub reader: Arc<dyn TrajectoryReader>,
    /// Records + cooldown lookups + stored memories.
    pub storage: Arc<dyn Storage>,
    /// Cooldown windows and record timestamps.
    pub clock: Arc<dyn TimeProvider>,
    /// For the end-of-turn review hop only.
    pub model_client: Arc<dyn ModelClient>,
    /// The registered review mode.
    pub review_mode: CorrectiveMode,
    /// Anthropic model id (review-hop cost attribution on checkpoint records).
    pub model: String,
    /// Gate/turn recall — `None` when the memory capability is disabled
    /// (memory-paired signals silently inactive, FR-004/spec edge case).
    pub embedder: Option<Arc<dyn Embedder>>,
    /// `CHECKPOINT_GATE_PATTERNS` extras (FR-013).
    pub gate_extra_patterns: Vec<String>,
    /// The Jev turn shadow (064); `None` when the site is off.
    pub jev: Option<crate::jev::shadow::TurnJev>,
    /// The 066 turn signals; `None` when `CHECKPOINT_TURN_SIGNALS` is off.
    pub turn_signals: Option<crate::checkpoint::signals::TurnSignalsDeps>,
}

/// What one evaluation concluded, before recording.
struct Evaluated {
    signal_kinds: Vec<SignalKind>,
    fired: Vec<Signal>,
    /// Keys actually delivered (FR-010 cooldown feed) — the unsuppressed
    /// subset for flags, the held signal for holds, empty otherwise.
    delivered_keys: Vec<String>,
    review_ran: bool,
    result: CheckpointResult,
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
}

impl Evaluated {
    fn pure(signal_kinds: Vec<SignalKind>, fired: Vec<Signal>, result: CheckpointResult) -> Self {
        Self {
            signal_kinds,
            fired,
            delivered_keys: vec![],
            review_ran: false,
            result,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
        }
    }
}

/// `checkpoint_batch` (US1): deterministic loop/repeated-failure screening.
///
/// # Errors
///
/// Only the audit write propagates; every evaluation failure is a recorded
/// fail-open silence (FR-008).
pub async fn run_batch(
    deps: &CheckpointDeps,
    params: &CheckpointBatchParams,
    caller: Option<&CallerTrace>,
) -> Result<(CheckpointResult, u64, u64), AppError> {
    let started = Instant::now();
    let evaluated_kinds = vec![SignalKind::Repetition, SignalKind::RepeatedFailure];

    let evaluation = async {
        let (window, _lines) = deps
            .reader
            .read(&params.transcript_path, &params.session_id)
            .await?;
        let fired = screen::screen(&window);
        if fired.is_empty() {
            return Ok(Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            ));
        }
        let remaining = unsuppressed(deps, &params.session_id, &fired).await?;
        let (result, delivered_keys) = if remaining.is_empty() {
            (
                CheckpointResult::suppressed(&fired, elapsed_ms(started)),
                vec![],
            )
        } else {
            (
                CheckpointResult::flag(
                    batch_flag_message(&remaining),
                    &remaining,
                    elapsed_ms(started),
                ),
                remaining.iter().map(|s| s.signal_key.clone()).collect(),
            )
        };
        let mut evaluation = Evaluated::pure(evaluated_kinds.clone(), fired, result);
        evaluation.delivered_keys = delivered_keys;
        Ok::<Evaluated, AppError>(evaluation)
    }
    .await;

    let evaluation = recover(evaluation, evaluated_kinds, started, "checkpoint_batch");
    record(
        deps,
        Boundary::Batch,
        &params.session_id,
        &evaluation,
        caller,
    )
    .await?;
    Ok((
        evaluation.result,
        evaluation.input_tokens,
        evaluation.output_tokens,
    ))
}

/// `checkpoint_action` (US2): the risk-matched, deterministic pre-action gate
/// under a hard budget.
///
/// # Errors
///
/// Only the audit write propagates (see [`run_batch`]).
pub async fn run_action(
    deps: &CheckpointDeps,
    params: &CheckpointActionParams,
    caller: Option<&CallerTrace>,
) -> Result<(CheckpointResult, u64, u64), AppError> {
    let started = Instant::now();

    // FR-013: non-risk-matched actions pass with no evaluation at all.
    if !gate::risk_matched(
        &params.tool_name,
        &params.tool_input,
        &deps.gate_extra_patterns,
    ) {
        let evaluation = Evaluated::pure(
            vec![],
            vec![],
            CheckpointResult::silence(elapsed_ms(started)),
        );
        record(
            deps,
            Boundary::Action,
            &params.session_id,
            &evaluation,
            caller,
        )
        .await?;
        return Ok((evaluation.result, 0, 0));
    }

    let evaluated_kinds = vec![SignalKind::MemoryConflict];
    let evaluation = async {
        // Memory capability absent → the signal is silently inactive
        // (spec edge case), not a failure.
        let Some(embedder) = deps.embedder.as_ref() else {
            return Ok(Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            ));
        };
        // The whole gate decision sits under the hard budget (FR-009).
        let work = async {
            let query = embedder
                .embed_query(&format!("{} {}", params.tool_name, params.tool_input))
                .await?;
            let memories = deps.storage.load_memories().await?;
            Ok::<_, AppError>((
                gate::constraint_hold(&query.vector, &memories),
                query.input_tokens,
            ))
        };
        let (held, embed_tokens) =
            tokio::time::timeout(std::time::Duration::from_millis(GATE_BUDGET_MS), work)
                .await
                .map_err(|_| AppError::Timeout {
                    what: "pre-action gate",
                    ms: GATE_BUDGET_MS,
                })??;

        let mut evaluation = match held {
            // FR-011: holds escalate to the user; FR-010: never rate-limited.
            Some((signal, memory_content)) => {
                let delivered_keys = vec![signal.signal_key.clone()];
                let fired = vec![signal];
                let result = CheckpointResult::hold(
                    hold_message(&memory_content),
                    &fired,
                    elapsed_ms(started),
                );
                let mut held_evaluation = Evaluated::pure(evaluated_kinds.clone(), fired, result);
                held_evaluation.delivered_keys = delivered_keys;
                held_evaluation
            }
            None => Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            ),
        };
        evaluation.input_tokens = embed_tokens;
        Ok::<Evaluated, AppError>(evaluation)
    }
    .await;

    let evaluation = recover(evaluation, evaluated_kinds, started, "checkpoint_action");
    record(
        deps,
        Boundary::Action,
        &params.session_id,
        &evaluation,
        caller,
    )
    .await?;
    Ok((
        evaluation.result,
        evaluation.input_tokens,
        evaluation.output_tokens,
    ))
}

/// `checkpoint_turn` (US3): deterministic candidate mining gating at most
/// one blind review hop; flags deliver as forced continuation (FR-014).
///
/// # Errors
///
/// Only the audit write propagates (see [`run_batch`]).
#[allow(clippy::too_many_lines)] // the turn's steps read in order; review_beside_jev already split out
pub async fn run_turn(
    deps: &CheckpointDeps,
    params: &CheckpointTurnParams,
    caller: Option<&CallerTrace>,
) -> Result<(CheckpointResult, u64, u64), AppError> {
    let started = Instant::now();
    // 064: the evaluation id exists from the start, so the Jev row can name it.
    let eval_id = uuid::Uuid::new_v4().to_string();
    let mut jev_row: Option<TurnJevOutcome> = None;
    // 066: the turn signals' row joins this evaluation's record.
    let signals_eval_id = deps.turn_signals.as_ref().map(|_| eval_id.clone());
    let has_final = !params.final_message.trim().is_empty();

    // FR-014: a turn end that follows a forced continuation never reviews
    // again — continuation cannot loop.
    if params.continuation {
        // 066 FR-005: continuation turns write no row but still update the
        // session's previous final message.
        if let (Some(signals), true) = (&deps.turn_signals, has_final) {
            signals
                .previous_finals
                .swap(&params.session_id, &params.final_message);
        }
        let evaluation = Evaluated::pure(
            vec![],
            vec![],
            CheckpointResult::silence(elapsed_ms(started)),
        );
        record_with_jev(
            deps,
            Boundary::Turn,
            &params.session_id,
            &evaluation,
            caller,
            eval_id,
            None,
        )
        .await?;
        return Ok((evaluation.result, 0, 0));
    }

    // 015 research D7: the enforcement signal is listed as evaluated only
    // when the capability that feeds it is configured — with memory off,
    // records stay byte-identical to the contradiction-only layer (FR-006).
    let evaluated_kinds = if deps.embedder.is_some() {
        vec![
            SignalKind::SelfContradiction,
            SignalKind::PreferenceViolation,
            SignalKind::CaptureProposal,
        ]
    } else {
        vec![SignalKind::SelfContradiction]
    };
    // 066: the raw tail lines of the one transcript read; `None` after the
    // evaluation means the read failed (or there was no final message).
    let mut turn_lines: Option<Vec<String>> = None;
    let evaluation = async {
        if params.final_message.trim().is_empty() {
            return Ok(Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            ));
        }
        let (window, lines) = deps
            .reader
            .read(&params.transcript_path, &params.session_id)
            .await?;
        turn_lines = Some(lines);
        let (recall, mut input_tokens) = turn_recall(deps, &params.final_message).await?;
        let candidates = review::mine_candidates(&window, &params.final_message, &recall);
        let preferences = preference::mine_preference_candidates(&recall);
        // 017: with memory configured, the hop always runs — capture is an
        // every-turn-end judgment (clarify Q1: harness-triggered), and its
        // "screen" is the hop's own decline bias. Without memory, the
        // 006-era gate stands: no candidates → no model pass (US3-AS2).
        let capture_enabled = deps.embedder.is_some();
        if candidates.is_empty() && preferences.is_empty() && !capture_enabled {
            let mut evaluation = Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            );
            evaluation.input_tokens = input_tokens;
            return Ok(evaluation);
        }

        // One hop, three judgments (015 FR-010 · 017 research D6).
        let (review, row) = review_beside_jev(
            deps,
            params,
            &eval_id,
            &candidates,
            &preferences,
            &window,
            capture_enabled,
        )
        .await;
        jev_row = row;
        let (flagged, proposal, inp, out, _raw) = review
            // The recall hop above already ran and was billed; fold it in so
            // the whole boundary's spend survives a review failure (020).
            .map_err(|error| error.metered(input_tokens, 0))?;
        input_tokens += inp;
        let cost_usd = telemetry::cost_usd(&deps.model, inp, out);

        // 017: store an accepted capture proposal — entirely fail-open; the
        // turn's verdict is never affected by capture (FR-008).
        if let Some(proposal) = proposal {
            let stored = match store_capture(deps, &params.session_id, proposal, caller).await {
                Ok(stored) => stored,
                Err(error) => {
                    tracing::warn!(%error, "capture storage failed open");
                    false
                }
            };
            // The Jev label counts a capture only once it is stored (§7.1).
            if let Some(label) = jev_row.as_mut().and_then(|j| j.row.label_json.as_mut()) {
                if label["captured"].is_boolean() {
                    label["captured"] = serde_json::Value::Bool(stored);
                }
            }
        }

        let (fired, delivered_keys, result) =
            deliver_turn_flags(deps, &params.session_id, flagged, started).await?;
        Ok::<Evaluated, AppError>(Evaluated {
            signal_kinds: evaluated_kinds.clone(),
            fired,
            delivered_keys,
            review_ran: true,
            result,
            cost_usd,
            input_tokens,
            output_tokens: out,
        })
    }
    .await;

    let evaluation = recover(evaluation, evaluated_kinds, started, "checkpoint_turn");
    record_with_jev(
        deps,
        Boundary::Turn,
        &params.session_id,
        &evaluation,
        caller,
        eval_id,
        jev_row,
    )
    .await?;
    if let (Some(signals), Some(eval_id), true) =
        (&deps.turn_signals, signals_eval_id.as_deref(), has_final)
    {
        crate::checkpoint::signals::record_turn(
            signals,
            deps.storage.as_ref(),
            deps.clock.now(),
            eval_id,
            params,
            turn_lines.as_deref(),
        )
        .await?;
    }
    Ok((
        evaluation.result,
        evaluation.input_tokens,
        evaluation.output_tokens,
    ))
}

/// What one review hop returns: flags, capture proposal, tokens in and out,
/// and the raw answers.
type ReviewOutput = (
    Vec<(Signal, String)>,
    Option<review::CaptureProposal>,
    u64,
    u64,
    review::ReviewRaw,
);

/// 064: run the review hop and, when the turn site is in shadow, Jev beside
/// it (joined, never instead of it). The Jev row is built whatever either
/// does; its label comes from the review's raw answers when the review
/// succeeded.
async fn review_beside_jev(
    deps: &CheckpointDeps,
    params: &CheckpointTurnParams,
    eval_id: &str,
    candidates: &[review::Candidate],
    preferences: &[preference::PreferenceCandidate],
    window: &crate::checkpoint::trajectory::TrajectoryWindow,
    capture_enabled: bool,
) -> (Result<ReviewOutput, AppError>, Option<TurnJevOutcome>) {
    let activity = review::turn_activity(window);
    let review = review::review_once(
        deps.model_client.as_ref(),
        &deps.review_mode,
        candidates,
        preferences,
        &params.final_message,
        &activity,
        capture_enabled,
    );
    let Some(jev) = &deps.jev else {
        return (review.await, None);
    };
    let shadow = Box::pin(crate::jev::shadow::shadow_turn(
        jev,
        crate::jev::shadow::TurnInputs {
            window,
            activity: &activity,
            final_message: &params.final_message,
            capture_enabled,
        },
        crate::jev::shadow::TurnIds {
            eval_id,
            harness_session_id: &params.session_id,
            transcript_path: &params.transcript_path,
            now: deps.clock.now(),
        },
    ));
    // Each side notes when it finished, so the wait beyond the review is known.
    let review = async {
        let result = review.await;
        (result, Instant::now())
    };
    let shadow = async {
        let row = shadow.await;
        (row, Instant::now())
    };
    let ((review, review_done), (mut row, jev_done)) = tokio::join!(review, shadow);
    let wait_ms = u64::try_from(jev_done.saturating_duration_since(review_done).as_millis())
        .unwrap_or(u64::MAX);
    if let Ok((flags, _, _, _, raw)) = &review {
        let memory_pairs = candidates
            .iter()
            .any(|c| c.origin == review::CandidateOrigin::Memory);
        let flagged = |kind: SignalKind| flags.iter().any(|(signal, _)| signal.kind == kind);
        row.label_json = Some(crate::jev::shadow::turn_label(
            raw,
            memory_pairs,
            capture_enabled,
            crate::jev::shadow::ReviewOutcome {
                contradiction_flagged: flagged(SignalKind::SelfContradiction),
                violation_flagged: flagged(SignalKind::PreferenceViolation),
                // Whether a capture was stored is known only after the store
                // step in `run_turn`, which sets it there.
                captured: false,
            },
        ));
        row.label_source = Some(format!("{}/{}", deps.model, review::REVIEW_MODE_ID));
    }
    (
        review,
        Some(TurnJevOutcome {
            row: Box::new(row),
            wait_ms,
        }),
    )
}

/// The turn's Jev row and how long the turn waited on Jev after the review
/// finished (064).
struct TurnJevOutcome {
    // Boxed: the row is large, and carrying it by value through the record
    // step pushed the turn future past the large-future lint (064).
    row: Box<crate::storage::jev::JevRecord>,
    wait_ms: u64,
}

/// Turn-flag delivery (015 research D6): one message in delivery order
/// (contradiction first), per-key cooldown via `unsuppressed` — a
/// suppressed signal's message part is dropped, not redelivered.
async fn deliver_turn_flags(
    deps: &CheckpointDeps,
    session_id: &str,
    flagged: Vec<(Signal, String)>,
    started: Instant,
) -> Result<(Vec<Signal>, Vec<String>, CheckpointResult), AppError> {
    if flagged.is_empty() {
        return Ok((
            vec![],
            vec![],
            CheckpointResult::silence(elapsed_ms(started)),
        ));
    }
    let fired: Vec<Signal> = flagged.iter().map(|(signal, _)| signal.clone()).collect();
    let remaining = unsuppressed(deps, session_id, &fired).await?;
    if remaining.is_empty() {
        let result = CheckpointResult::suppressed(&fired, elapsed_ms(started));
        return Ok((fired, vec![], result));
    }
    let remaining_keys: std::collections::HashSet<&str> =
        remaining.iter().map(|s| s.signal_key.as_str()).collect();
    let message = flagged
        .iter()
        .filter(|(signal, _)| remaining_keys.contains(signal.signal_key.as_str()))
        .map(|(_, message)| message.as_str())
        .collect::<Vec<&str>>()
        .join(" ");
    let delivered_keys = remaining.iter().map(|s| s.signal_key.clone()).collect();
    let result = CheckpointResult::flag(message, &remaining, elapsed_ms(started));
    Ok((fired, delivered_keys, result))
}

/// Store one capture proposal (017 research D6): cap check → quarantined
/// memory (`external = true` ⇒ `Untrusted` via the existing derivation) →
/// one consolidation audit row. Returns whether the memory was stored.
/// Callers treat every error as fail-open.
async fn store_capture(
    deps: &CheckpointDeps,
    session_id: &str,
    proposal: review::CaptureProposal,
    caller: Option<&CallerTrace>,
) -> Result<bool, AppError> {
    use crate::memory::consolidate::{
        ConsolidationAction, ConsolidationRecord, CAPTURE_SESSION_CAP,
    };

    let Some(embedder) = deps.embedder.as_ref() else {
        return Ok(false); // capture is memory-gated; the hop was told DISABLED
    };
    let proposal_id = uuid::Uuid::new_v4().to_string();
    let capped = deps.storage.captures_in_session(session_id).await? >= CAPTURE_SESSION_CAP;
    if capped {
        let record = ConsolidationRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: Some(session_id.to_string()),
            action: ConsolidationAction::CaptureDropped,
            source_id: proposal_id,
            target_id: None,
            basis: proposal.basis,
            created_at: deps.clock.now(),
        };
        crate::observability::emit_consolidation(&record, caller);
        return deps
            .storage
            .record_consolidation(&record)
            .await
            .map(|()| false);
    }

    let embedding = embedder.embed_document(&proposal.content).await?;
    let now = deps.clock.now();
    let memory = crate::memory::Memory {
        id: proposal_id.clone(),
        content: proposal.content,
        kind: proposal.kind,
        origin: format!("auto-capture: session {session_id}, end-of-turn"),
        // Model-authored content the user did not write is not first-hand
        // experience — quarantine falls out of the existing derivation
        // (research D6): external + unverified ⇒ Untrusted.
        external: true,
        trust: crate::memory::Trust::Untrusted,
        tags: vec![],
        embedding: embedding.vector,
        embedding_model: embedder.model_id().to_string(),
        created_at: now,
        status: crate::memory::Status::Active,
        replaced_by: None,
        last_reinforced_at: now,
    };
    deps.storage.save_memory(&memory).await?;
    let record = ConsolidationRecord {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: Some(session_id.to_string()),
        action: ConsolidationAction::CaptureProposed,
        source_id: proposal_id,
        target_id: None,
        basis: proposal.basis,
        created_at: now,
    };
    crate::observability::emit_consolidation(&record, caller);
    // The memory is stored; a failed audit row does not undo that.
    if let Err(error) = deps.storage.record_consolidation(&record).await {
        tracing::warn!(%error, "capture audit row failed open");
    }
    Ok(true)
}

/// Recall for the turn review: final message embedded, memories ranked.
async fn turn_recall(
    deps: &CheckpointDeps,
    final_message: &str,
) -> Result<(Vec<(f32, crate::memory::Memory)>, u64), AppError> {
    let Some(embedder) = deps.embedder.as_ref() else {
        return Ok((vec![], 0));
    };
    let query = embedder.embed_query(final_message).await?;
    let memories = deps.storage.load_memories().await?;
    Ok((
        review::rank_recall(&query.vector, &memories),
        query.input_tokens,
    ))
}

/// FR-008: an evaluation failure becomes a recorded fail-open silence.
fn recover(
    evaluation: Result<Evaluated, AppError>,
    evaluated: Vec<SignalKind>,
    started: Instant,
    boundary: &str,
) -> Evaluated {
    evaluation.unwrap_or_else(|error| {
        tracing::warn!(boundary, %error, "checkpoint evaluation failed open");
        let mut failed = Evaluated::pure(
            evaluated,
            vec![],
            CheckpointResult::fail_open(elapsed_ms(started)),
        );
        failed.review_ran = false;
        // 020: failing open changes the verdict, not the bill. Whatever the
        // boundary spent before it failed is carried onto the recovered
        // evaluation, so the checkpoint record shows the cost of an evaluation
        // that silently produced nothing.
        let (input_tokens, output_tokens) = error.billed();
        failed.input_tokens = input_tokens;
        failed.output_tokens = output_tokens;
        failed
    })
}

/// FR-010: drop signals whose key was delivered within the cooldown window.
async fn unsuppressed(
    deps: &CheckpointDeps,
    session_id: &str,
    fired: &[Signal],
) -> Result<Vec<Signal>, AppError> {
    let since = deps.clock.now() - Duration::milliseconds(COOLDOWN_WINDOW_MS);
    let delivered = deps
        .storage
        .delivered_signal_keys_since(session_id, since)
        .await?;
    Ok(fired
        .iter()
        .filter(|signal| !delivered.contains(&signal.signal_key))
        .cloned()
        .collect())
}

/// Exactly one audit row per evaluation (FR-006).
async fn record(
    deps: &CheckpointDeps,
    boundary: Boundary,
    session_id: &str,
    evaluation: &Evaluated,
    caller: Option<&CallerTrace>,
) -> Result<(), AppError> {
    record_with_jev(
        deps,
        boundary,
        session_id,
        evaluation,
        caller,
        uuid::Uuid::new_v4().to_string(),
        None,
    )
    .await
}

/// The one record step (064): the checkpoint record under `eval_id`, then
/// the Jev row, if any. A failed Jev write is logged and never affects the
/// turn; the checkpoint write still propagates as before.
async fn record_with_jev(
    deps: &CheckpointDeps,
    boundary: Boundary,
    session_id: &str,
    evaluation: &Evaluated,
    caller: Option<&CallerTrace>,
    eval_id: String,
    jev: Option<TurnJevOutcome>,
) -> Result<(), AppError> {
    let jev_summary = jev.as_ref().map(|j| {
        crate::observability::JevSpanSummary::from_rows(
            std::slice::from_ref(&j.row),
            j.wait_ms,
            crate::traits::classifier::JevMode::Shadow,
        )
    });
    let jev_row = jev.map(|j| *j.row);
    let row = CheckpointRecord {
        id: eval_id,
        session_id: session_id.to_string(),
        boundary,
        signals_evaluated: evaluation.signal_kinds.clone(),
        signals_fired: evaluation.fired.clone(),
        delivered_keys: evaluation.delivered_keys.clone(),
        review_ran: evaluation.review_ran,
        verdict: evaluation.result.verdict,
        suppressed: evaluation.result.suppressed,
        fail_open: evaluation.result.fail_open,
        latency_ms: evaluation.result.latency_ms,
        cost_usd: evaluation.cost_usd,
        jev_cost_usd: jev_row.as_ref().and_then(|r| r.cost_usd),
        created_at: deps.clock.now(),
    };
    // One measurement, two sinks (007 FR-009) — same value, same exit point.
    crate::observability::emit_checkpoint(&row, caller, jev_summary.as_ref());
    deps.storage.record_checkpoint(&row).await?;
    if let Some(jev_row) = jev_row {
        if let Err(error) = deps
            .storage
            .record_jev(std::slice::from_ref(&jev_row))
            .await
        {
            tracing::warn!(%error, "jev row write failed; the turn is unaffected");
        }
    }
    Ok(())
}

/// The opening text of the batch flag; the turn-start finder skips any
/// transcript line that contains it (066 FR-014).
pub const BATCH_FLAG_PREFIX: &str = "Trajectory checkpoint (automated): ";

/// The opening text of the gate hold; the turn-start finder skips any
/// transcript line that contains it (066 FR-014).
pub const HOLD_PREFIX: &str = "Checkpoint hold (automated): ";

/// Fixed message templates (FR-005/SC-007): parameterized only by evidence.
pub(crate) fn batch_flag_message(signals: &[Signal]) -> String {
    let evidence: Vec<&str> = signals.iter().map(|s| s.evidence.as_str()).collect();
    format!(
        "{BATCH_FLAG_PREFIX}{}. This pattern suggests the current \
         approach is looping — change the approach (or call the unstick corrective) \
         instead of repeating it.",
        evidence.join("; ")
    )
}

pub(crate) fn hold_message(memory_content: &str) -> String {
    format!(
        "{HOLD_PREFIX}this action appears to conflict with a stored, \
         verified constraint: \"{memory_content}\". Confirm to run it unchanged, or \
         deny it to let the agent course-correct."
    )
}

#[allow(clippy::cast_possible_truncation)] // checkpoint latencies are far below u64::MAX ms
fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // 066 T005: the batch flag and the hold start with their exported prefixes.
    #[test]
    fn the_batch_flag_and_the_hold_start_with_their_exported_prefixes() {
        let signal = Signal::new(SignalKind::Repetition, "e".into(), "k");
        assert!(batch_flag_message(&[signal]).starts_with(BATCH_FLAG_PREFIX));
        assert!(hold_message("c").starts_with(HOLD_PREFIX));
    }
    use crate::checkpoint::trajectory::{TrajectoryEntry, TrajectoryWindow};
    use crate::checkpoint::Verdict;
    use crate::modes::ModeRegistry;
    use crate::traits::client::{Completion, MockModelClient};
    use crate::traits::clock::MockTimeProvider;
    use crate::traits::embedder::{Embedding, MockEmbedder};
    use crate::traits::storage::MockStorage;
    use crate::traits::trajectory::MockTrajectoryReader;
    use chrono::{DateTime, Utc};
    use serde_json::json;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-12T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn review_mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        review::register(&mut registry).unwrap();
        registry.get(review::REVIEW_MODE_ID).unwrap().clone()
    }

    fn looping_window() -> TrajectoryWindow {
        TrajectoryWindow {
            session_id: "s1".into(),
            entries: (1..=4)
                .map(|i| TrajectoryEntry::ToolCall {
                    batch_index: i,
                    tool_name: "bash".into(),
                    normalized_input: "{command=cargo test;}".into(),
                    failed: false,
                })
                .collect(),
        }
    }

    struct DepsBuilder {
        reader: MockTrajectoryReader,
        storage: MockStorage,
        clock: MockTimeProvider,
        client: MockModelClient,
        embedder: Option<Arc<dyn Embedder>>,
        patterns: Vec<String>,
        jev: Option<crate::jev::shadow::TurnJev>,
        turn_signals: Option<crate::checkpoint::signals::TurnSignalsDeps>,
    }

    impl DepsBuilder {
        fn new() -> Self {
            let mut clock = MockTimeProvider::new();
            clock.expect_now().return_const(fixed_now());
            Self {
                reader: MockTrajectoryReader::new(),
                storage: MockStorage::new(),
                clock,
                client: MockModelClient::new(),
                embedder: None,
                patterns: vec![],
                jev: None,
                turn_signals: None,
            }
        }

        fn build(self) -> CheckpointDeps {
            CheckpointDeps {
                reader: Arc::new(self.reader),
                storage: Arc::new(self.storage),
                clock: Arc::new(self.clock),
                model_client: Arc::new(self.client),
                review_mode: review_mode(),
                model: "claude-opus-4-8".into(),
                embedder: self.embedder,
                gate_extra_patterns: self.patterns,
                jev: self.jev,
                turn_signals: self.turn_signals,
            }
        }
    }

    fn batch_params() -> CheckpointBatchParams {
        CheckpointBatchParams {
            session_id: "s1".into(),
            transcript_path: "t.jsonl".into(),
        }
    }

    #[tokio::test]
    async fn batch_flags_a_loop_and_records_exactly_once() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((looping_window(), vec![])));
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(|_, _| Ok(vec![]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.boundary == Boundary::Batch
                    && r.verdict == Verdict::Flag
                    && !r.suppressed
                    && !r.fail_open
                    && r.signals_fired.len() == 1
            })
            .returning(|_| Ok(()));

        let (result, _, _) = run_batch(&b.build(), &batch_params(), None).await.unwrap();
        assert_eq!(result.verdict, Verdict::Flag);
        let message = result.message.unwrap();
        assert!(message.contains("cargo test"), "{message}");
        assert!(message.contains("4 times"), "{message}");
    }

    #[tokio::test]
    async fn batch_cooldown_suppresses_an_already_delivered_signal() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((looping_window(), vec![])));
        // The same signal key was delivered recently.
        let key = screen::screen(&looping_window())[0].signal_key.clone();
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(move |_, _| Ok(vec![key.clone()]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && r.suppressed)
            .returning(|_| Ok(()));

        let (result, _, _) = run_batch(&b.build(), &batch_params(), None).await.unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(result.suppressed);
        assert!(result.message.is_none());
    }

    #[tokio::test]
    async fn batch_fails_open_on_a_reader_error_and_still_records() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Err(AppError::ValidationFailure("no transcript".into())));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.fail_open && r.verdict == Verdict::Silence)
            .returning(|_| Ok(()));

        let (result, _, _) = run_batch(&b.build(), &batch_params(), None).await.unwrap();
        assert!(result.fail_open);
        assert_eq!(result.verdict, Verdict::Silence);
    }

    // T049 / 018 FR-015, US3 scenario 4: a routed review model that cannot be
    // reached must not block the turn. The checkpoint layer's fail-open
    // guarantee has to survive routing — the review hop is a routable call
    // site, so an operator pointing it at an unreachable model is now a
    // reachable state.
    #[tokio::test]
    async fn turn_fails_open_when_the_routed_review_model_is_unreachable() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((reversal_window(), vec![])));
        // The routed review client is unreachable.
        b.client.expect_complete().returning(|_, _| {
            Err(AppError::RetriesExhausted {
                attempts: 4,
                last: "provider unreachable".into(),
            })
        });
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(|_, _| Ok(vec![]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.fail_open && r.verdict == Verdict::Silence)
            .returning(|_| Ok(()));

        let mut deps = b.build();
        // The model the operator routed this call site to.
        deps.model = "unreachable-model".into();

        let (result, _, _) = run_turn(
            &deps,
            &turn_params(
                "On reflection the database migration is not reversible in practice.",
                false,
            ),
            None,
        )
        .await
        .unwrap();

        // Silence, recorded, turn unblocked — never an error the caller sees.
        assert!(result.fail_open);
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(result.message.is_none());
    }

    // T047 / 018 C3: the checkpoint layer keeps its own cost record, priced
    // from `CheckpointDeps.model`. That field now carries the ROUTED review
    // model, so a routed review is priced at the routed model's rate.
    #[test]
    fn checkpoint_cost_prices_at_the_routed_model_rate() {
        // Haiku and Opus differ by 5x on input; the field must decide which.
        let haiku = telemetry::cost_usd("claude-haiku-4-5", 1_000_000, 0);
        let opus = telemetry::cost_usd("claude-opus-4-8", 1_000_000, 0);
        assert!(
            (haiku - 1.0).abs() < 1e-9 && (opus - 5.0).abs() < 1e-9,
            "rates differ, so the attributed model is load-bearing"
        );
        // An unpriced routed model still over-estimates rather than
        // under-reports, and says so.
        assert!(!telemetry::pricing_known("unreachable-model"));
        assert!(
            (telemetry::cost_usd("unreachable-model", 1_000_000, 0) - opus).abs() < 1e-9,
            "unknown routed models fall back to Opus-tier, never cheaper"
        );
    }

    fn action_params(tool: &str, input: &str) -> CheckpointActionParams {
        CheckpointActionParams {
            session_id: "s1".into(),
            transcript_path: "t.jsonl".into(),
            tool_name: tool.into(),
            tool_input: input.into(),
        }
    }

    fn constraint_memory() -> crate::memory::Memory {
        crate::memory::Memory {
            id: "m1".into(),
            content: "deployments go through staging first — never straight to production".into(),
            kind: crate::memory::Kind::Lesson,
            origin: "test".into(),
            external: false,
            trust: crate::memory::Trust::FirstHand,
            tags: vec![],
            embedding: vec![1.0, 0.0],
            embedding_model: "voyage-4".into(),
            created_at: fixed_now(),
            status: crate::memory::Status::Active,
            replaced_by: None,
            last_reinforced_at: fixed_now(),
        }
    }

    #[tokio::test]
    async fn gate_holds_a_risky_action_quoting_the_memory() {
        let mut b = DepsBuilder::new();
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_query().returning(|_| {
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 6,
            })
        });
        b.embedder = Some(Arc::new(embedder));
        b.storage
            .expect_load_memories()
            .returning(|| Ok(vec![constraint_memory()]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.boundary == Boundary::Action && r.verdict == Verdict::Hold)
            .returning(|_| Ok(()));

        let (result, inp, _) = run_action(
            &b.build(),
            &action_params("bash", "deploy to production"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Hold);
        assert!(result.message.unwrap().contains("staging first"));
        assert_eq!(inp, 6); // embed usage attributed
    }

    #[tokio::test]
    async fn gate_passes_non_risk_actions_without_any_evaluation() {
        let mut b = DepsBuilder::new();
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_query().times(0);
        b.embedder = Some(Arc::new(embedder));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && r.signals_evaluated.is_empty())
            .returning(|_| Ok(()));

        let (result, _, _) = run_action(&b.build(), &action_params("read", "src/main.rs"), None)
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(!result.fail_open);
    }

    /// An embedder that outlives the gate budget — the timeout test double.
    struct SlowEmbedder;

    #[async_trait::async_trait]
    impl Embedder for SlowEmbedder {
        async fn embed_document(&self, _text: &str) -> Result<Embedding, AppError> {
            unreachable!("the gate never embeds documents")
        }

        async fn embed_query(&self, _text: &str) -> Result<Embedding, AppError> {
            tokio::time::sleep(std::time::Duration::from_millis(GATE_BUDGET_MS + 200)).await;
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 6,
            })
        }

        fn model_id(&self) -> &'static str {
            "voyage-4"
        }
    }

    #[tokio::test]
    async fn gate_fails_open_when_the_budget_elapses() {
        let mut b = DepsBuilder::new();
        b.embedder = Some(Arc::new(SlowEmbedder));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.fail_open)
            .returning(|_| Ok(()));

        let (result, _, _) =
            run_action(&b.build(), &action_params("bash", "git push --force"), None)
                .await
                .unwrap();
        assert!(result.fail_open);
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn gate_without_memory_capability_is_silent_not_failed() {
        let mut b = DepsBuilder::new();
        b.embedder = None;
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.verdict == Verdict::Silence
                    && !r.fail_open
                    && r.signals_evaluated == vec![SignalKind::MemoryConflict]
            })
            .returning(|_| Ok(()));

        let (result, _, _) =
            run_action(&b.build(), &action_params("bash", "git push --force"), None)
                .await
                .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(!result.fail_open);
    }

    fn turn_params(final_message: &str, continuation: bool) -> CheckpointTurnParams {
        CheckpointTurnParams {
            session_id: "s1".into(),
            transcript_path: "t.jsonl".into(),
            final_message: final_message.into(),
            continuation,
        }
    }

    fn reversal_window() -> TrajectoryWindow {
        TrajectoryWindow {
            session_id: "s1".into(),
            entries: vec![TrajectoryEntry::Assistant {
                text: "The database migration is fully reversible and safe to run.".into(),
            }],
        }
    }

    #[tokio::test]
    async fn turn_flags_a_confirmed_contradiction_citing_both_statements() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((reversal_window(), vec![])));
        b.client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({
                    "contradicts": true,
                    "statement_a": "The database migration is fully reversible and safe to run",
                    "statement_b": "the database migration is not reversible",
                    "basis": "Both cannot hold.",
                    "violates": false,
                    "violated_preference": "",
                    "capture_worthy": false,
                    "capture_kind": "none",
                    "capture_content": "",
                    "capture_basis": "",
                    "violation_basis": ""
                }),
                input_tokens: 80,
                output_tokens: 30,
                reported_cost_micros: None,
            })
        });
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(|_, _| Ok(vec![]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.boundary == Boundary::Turn
                    && r.verdict == Verdict::Flag
                    && r.review_ran
                    && r.cost_usd > 0.0
            })
            .returning(|_| Ok(()));

        let (result, inp, out) = run_turn(
            &b.build(),
            &turn_params(
                "After checking, the database migration is not reversible after all.",
                false,
            ),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Flag);
        let message = result.message.unwrap();
        assert!(message.contains("fully reversible"), "{message}");
        assert!(message.contains("not reversible"), "{message}");
        assert_eq!((inp, out), (80, 30));
    }

    #[tokio::test]
    async fn turn_with_no_candidates_never_invokes_the_hop() {
        let mut b = DepsBuilder::new();
        b.reader.expect_read().returning(|_, _| {
            Ok((
                TrajectoryWindow {
                    session_id: "s1".into(),
                    entries: vec![],
                },
                vec![],
            ))
        });
        b.client.expect_complete().times(0);
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && !r.review_ran)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("Everything completed without surprises today.", false),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn turn_screening_cleared_records_review_ran_with_silence() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((reversal_window(), vec![])));
        b.client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({
                    "contradicts": false,
                    "statement_a": "",
                    "statement_b": "",
                    "basis": "The final statement is an evidence-justified update.",
                    "violates": false,
                    "violated_preference": "",
                    "capture_worthy": false,
                    "capture_kind": "none",
                    "capture_content": "",
                    "capture_basis": "",
                    "violation_basis": ""
                }),
                input_tokens: 60,
                output_tokens: 20,
                reported_cost_micros: None,
            })
        });
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && r.review_ran && !r.suppressed)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params(
                "On reflection the database migration is not reversible in practice.",
                false,
            ),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }

    fn preference_memory(
        id: &str,
        content: &str,
        trust: crate::memory::Trust,
    ) -> crate::memory::Memory {
        crate::memory::Memory {
            id: id.into(),
            content: content.into(),
            kind: crate::memory::Kind::Fact,
            origin: "test".into(),
            external: matches!(trust, crate::memory::Trust::Untrusted),
            trust,
            tags: vec![],
            embedding: vec![1.0, 0.0],
            embedding_model: "voyage-4".into(),
            created_at: fixed_now(),
            status: crate::memory::Status::Active,
            replaced_by: None,
            last_reinforced_at: fixed_now(),
        }
    }

    fn matching_embedder() -> MockEmbedder {
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_query().returning(|_| {
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 6,
            })
        });
        embedder
    }

    fn violation_completion() -> Completion {
        Completion {
            value: json!({
                "contradicts": false,
                "statement_a": "",
                "statement_b": "",
                "basis": "No contradiction among the pairs.",
                "violates": true,
                "violated_preference": "final messages must never contain the word delve",
                "capture_worthy": false,
                    "capture_kind": "none",
                    "capture_content": "",
                    "capture_basis": "",
                    "violation_basis": "The final message contains the word delve."
            }),
            input_tokens: 90,
            output_tokens: 35,
            reported_cost_micros: None,
        }
    }

    fn empty_window() -> TrajectoryWindow {
        TrajectoryWindow {
            session_id: "s1".into(),
            entries: vec![],
        }
    }

    /// The expected cooldown key for a violation of `memory_id`.
    fn violation_key(memory_id: &str) -> String {
        let candidate = crate::checkpoint::preference::PreferenceCandidate {
            memory_id: memory_id.into(),
            content: "x".into(),
            trust: crate::memory::Trust::FirstHand,
            score: 1.0,
        };
        crate::checkpoint::preference::violation_signal(&candidate, "y")
            .0
            .signal_key
    }

    // ---- 015 US1: violation flagged at end of turn -------------------------

    #[tokio::test]
    async fn turn_flags_a_preference_violation_quoting_memory_and_provenance() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        b.embedder = Some(Arc::new(matching_embedder()));
        b.storage.expect_load_memories().returning(|| {
            Ok(vec![preference_memory(
                "mem-delve",
                "final messages must never contain the word delve",
                crate::memory::Trust::FirstHand,
            )])
        });
        // FR-010: one hop, both judgments — times(1) is the contract.
        b.client
            .expect_complete()
            .times(1)
            .returning(|_, _| Ok(violation_completion()));
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(|_, _| Ok(vec![]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.boundary == Boundary::Turn
                    && r.verdict == Verdict::Flag
                    && r.review_ran
                    && r.cost_usd > 0.0
                    && r.signals_evaluated
                        == vec![
                            SignalKind::SelfContradiction,
                            SignalKind::PreferenceViolation,
                            SignalKind::CaptureProposal,
                        ]
                    && r.signals_fired.len() == 1
                    && r.signals_fired[0].kind == SignalKind::PreferenceViolation
                    && r.signals_fired[0].evidence.contains("mem-delve")
                    && r.delivered_keys.len() == 1
            })
            .returning(|_| Ok(()));

        let (result, inp, out) = run_turn(
            &b.build(),
            &turn_params("Let me delve into the details of this fix.", false),
            None,
        )
        .await
        .unwrap();
        // FR-003: flag is the maximum authority — never hold.
        assert_eq!(result.verdict, Verdict::Flag);
        let message = result.message.unwrap();
        assert!(
            message.contains("final messages must never contain the word delve"),
            "{message}"
        );
        assert!(message.contains("mem-delve"), "{message}");
        assert!(message.contains("first_hand provenance"), "{message}");
        assert!(message.contains("why it does not apply"), "{message}");
        assert_eq!((inp, out), (96, 35)); // 6 embed + 90 hop input
    }

    #[tokio::test]
    async fn turn_delivers_both_flags_as_one_message_contradiction_first() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((reversal_window(), vec![])));
        b.embedder = Some(Arc::new(matching_embedder()));
        b.storage.expect_load_memories().returning(|| {
            Ok(vec![preference_memory(
                "mem-delve",
                "final messages must never contain the word delve",
                crate::memory::Trust::FirstHand,
            )])
        });
        b.client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({
                    "contradicts": true,
                    "statement_a": "The database migration is fully reversible and safe to run.",
                    "statement_b": "the database migration is not reversible",
                    "basis": "Both cannot hold.",
                    "violates": true,
                    "violated_preference": "final messages must never contain the word delve",
                    "capture_worthy": false,
                    "capture_kind": "none",
                    "capture_content": "",
                    "capture_basis": "",
                    "violation_basis": "The final message contains the word delve."
                }),
                input_tokens: 90,
                output_tokens: 35,
                reported_cost_micros: None,
            })
        });
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(|_, _| Ok(vec![]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.verdict == Verdict::Flag
                    && r.signals_fired.len() == 2
                    && r.signals_fired[0].kind == SignalKind::SelfContradiction
                    && r.signals_fired[1].kind == SignalKind::PreferenceViolation
                    && r.delivered_keys.len() == 2
            })
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params(
                "I must delve deeper: the database migration is not reversible after all.",
                false,
            ),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Flag);
        let message = result.message.unwrap();
        let contradiction_at = message
            .find("contradicts an earlier committed statement")
            .unwrap();
        let violation_at = message.find("violate a stored preference").unwrap();
        assert!(contradiction_at < violation_at, "{message}");
    }

    // ---- 015 US2: enforcement never degrades a session ---------------------

    #[tokio::test]
    async fn turn_without_memory_lists_only_the_contradiction_signal() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        b.client.expect_complete().times(0);
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.verdict == Verdict::Silence
                    && !r.fail_open
                    && r.signals_evaluated == vec![SignalKind::SelfContradiction]
            })
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("Everything completed without surprises today.", false),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn turn_fails_open_when_recall_errors_and_still_records() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        let mut embedder = MockEmbedder::new();
        embedder
            .expect_embed_query()
            .returning(|_| Err(AppError::ValidationFailure("embedding backend down".into())));
        b.embedder = Some(Arc::new(embedder));
        b.client.expect_complete().times(0);
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.fail_open && r.verdict == Verdict::Silence)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("A perfectly ordinary final message for the turn.", false),
            None,
        )
        .await
        .unwrap();
        assert!(result.fail_open);
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn turn_with_only_untrusted_memories_never_mines_preference_candidates() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        b.embedder = Some(Arc::new(matching_embedder()));
        b.storage.expect_load_memories().returning(|| {
            Ok(vec![preference_memory(
                "mem-untrusted",
                "final messages must never contain the word delve",
                crate::memory::Trust::Untrusted,
            )])
        });
        // 015 FR-005: untrusted is structurally excluded from preference
        // mining. The hop still runs once (017: capture is an every-turn
        // judgment with memory on) — with NO preferences listed, so a
        // violates=true answer would be ignored anyway; the mock declines.
        b.client.expect_complete().times(1).returning(|prompt, _| {
            assert!(prompt.contains("Stored preferences:\n(none)"), "{prompt}");
            Ok(Completion {
                value: json!({
                    "contradicts": false,
                    "statement_a": "",
                    "statement_b": "",
                    "basis": "Nothing to compare.",
                    "violates": false,
                    "violated_preference": "",
                    "violation_basis": "",
                    "capture_worthy": false,
                    "capture_kind": "none",
                    "capture_content": "",
                    "capture_basis": ""
                }),
                input_tokens: 30,
                output_tokens: 10,
                reported_cost_micros: None,
            })
        });
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.verdict == Verdict::Silence
                    && r.review_ran
                    && !r.fail_open
                    && r.signals_fired.is_empty()
            })
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("Let me delve into the details of this fix.", false),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }

    // ---- 017 US3: end-of-turn capture --------------------------------------

    fn capture_completion() -> Completion {
        Completion {
            value: json!({
                "contradicts": false,
                "statement_a": "",
                "statement_b": "",
                "basis": "Nothing to compare.",
                "violates": false,
                "violated_preference": "",
                "violation_basis": "",
                "capture_worthy": true,
                "capture_kind": "lesson",
                "capture_content": "the staging deploy fails unless the cache is cleared first",
                "capture_basis": "The turn diagnosed and fixed exactly this failure."
            }),
            input_tokens: 70,
            output_tokens: 30,
            reported_cost_micros: None,
        }
    }

    #[tokio::test]
    async fn turn_stores_a_capture_proposal_quarantined_with_one_audit_row() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        let mut embedder = matching_embedder();
        embedder.expect_embed_document().times(1).returning(|_| {
            Ok(Embedding {
                vector: vec![0.5, 0.5],
                input_tokens: 9,
            })
        });
        embedder
            .expect_model_id()
            .return_const("voyage-4".to_string());
        b.embedder = Some(Arc::new(embedder));
        b.storage.expect_load_memories().returning(|| Ok(vec![]));
        b.client
            .expect_complete()
            .times(1)
            .returning(|_, _| Ok(capture_completion()));
        b.storage
            .expect_captures_in_session()
            .withf(|s| s == "s1")
            .returning(|_| Ok(0));
        // Quarantine (research D6): external=true => Untrusted; origin names
        // the session; content verbatim from the hop.
        b.storage
            .expect_save_memory()
            .times(1)
            .withf(|m| {
                m.trust == crate::memory::Trust::Untrusted
                    && m.external
                    && m.kind == crate::memory::Kind::Lesson
                    && m.origin.contains("auto-capture: session s1")
                    && m.content == "the staging deploy fails unless the cache is cleared first"
                    && m.status == crate::memory::Status::Active
            })
            .returning(|_| Ok(()));
        b.storage
            .expect_record_consolidation()
            .times(1)
            .withf(|r| {
                r.action == crate::memory::consolidate::ConsolidationAction::CaptureProposed
                    && r.session_id.as_deref() == Some("s1")
            })
            .returning(|_| Ok(()));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.verdict == Verdict::Silence
                    && r.review_ran
                    && r.signals_evaluated.contains(&SignalKind::CaptureProposal)
            })
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params(
                "Fixed it: the cache had to be cleared before the deploy.",
                false,
            ),
            None,
        )
        .await
        .unwrap();
        // Capture never affects the verdict (FR-008).
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn turn_drops_capture_proposals_at_the_session_cap() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        b.embedder = Some(Arc::new(matching_embedder()));
        b.storage.expect_load_memories().returning(|| Ok(vec![]));
        b.client
            .expect_complete()
            .times(1)
            .returning(|_, _| Ok(capture_completion()));
        b.storage
            .expect_captures_in_session()
            .returning(|_| Ok(crate::memory::consolidate::CAPTURE_SESSION_CAP));
        b.storage.expect_save_memory().times(0);
        b.storage
            .expect_record_consolidation()
            .times(1)
            .withf(|r| r.action == crate::memory::consolidate::ConsolidationAction::CaptureDropped)
            .returning(|_| Ok(()));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params(
                "Another fix worth remembering, but the cap is reached.",
                false,
            ),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn capture_storage_failure_never_affects_the_turn_verdict() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        let mut embedder = matching_embedder();
        embedder
            .expect_embed_document()
            .returning(|_| Err(AppError::ValidationFailure("embed down".into())));
        b.embedder = Some(Arc::new(embedder));
        b.storage.expect_load_memories().returning(|| Ok(vec![]));
        b.client
            .expect_complete()
            .times(1)
            .returning(|_, _| Ok(capture_completion()));
        b.storage.expect_captures_in_session().returning(|_| Ok(0));
        b.storage.expect_save_memory().times(0);
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && !r.fail_open)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("A fix that would be captured if embedding worked.", false),
            None,
        )
        .await
        .unwrap();
        // FR-008: fail-open — the turn proceeds; the verdict is untouched.
        assert_eq!(result.verdict, Verdict::Silence);
    }

    // ---- 015 US3: audit + cooldown by memory id ----------------------------

    #[tokio::test]
    async fn turn_suppresses_a_recently_delivered_violation_by_memory_id() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        b.embedder = Some(Arc::new(matching_embedder()));
        b.storage.expect_load_memories().returning(|| {
            Ok(vec![preference_memory(
                "mem-delve",
                "final messages must never contain the word delve",
                crate::memory::Trust::FirstHand,
            )])
        });
        b.client
            .expect_complete()
            .times(1)
            .returning(|_, _| Ok(violation_completion()));
        // The same memory's violation was delivered within the window.
        let delivered = violation_key("mem-delve");
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(move |_, _| Ok(vec![delivered.clone()]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.verdict == Verdict::Silence
                    && r.suppressed
                    && r.delivered_keys.is_empty()
                    && r.signals_fired.len() == 1
            })
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("Let me delve into the details of this fix.", false),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(result.suppressed);
    }

    #[tokio::test]
    async fn turn_cooldown_is_per_key_a_different_memory_still_delivers() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok((empty_window(), vec![])));
        b.embedder = Some(Arc::new(matching_embedder()));
        b.storage.expect_load_memories().returning(|| {
            Ok(vec![preference_memory(
                "mem-delve",
                "final messages must never contain the word delve",
                crate::memory::Trust::FirstHand,
            )])
        });
        b.client
            .expect_complete()
            .times(1)
            .returning(|_, _| Ok(violation_completion()));
        // A DIFFERENT memory's violation is in cooldown — this one delivers.
        let delivered = violation_key("mem-other");
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(move |_, _| Ok(vec![delivered.clone()]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Flag && !r.suppressed && r.delivered_keys.len() == 1)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("Let me delve into the details of this fix.", false),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Flag);
    }

    #[tokio::test]
    async fn continuation_turns_are_screening_only_and_never_review() {
        let mut b = DepsBuilder::new();
        b.reader.expect_read().times(0);
        b.client.expect_complete().times(0);
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && !r.review_ran)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("Reconciled: the migration is not reversible.", true),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }

    mod turn_signals;

    // ---- 064 T019: the turn shadow ------------------------------------------

    mod jev_shadow {
        use super::*;
        use crate::error::Outcome;
        use crate::jev::redact::Redactor;
        use crate::jev::shadow::TurnJev;
        use crate::storage::jev::{JevOutcome, JevRecord};
        use crate::traits::classifier::{
            Classifier, JevAnswer, JevRequest, JevResponse, MockClassifier,
        };
        use std::sync::Mutex;
        use std::time::Duration as StdDuration;

        const PINNED: &str = "jev-1.14.0";
        const REVERSAL_FINAL: &str =
            "After checking, the database migration is not reversible after all.";

        type Shared<T> = Arc<Mutex<Vec<T>>>;

        /// What one turn wrote and what Jev was asked.
        struct Written {
            checkpoints: Shared<CheckpointRecord>,
            jev_rows: Shared<JevRecord>,
        }

        impl Written {
            fn new() -> Self {
                Self {
                    checkpoints: Arc::default(),
                    jev_rows: Arc::default(),
                }
            }
            fn checkpoint(&self) -> CheckpointRecord {
                let all = self.checkpoints.lock().unwrap();
                assert_eq!(all.len(), 1, "one checkpoint record per turn");
                all[0].clone()
            }
            fn rows(&self) -> Vec<JevRecord> {
                self.jev_rows.lock().unwrap().clone()
            }
        }

        fn turn_jev(classifier: Arc<dyn Classifier>) -> TurnJev {
            TurnJev {
                classifier,
                redactor: Redactor::new(vec![]),
                model: PINNED.into(),
                server_session_id: "server-1".into(),
                deadline: StdDuration::from_millis(50),
            }
        }

        /// Answers every question asked at 0.9, and keeps the request.
        fn confident(requests: Shared<JevRequest>, model: &'static str) -> MockClassifier {
            let mut mock = MockClassifier::new();
            mock.expect_model_id().return_const(PINNED.to_string());
            mock.expect_classify().returning(move |request| {
                requests.lock().unwrap().push(request.clone());
                Ok(JevResponse {
                    model_answered: model.into(),
                    answers: request
                        .questions
                        .iter()
                        .map(|q| (q.id.clone(), JevAnswer::Noul { p: 0.9 }))
                        .collect(),
                    input_tokens: Some(200),
                    request_id: Some("req-1".into()),
                    latency_ms: 90,
                })
            });
            mock
        }

        fn failing() -> MockClassifier {
            let mut mock = MockClassifier::new();
            mock.expect_model_id().return_const(PINNED.to_string());
            mock.expect_classify()
                .returning(|_| Err(AppError::ClassifierProvider("typesafe 529".into())));
            mock
        }

        /// A Jev that never answers.
        struct Hanging;

        #[async_trait::async_trait]
        impl Classifier for Hanging {
            async fn classify(&self, _: &JevRequest) -> Result<JevResponse, AppError> {
                tokio::time::sleep(StdDuration::from_secs(3600)).await;
                Err(AppError::ClassifierProvider("unreachable".into()))
            }
            fn model_id(&self) -> &str {
                PINNED
            }
        }

        fn contradiction_completion() -> Completion {
            Completion {
                value: json!({
                    "contradicts": true,
                    "statement_a": "The database migration is fully reversible and safe to run",
                    "statement_b": "the database migration is not reversible",
                    "basis": "Both cannot hold.",
                    "violates": false,
                    "violated_preference": "",
                    "capture_worthy": false,
                    "capture_kind": "none",
                    "capture_content": "",
                    "capture_basis": "",
                    "violation_basis": ""
                }),
                input_tokens: 80,
                output_tokens: 30,
                reported_cost_micros: None,
            }
        }

        /// A builder whose storage captures everything written, in order.
        fn capturing(written: &Written) -> DepsBuilder {
            let mut b = DepsBuilder::new();
            let checkpoints = Arc::clone(&written.checkpoints);
            let rows = Arc::clone(&written.jev_rows);
            let mut sequence = mockall::Sequence::new();
            b.storage
                .expect_delivered_signal_keys_since()
                .returning(|_, _| Ok(vec![]));
            b.storage
                .expect_record_checkpoint()
                .times(1)
                .in_sequence(&mut sequence)
                .returning(move |r| {
                    checkpoints.lock().unwrap().push(r.clone());
                    Ok(())
                });
            b.storage
                .expect_record_jev()
                .times(0..=1)
                .in_sequence(&mut sequence)
                .returning(move |r| {
                    rows.lock().unwrap().extend_from_slice(r);
                    Ok(())
                });
            b
        }

        /// The reversal turn with this Jev; returns the result as JSON.
        async fn reversal_turn(written: &Written, jev: Option<TurnJev>) -> serde_json::Value {
            let mut b = capturing(written);
            b.reader
                .expect_read()
                .returning(|_, _| Ok((reversal_window(), vec![])));
            b.client
                .expect_complete()
                .times(1)
                .returning(|_, _| Ok(contradiction_completion()));
            b.jev = jev;
            let (result, _, _) = run_turn(&b.build(), &turn_params(REVERSAL_FINAL, false), None)
                .await
                .unwrap();
            let mut value = serde_json::to_value(&result).unwrap();
            value.as_object_mut().unwrap().remove("latency_ms");
            value
        }

        fn comparable(mut record: CheckpointRecord) -> CheckpointRecord {
            record.id = String::new();
            record.latency_ms = 0;
            record.jev_cost_usd = None;
            record
        }

        #[tokio::test]
        async fn the_turn_and_its_record_are_identical_whatever_jev_does() {
            let off = Written::new();
            let result_off = reversal_turn(&off, None).await;
            assert!(off.rows().is_empty());

            let cases: Vec<(&str, Arc<dyn Classifier>, Outcome)> = vec![
                (
                    "confident",
                    Arc::new(confident(Arc::default(), PINNED)),
                    Outcome::Success,
                ),
                ("errors", Arc::new(failing()), Outcome::ClassifierProvider),
                (
                    "rejected",
                    Arc::new(confident(Arc::default(), "jev-9.9.9")),
                    Outcome::ValidationFailure,
                ),
                ("never returns", Arc::new(Hanging), Outcome::Timeout),
            ];
            for (name, classifier, outcome) in cases {
                let on = Written::new();
                let started = std::time::Instant::now();
                let result_on = reversal_turn(&on, Some(turn_jev(classifier))).await;
                assert!(
                    started.elapsed() < StdDuration::from_secs(5),
                    "{name}: waited"
                );
                assert_eq!(result_on, result_off, "{name}: result changed");
                assert_eq!(
                    comparable(on.checkpoint()),
                    comparable(off.checkpoint()),
                    "{name}: record changed"
                );
                let rows = on.rows();
                assert_eq!(rows.len(), 1, "{name}");
                assert_eq!(rows[0].outcome, JevOutcome::Call(outcome), "{name}");
            }
        }

        #[tokio::test]
        async fn the_row_links_to_its_checkpoint_record_and_carries_the_label() {
            let written = Written::new();
            let requests: Shared<JevRequest> = Arc::default();
            reversal_turn(
                &written,
                Some(turn_jev(Arc::new(confident(Arc::clone(&requests), PINNED)))),
            )
            .await;
            let record = written.checkpoint();
            let row = &written.rows()[0];
            assert_eq!(row.eval_id.as_deref(), Some(record.id.as_str()));
            assert_eq!(record.jev_cost_usd, row.cost_usd);
            assert!(row.cost_usd.is_some());
            assert_eq!(
                row.label_json,
                Some(json!({
                    "contradicts": true,
                    "violates": false,
                    "capture_kind": null,
                    "contradiction_flagged": true,
                    "violation_flagged": false,
                    "captured": null,
                }))
            );
            assert_eq!(row.harness_session_id.as_deref(), Some("s1"));
            assert_eq!(row.model_answered.as_deref(), Some(PINNED));
        }

        /// Runs one turn with Jev on and returns the label its row carries.
        async fn label_of(
            mut b: DepsBuilder,
            written: &Written,
            final_message: &str,
        ) -> serde_json::Value {
            b.jev = Some(turn_jev(Arc::new(confident(Arc::default(), PINNED))));
            run_turn(&b.build(), &turn_params(final_message, false), None)
                .await
                .unwrap();
            written.rows()[0].label_json.clone().unwrap()
        }

        #[tokio::test]
        async fn a_violation_flag_is_labeled_as_a_violation_not_a_contradiction() {
            let written = Written::new();
            let mut b = capturing(&written);
            b.reader
                .expect_read()
                .returning(|_, _| Ok((empty_window(), vec![])));
            b.embedder = Some(Arc::new(matching_embedder()));
            b.storage.expect_load_memories().returning(|| {
                Ok(vec![preference_memory(
                    "mem-delve",
                    "final messages must never contain the word delve",
                    crate::memory::Trust::FirstHand,
                )])
            });
            b.client
                .expect_complete()
                .times(1)
                .returning(|_, _| Ok(violation_completion()));
            let label = label_of(b, &written, "Let me delve into the details.").await;
            assert_eq!(label["violation_flagged"], json!(true));
            assert_ne!(label["contradiction_flagged"], json!(true));
            assert_eq!(label["captured"], json!(false));
        }

        fn storing_embedder() -> MockEmbedder {
            let mut embedder = matching_embedder();
            embedder.expect_embed_document().returning(|_| {
                Ok(Embedding {
                    vector: vec![0.5, 0.5],
                    input_tokens: 9,
                })
            });
            embedder
                .expect_model_id()
                .return_const("voyage-4".to_string());
            embedder
        }

        /// A capture turn whose session already holds `captures` captures.
        fn capture_builder(written: &Written, captures: u32) -> DepsBuilder {
            capture_builder_with_audit(written, captures, true)
        }

        /// As [`capture_builder`], with the audit-row write failing when
        /// `audit_ok` is false.
        fn capture_builder_with_audit(
            written: &Written,
            captures: u32,
            audit_ok: bool,
        ) -> DepsBuilder {
            let mut b = capturing(written);
            b.reader
                .expect_read()
                .returning(|_, _| Ok((empty_window(), vec![])));
            b.embedder = Some(Arc::new(storing_embedder()));
            b.storage.expect_load_memories().returning(|| Ok(vec![]));
            b.client
                .expect_complete()
                .times(1)
                .returning(|_, _| Ok(capture_completion()));
            b.storage
                .expect_captures_in_session()
                .returning(move |_| Ok(captures));
            b.storage.expect_save_memory().returning(|_| Ok(()));
            b.storage.expect_record_consolidation().returning(move |_| {
                if audit_ok {
                    Ok(())
                } else {
                    Err(AppError::ValidationFailure("audit write failed".into()))
                }
            });
            b
        }

        #[tokio::test]
        async fn a_saved_capture_whose_audit_row_fails_is_still_labeled_captured() {
            let written = Written::new();
            let b = capture_builder_with_audit(&written, 0, false);
            let label = label_of(b, &written, "Fixed it: clear the cache first.").await;
            assert_eq!(label["captured"], json!(true));
        }

        #[tokio::test]
        async fn a_stored_capture_is_labeled_captured() {
            let written = Written::new();
            let b = capture_builder(&written, 0);
            let label = label_of(b, &written, "Fixed it: clear the cache first.").await;
            assert_eq!(label["captured"], json!(true));
            assert_eq!(label["violation_flagged"], json!(false));
            assert_eq!(label["contradiction_flagged"], json!(false));
        }

        #[tokio::test]
        async fn a_capture_dropped_at_the_session_cap_is_not_labeled_captured() {
            let written = Written::new();
            let b = capture_builder(&written, crate::memory::consolidate::CAPTURE_SESSION_CAP);
            let label = label_of(b, &written, "Another fix, but the cap is reached.").await;
            assert_eq!(label["captured"], json!(false));
        }

        async fn assert_no_jev_call(
            final_message: &str,
            continuation: bool,
            window: TrajectoryWindow,
        ) {
            let written = Written::new();
            let mut b = capturing(&written);
            b.reader
                .expect_read()
                .returning(move |_, _| Ok((window.clone(), vec![])));
            b.client.expect_complete().times(0);
            let mut mock = MockClassifier::new();
            mock.expect_classify().times(0);
            mock.expect_model_id().return_const(PINNED.to_string());
            b.jev = Some(turn_jev(Arc::new(mock)));
            run_turn(&b.build(), &turn_params(final_message, continuation), None)
                .await
                .unwrap();
            assert!(written.rows().is_empty());
        }

        #[tokio::test]
        async fn no_jev_call_when_the_review_does_not_run() {
            assert_no_jev_call(REVERSAL_FINAL, true, reversal_window()).await;
            assert_no_jev_call("   ", false, reversal_window()).await;
            assert_no_jev_call(
                "Everything completed without surprises today.",
                false,
                empty_window(),
            )
            .await;
        }

        #[tokio::test]
        async fn a_review_that_fails_open_still_writes_a_row_with_no_label() {
            let written = Written::new();
            let mut b = capturing(&written);
            b.reader
                .expect_read()
                .returning(|_, _| Ok((reversal_window(), vec![])));
            b.client
                .expect_complete()
                .returning(|_, _| Err(AppError::ValidationFailure("bad shape".into())));
            b.jev = Some(turn_jev(Arc::new(confident(Arc::default(), PINNED))));
            run_turn(&b.build(), &turn_params(REVERSAL_FINAL, false), None)
                .await
                .unwrap();
            assert!(written.checkpoint().fail_open);
            let rows = written.rows();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].label_json, None);
        }

        #[tokio::test]
        async fn a_cut_final_message_is_recorded_as_truncated() {
            let written = Written::new();
            let mut b = capturing(&written);
            b.reader
                .expect_read()
                .returning(|_, _| Ok((reversal_window(), vec![])));
            b.client
                .expect_complete()
                .returning(|_, _| Ok(contradiction_completion()));
            b.jev = Some(turn_jev(Arc::new(confident(Arc::default(), PINNED))));
            let long = format!("{REVERSAL_FINAL} {}", "filler words here ".repeat(150));
            run_turn(&b.build(), &turn_params(&long, false), None)
                .await
                .unwrap();
            assert!(written.rows()[0].truncated);
        }

        #[tokio::test]
        async fn memory_text_never_reaches_jev_and_capture_is_asked_only_with_memory() {
            // Memory on: a stored preference is recalled.
            let written = Written::new();
            let requests: Shared<JevRequest> = Arc::default();
            let mut b = capturing(&written);
            b.reader
                .expect_read()
                .returning(|_, _| Ok((empty_window(), vec![])));
            b.embedder = Some(Arc::new(matching_embedder()));
            b.storage.expect_load_memories().returning(|| {
                Ok(vec![preference_memory(
                    "mem-delve",
                    "final messages must never contain the word delve",
                    crate::memory::Trust::FirstHand,
                )])
            });
            b.client
                .expect_complete()
                .returning(|_, _| Ok(violation_completion()));
            b.jev = Some(turn_jev(Arc::new(confident(Arc::clone(&requests), PINNED))));
            run_turn(
                &b.build(),
                &turn_params("Let me delve into the details of this fix.", false),
                None,
            )
            .await
            .unwrap();
            let sent = requests.lock().unwrap()[0].clone();
            assert!(
                !sent.state.as_str().contains("never contain the word delve"),
                "memory text sent: {}",
                sent.state.as_str()
            );
            let ids: Vec<&str> = sent.questions.iter().map(|q| q.id.as_str()).collect();
            assert_eq!(ids, ["contradiction", "capture_worthy"]);

            // Memory off: contradiction only.
            let written = Written::new();
            let requests: Shared<JevRequest> = Arc::default();
            reversal_turn(
                &written,
                Some(turn_jev(Arc::new(confident(Arc::clone(&requests), PINNED)))),
            )
            .await;
            let sent = requests.lock().unwrap()[0].clone();
            let ids: Vec<&str> = sent.questions.iter().map(|q| q.id.as_str()).collect();
            assert_eq!(ids, ["contradiction"]);
        }

        #[tokio::test]
        async fn a_failed_jev_row_write_never_affects_the_turn() {
            let off = Written::new();
            let result_off = reversal_turn(&off, None).await;

            let mut b = DepsBuilder::new();
            b.reader
                .expect_read()
                .returning(|_, _| Ok((reversal_window(), vec![])));
            b.client
                .expect_complete()
                .returning(|_, _| Ok(contradiction_completion()));
            b.storage
                .expect_delivered_signal_keys_since()
                .returning(|_, _| Ok(vec![]));
            b.storage
                .expect_record_checkpoint()
                .times(1)
                .returning(|_| Ok(()));
            b.storage
                .expect_record_jev()
                .times(1)
                .returning(|_| Err(AppError::Storage("disk full".into())));
            b.jev = Some(turn_jev(Arc::new(confident(Arc::default(), PINNED))));
            let (result, _, _) = run_turn(&b.build(), &turn_params(REVERSAL_FINAL, false), None)
                .await
                .unwrap();
            let mut value = serde_json::to_value(&result).unwrap();
            value.as_object_mut().unwrap().remove("latency_ms");
            assert_eq!(value, result_off);
        }
    }
}
