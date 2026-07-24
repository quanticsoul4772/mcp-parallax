//! save / recall / forget — the memory tool logic.
//!
//! Trust is derived, never caller-set (research.md 003 D3): first-hand →
//! `first_hand`; external + verification → the existing verify ensemble
//! decides (`verified` or rejection-with-findings); external without →
//! `untrusted`, stored quarantined and down-ranked.

use crate::config::MEMORY_RECALL_LIMIT_MAX;
use crate::error::AppError;
use crate::memory::{ranking, Memory, Trust};
use crate::modes::verify::{self, VerdictKind, VerifyParams};
use crate::modes::CorrectiveMode;
use crate::traits::client::ModelClient;
use crate::traits::clock::TimeProvider;
use crate::traits::embedder::Embedder;
use crate::traits::storage::Storage;
use std::sync::Arc;

pub use super::contract::{
    ForgetParams, ForgetResult, RecallParams, RecallResult, RecalledMemory, SaveParams, SaveResult,
};

/// Maximum number of tags per memory.
pub const MAX_TAGS: usize = 20;
/// Maximum characters per tag.
pub const MAX_TAG_CHARS: usize = 100;

/// Everything the memory tools need, composed from the server's seams.
pub struct MemoryDeps {
    /// The embedding backend (present only when the capability is enabled).
    pub embedder: Arc<dyn Embedder>,
    /// The shared store.
    pub storage: Arc<dyn Storage>,
    /// The shared clock.
    pub clock: Arc<dyn TimeProvider>,
    /// For verify-at-save.
    pub model_client: Arc<dyn ModelClient>,
    /// The registered verify mode (verify-at-save reuses it unchanged).
    pub verify_mode: CorrectiveMode,
    /// The registered consolidation judgment mode (017 research D3).
    pub consolidation_mode: CorrectiveMode,
    /// Generic input bound (`INPUT_MAX_CHARS`).
    pub input_max_chars: usize,
    /// Default recall top-k (`MEMORY_RECALL_LIMIT`).
    pub default_recall_limit: u8,
}

/// Should this save run verification? Known up front from the params — the
/// server uses it to attribute the invocation record's model correctly.
#[must_use]
pub fn save_runs_verification(params: &SaveParams) -> bool {
    params.external && params.verify == Some(true)
}

/// Save one memory. Returns the result plus (input, output) token usage —
/// embedding tokens, plus the verify ensemble's when verification ran.
///
/// # Errors
///
/// `InvalidInput` before any provider call; `ValidationFailure` when
/// verification refutes the content (the save is rejected, findings included).
pub async fn save(
    deps: &MemoryDeps,
    params: &SaveParams,
) -> Result<(SaveResult, u64, u64), AppError> {
    check_text("content", &params.content, deps.input_max_chars)?;
    check_text("origin", &params.origin, deps.input_max_chars)?;
    check_tags(params.tags.as_deref())?;

    let (mut input_tokens, mut output_tokens) = (0_u64, 0_u64);

    // Trust derivation (D3) — verification first, so a refuted save costs no
    // embedding call.
    let (trust, findings) = if params.external {
        if params.verify == Some(true) {
            let verify_params = VerifyParams {
                claim: params.content.clone(),
                context: None,
            };
            let run = verify::run(
                deps.model_client.as_ref(),
                &deps.verify_mode,
                &verify_params,
                deps.input_max_chars,
            )
            .await?;
            input_tokens += run.input_tokens;
            output_tokens += run.output_tokens;
            if run.verdict.verdict == VerdictKind::Refuted {
                return Err(AppError::ValidationFailure(format!(
                    "save rejected: verification refuted the content: {}",
                    run.verdict.findings.join(" | ")
                )));
            }
            (Trust::Verified, run.verdict.findings)
        } else {
            (Trust::Untrusted, Vec::new())
        }
    } else {
        (Trust::FirstHand, Vec::new())
    };

    let embedding = deps.embedder.embed_document(&params.content).await?;
    input_tokens += embedding.input_tokens;

    let memory = Memory {
        id: uuid::Uuid::new_v4().to_string(),
        content: params.content.clone(),
        kind: params.kind,
        origin: params.origin.clone(),
        external: params.external,
        trust,
        tags: params.tags.clone().unwrap_or_default(),
        embedding: embedding.vector,
        embedding_model: deps.embedder.model_id().to_string(),
        created_at: deps.clock.now(),
        status: crate::memory::Status::Active,
        replaced_by: None,
        last_reinforced_at: deps.clock.now(),
    };
    deps.storage.save_memory(&memory).await?;

    // 017: admission-time consolidation — fail-open (any failure ⇒ keep
    // both, which is the decline-biased outcome anyway).
    let (cons_in, cons_out) = consolidate_admission(deps, &memory, None).await;
    input_tokens += cons_in;
    output_tokens += cons_out;

    Ok((
        SaveResult {
            id: memory.id,
            trust,
            findings,
        },
        input_tokens,
        output_tokens,
    ))
}

/// Recall the most relevant memories for a query.
///
/// # Errors
///
/// `InvalidInput` for an empty/oversized query or an out-of-range limit.
pub async fn recall(
    deps: &MemoryDeps,
    params: &RecallParams,
) -> Result<(RecallResult, u64, u64), AppError> {
    check_text("query", &params.query, deps.input_max_chars)?;
    let limit = match params.limit {
        None => usize::from(deps.default_recall_limit),
        Some(n) if (1..=u32::from(MEMORY_RECALL_LIMIT_MAX)).contains(&n) => n as usize,
        Some(n) => {
            return Err(AppError::InvalidInput(format!(
                "limit {n} is out of range 1..={MEMORY_RECALL_LIMIT_MAX}"
            )))
        }
    };

    let embedding = deps.embedder.embed_query(&params.query).await?;

    let mut memories = deps.storage.load_memories().await?;
    // The per-row embedding_model exists to make a model switch detectable —
    // mismatched memories rank against an incompatible space (often scoring
    // 0.0), so say it loudly instead of degrading silently.
    let current_model = deps.embedder.model_id();
    let mismatched = memories
        .iter()
        .filter(|m| m.embedding_model != current_model)
        .count();
    if mismatched > 0 {
        tracing::warn!(
            current_model,
            mismatched,
            "memories were embedded with a different model; their recall \
             ranking is unreliable — re-save them or restore the prior \
             VOYAGE_MODEL"
        );
    }
    // 017 FR-011: only active records participate in retrieval.
    memories.retain(|m| m.status.is_active());
    if let Some(kind) = params.kind {
        memories.retain(|m| m.kind == kind);
    }

    let ranked = ranking::rank(memories, &embedding.vector, deps.clock.now());
    let memories: Vec<RecalledMemory> = ranked
        .into_iter()
        .take(limit)
        .map(|r| RecalledMemory {
            id: r.memory.id,
            content: r.memory.content,
            kind: r.memory.kind,
            origin: r.memory.origin,
            external: r.memory.external,
            trust: r.memory.trust,
            created_at: r.memory.created_at.to_rfc3339(),
            score: r.relevance,
        })
        .collect();

    // 017 research D5: being returned refreshes the decay clock —
    // fire-and-forget, failures never affect the response.
    let returned: Vec<String> = memories.iter().map(|m| m.id.clone()).collect();
    if !returned.is_empty() {
        if let Err(error) = deps
            .storage
            .touch_reinforcement(&returned, deps.clock.now())
            .await
        {
            tracing::warn!(%error, "recall reinforcement update failed (ignored)");
        }
    }

    Ok((RecallResult { memories }, embedding.input_tokens, 0))
}

/// Permanently delete a memory by id.
///
/// # Errors
///
/// `InvalidInput` (distinct not-found message) when no memory has the id.
pub async fn forget(
    deps: &MemoryDeps,
    params: &ForgetParams,
) -> Result<(ForgetResult, u64, u64), AppError> {
    if params.id.trim().is_empty() {
        return Err(AppError::InvalidInput("id is empty".to_string()));
    }
    let found = deps.storage.delete_memory(&params.id).await?;
    if !found {
        return Err(AppError::InvalidInput(format!(
            "no memory with id {:?}",
            params.id
        )));
    }
    Ok((ForgetResult { forgotten: true }, 0, 0))
}

/// Tag validation: bounded count, each tag non-empty and bounded — the
/// `INPUT_MAX_CHARS` bound on content/origin must not be bypassable via tags.
fn check_tags(tags: Option<&[String]>) -> Result<(), AppError> {
    let Some(tags) = tags else { return Ok(()) };
    if tags.len() > MAX_TAGS {
        return Err(AppError::InvalidInput(format!(
            "{} tags exceed the maximum of {MAX_TAGS}",
            tags.len()
        )));
    }
    for tag in tags {
        if tag.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "a tag is empty or whitespace-only".to_string(),
            ));
        }
        let len = tag.chars().count();
        if len > MAX_TAG_CHARS {
            return Err(AppError::InvalidInput(format!(
                "a tag is {len} characters; the maximum is {MAX_TAG_CHARS}"
            )));
        }
    }
    Ok(())
}

/// Shared input validation: non-empty after trim, bounded (FR-010).
fn check_text(field: &str, text: &str, max_chars: usize) -> Result<(), AppError> {
    if text.trim().is_empty() {
        return Err(AppError::InvalidInput(format!(
            "{field} is empty or whitespace-only"
        )));
    }
    let len = text.chars().count();
    if len > max_chars {
        return Err(AppError::InvalidInput(format!(
            "{field} is {len} characters; the configured maximum is {max_chars} \
             (INPUT_MAX_CHARS); it was not trimmed"
        )));
    }
    Ok(())
}

/// Admission-time consolidation (017 research D3): screen → at most one
/// judgment → pure apply → audit.
///
/// Entirely fail-open — a wrong keep-both is the decline-biased outcome; a
/// wrong action would destroy knowledge. Returns the judgment's token usage
/// for attribution.
pub async fn consolidate_admission(
    deps: &MemoryDeps,
    new_memory: &Memory,
    session_id: Option<&str>,
) -> (u64, u64) {
    use crate::memory::consolidate::{self, ConsolidationRecord};

    let evaluation = async {
        let memories = deps.storage.load_memories().await?;
        let Some((cosine, old)) = consolidate::screen(new_memory, &memories) else {
            return Ok::<_, AppError>((0, 0));
        };
        let (out, input_tokens, output_tokens) = consolidate::judge(
            deps.model_client.as_ref(),
            &deps.consolidation_mode,
            new_memory,
            old,
        )
        .await?;
        let Some(applied) = consolidate::apply(out.relation, cosine, new_memory, old) else {
            return Ok((input_tokens, output_tokens));
        };
        deps.storage
            .update_memory_status(&old.id, applied.old_status, Some(new_memory.id.clone()))
            .await?;
        let record = ConsolidationRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.map(str::to_string),
            action: applied.action,
            source_id: old.id.clone(),
            target_id: Some(new_memory.id.clone()),
            basis: out.basis,
            created_at: deps.clock.now(),
        };
        crate::observability::emit_consolidation(&record);
        deps.storage.record_consolidation(&record).await?;
        Ok((input_tokens, output_tokens))
    };
    match evaluation.await {
        Ok(tokens) => tokens,
        Err(error) => {
            tracing::warn!(%error, "admission consolidation failed open (kept both)");
            (0, 0)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::memory::Kind;
    use crate::modes::ModeRegistry;
    use crate::storage::SqliteStorage;
    use crate::traits::client::{Completion, MockModelClient};
    use crate::traits::clock::SystemClock;
    use crate::traits::embedder::{Embedding, MockEmbedder};
    use serde_json::{json, Value};

    fn verify_mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        verify::register(&mut registry, 1).unwrap();
        registry.get(verify::VERIFY_ID).unwrap().clone()
    }

    fn consolidation_mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        crate::memory::consolidate::register(&mut registry).unwrap();
        registry
            .get(crate::memory::consolidate::CONSOLIDATION_MODE_ID)
            .unwrap()
            .clone()
    }

    async fn deps_with(embedder: MockEmbedder, model_client: MockModelClient) -> MemoryDeps {
        MemoryDeps {
            embedder: Arc::new(embedder),
            storage: Arc::new(SqliteStorage::connect(":memory:").await.unwrap()),
            clock: Arc::new(SystemClock),
            model_client: Arc::new(model_client),
            verify_mode: verify_mode(),
            consolidation_mode: consolidation_mode(),
            input_max_chars: 50_000,
            default_recall_limit: 5,
        }
    }

    fn embedder_returning(doc: Vec<f32>, query: Vec<f32>) -> MockEmbedder {
        let mut mock = MockEmbedder::new();
        mock.expect_embed_document().returning(move |_| {
            Ok(Embedding {
                vector: doc.clone(),
                input_tokens: 9,
            })
        });
        mock.expect_embed_query().returning(move |_| {
            Ok(Embedding {
                vector: query.clone(),
                input_tokens: 4,
            })
        });
        mock.expect_model_id().return_const("voyage-4".to_string());
        mock
    }

    fn no_model_calls() -> MockModelClient {
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(0);
        mock
    }

    fn save_params(content: &str, external: bool, verify: Option<bool>) -> SaveParams {
        SaveParams {
            content: content.to_string(),
            kind: Kind::Skill,
            origin: "test origin".to_string(),
            external,
            tags: None,
            verify,
        }
    }

    // ---- T009: save/recall round trip --------------------------------------

    #[tokio::test]
    async fn save_embeds_as_document_and_recall_as_query() {
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_document().times(1).returning(|_| {
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 9,
            })
        });
        embedder.expect_embed_query().times(1).returning(|_| {
            Ok(Embedding {
                vector: vec![0.9, 0.1],
                input_tokens: 4,
            })
        });
        embedder
            .expect_model_id()
            .return_const("voyage-4".to_string());
        let deps = deps_with(embedder, no_model_calls()).await;

        let (saved, in_tokens, out_tokens) =
            save(&deps, &save_params("CI debugging skill", false, None))
                .await
                .unwrap();
        assert_eq!(saved.trust, Trust::FirstHand);
        assert!(saved.findings.is_empty());
        assert_eq!((in_tokens, out_tokens), (9, 0));

        let (recalled, q_tokens, _) = recall(
            &deps,
            &RecallParams {
                query: "how to debug CI".into(),
                kind: None,
                limit: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(q_tokens, 4);
        assert_eq!(recalled.memories.len(), 1);
        assert_eq!(recalled.memories[0].id, saved.id);
        assert_eq!(recalled.memories[0].trust, Trust::FirstHand);
        assert!(recalled.memories[0].score > 0.9);
    }

    #[tokio::test]
    async fn recall_excludes_superseded_memories() {
        // 017 FR-011: only active records participate in retrieval.
        let embedder = embedder_returning(vec![1.0, 0.0], vec![1.0, 0.0]);
        let deps = deps_with(embedder, distinct_judgments()).await;
        let (old_save, _, _) = save(&deps, &save_params("stale fact", false, None))
            .await
            .unwrap();
        let (new_save, _, _) = save(&deps, &save_params("current fact", false, None))
            .await
            .unwrap();
        deps.storage
            .update_memory_status(
                &old_save.id,
                crate::memory::Status::Superseded,
                Some(new_save.id.clone()),
            )
            .await
            .unwrap();
        let (recalled, _, _) = recall(
            &deps,
            &RecallParams {
                query: "fact".into(),
                kind: None,
                limit: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(recalled.memories.len(), 1);
        assert_eq!(recalled.memories[0].id, new_save.id);
    }

    fn distinct_judgments() -> MockModelClient {
        // Identical mock embeddings make every same-kind save screen against
        // its predecessors (017); answering `distinct` keeps them all — this
        // test is about recall mechanics, not consolidation.
        let mut client = MockModelClient::new();
        client.expect_complete().returning(|_, _| {
            Ok(crate::traits::client::Completion {
                value: serde_json::json!({ "relation": "distinct", "basis": "test" }),
                input_tokens: 0,
                output_tokens: 0,
            })
        });
        client
    }

    #[tokio::test]
    async fn recall_respects_kind_filter_and_limit_and_empty_store() {
        let embedder = embedder_returning(vec![1.0, 0.0], vec![1.0, 0.0]);
        let deps = deps_with(embedder, distinct_judgments()).await;

        // Empty store → empty result, success.
        let (empty, _, _) = recall(
            &deps,
            &RecallParams {
                query: "anything".into(),
                kind: None,
                limit: None,
            },
        )
        .await
        .unwrap();
        assert!(empty.memories.is_empty());

        // Three skills + one lesson.
        for i in 0..3 {
            save(&deps, &save_params(&format!("skill {i}"), false, None))
                .await
                .unwrap();
        }
        let mut lesson = save_params("a lesson", false, None);
        lesson.kind = Kind::Lesson;
        save(&deps, &lesson).await.unwrap();

        let (lessons_only, _, _) = recall(
            &deps,
            &RecallParams {
                query: "q".into(),
                kind: Some(Kind::Lesson),
                limit: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(lessons_only.memories.len(), 1);
        assert_eq!(lessons_only.memories[0].kind, Kind::Lesson);

        let (limited, _, _) = recall(
            &deps,
            &RecallParams {
                query: "q".into(),
                kind: None,
                limit: Some(2),
            },
        )
        .await
        .unwrap();
        assert_eq!(limited.memories.len(), 2);

        // Out-of-range limit is invalid input.
        let err = recall(
            &deps,
            &RecallParams {
                query: "q".into(),
                kind: None,
                limit: Some(21),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn empty_and_oversized_inputs_rejected_before_any_provider_call() {
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_document().times(0);
        embedder.expect_embed_query().times(0);
        let deps = deps_with(embedder, no_model_calls()).await;

        let err = save(&deps, &save_params("   ", false, None))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));

        let mut oversized = save_params("ok", false, None);
        oversized.content = "x".repeat(50_001);
        let err = save(&deps, &oversized).await.unwrap_err();
        assert!(err.to_string().contains("INPUT_MAX_CHARS"));

        let err = recall(
            &deps,
            &RecallParams {
                query: "  ".into(),
                kind: None,
                limit: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    // ---- T012: trust derivation ---------------------------------------------

    #[tokio::test]
    async fn first_hand_never_calls_verify_even_when_requested() {
        let embedder = embedder_returning(vec![1.0], vec![1.0]);
        // times(0) on the model client is the assertion.
        let deps = deps_with(embedder, no_model_calls()).await;

        let (saved, _, _) = save(&deps, &save_params("mine", false, Some(true)))
            .await
            .unwrap();
        assert_eq!(saved.trust, Trust::FirstHand);
    }

    #[tokio::test]
    async fn external_without_verify_is_untrusted() {
        let embedder = embedder_returning(vec![1.0], vec![1.0]);
        let deps = deps_with(embedder, no_model_calls()).await;

        let (saved, _, _) = save(&deps, &save_params("from the web", true, None))
            .await
            .unwrap();
        assert_eq!(saved.trust, Trust::Untrusted);
    }

    fn verifier_returning(value: Value) -> MockModelClient {
        let mut mock = MockModelClient::new();
        mock.expect_complete().times(1).returning(move |_, _| {
            Ok(Completion {
                value: value.clone(),
                input_tokens: 100,
                output_tokens: 20,
            })
        });
        mock
    }

    #[tokio::test]
    async fn external_with_supporting_verification_is_verified() {
        let embedder = embedder_returning(vec![1.0], vec![1.0]);
        let client = verifier_returning(json!({ "verdict": "supported", "findings": [] }));
        let deps = deps_with(embedder, client).await;

        let (saved, in_tokens, out_tokens) = save(
            &deps,
            &save_params("sqlx hooks run per connection", true, Some(true)),
        )
        .await
        .unwrap();
        assert_eq!(saved.trust, Trust::Verified);
        // Verify tokens + embedding tokens.
        assert_eq!(in_tokens, 100 + 9);
        assert_eq!(out_tokens, 20);
    }

    #[tokio::test]
    async fn refuting_verification_rejects_the_save_with_findings() {
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_document().times(0); // rejected before embedding
        let client = verifier_returning(
            json!({ "verdict": "refuted", "findings": ["that is false because X"] }),
        );
        let deps = deps_with(embedder, client).await;

        let err = save(&deps, &save_params("a poisoned claim", true, Some(true)))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationFailure(_)), "{err}");
        assert!(err.to_string().contains("that is false because X"));

        // Nothing was stored — re-check via the same storage with an embedder
        // that does allow a query call.
        let deps2 = MemoryDeps {
            embedder: Arc::new(embedder_returning(vec![1.0], vec![1.0])),
            ..deps
        };
        let (result, _, _) = recall(
            &deps2,
            &RecallParams {
                query: "poisoned".into(),
                kind: None,
                limit: None,
            },
        )
        .await
        .unwrap();
        assert!(result.memories.is_empty());
    }

    // ---- forget ---------------------------------------------------------------

    #[tokio::test]
    async fn forget_removes_and_unknown_id_is_distinct_not_found() {
        let embedder = embedder_returning(vec![1.0], vec![1.0]);
        let deps = deps_with(embedder, no_model_calls()).await;

        let (saved, _, _) = save(&deps, &save_params("to be forgotten", false, None))
            .await
            .unwrap();
        let (forgotten, _, _) = forget(
            &deps,
            &ForgetParams {
                id: saved.id.clone(),
            },
        )
        .await
        .unwrap();
        assert!(forgotten.forgotten);

        let (after, _, _) = recall(
            &deps,
            &RecallParams {
                query: "forgotten".into(),
                kind: None,
                limit: None,
            },
        )
        .await
        .unwrap();
        assert!(after.memories.is_empty());

        let err = forget(&deps, &ForgetParams { id: saved.id })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no memory with id"), "{err}");
    }

    // Contract-sync tests live with the wire types in `memory::contract`.

    #[tokio::test]
    async fn oversized_or_blank_tags_are_rejected_before_any_provider_call() {
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_document().times(0);
        let deps = deps_with(embedder, no_model_calls()).await;

        let mut too_many = save_params("ok", false, None);
        too_many.tags = Some(vec!["t".to_string(); MAX_TAGS + 1]);
        let err = save(&deps, &too_many).await.unwrap_err();
        assert!(err.to_string().contains("maximum of"), "{err}");

        let mut blank = save_params("ok", false, None);
        blank.tags = Some(vec!["  ".to_string()]);
        let err = save(&deps, &blank).await.unwrap_err();
        assert!(err.to_string().contains("empty or whitespace"), "{err}");

        let mut oversized = save_params("ok", false, None);
        oversized.tags = Some(vec!["x".repeat(MAX_TAG_CHARS + 1)]);
        let err = save(&deps, &oversized).await.unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
    }

    #[test]
    fn save_runs_verification_is_known_up_front() {
        assert!(save_runs_verification(&save_params("c", true, Some(true))));
        assert!(!save_runs_verification(&save_params("c", true, None)));
        assert!(!save_runs_verification(&save_params(
            "c",
            false,
            Some(true)
        )));
    }
}
