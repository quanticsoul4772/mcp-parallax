//! save / recall / forget — the memory tool logic.
//!
//! Trust is derived, never caller-set (research.md 003 D3): first-hand →
//! `first_hand`; external + verification → the existing verify ensemble
//! decides (`verified` or rejection-with-findings); external without →
//! `untrusted`, stored quarantined and down-ranked.

use crate::error::AppError;
use crate::memory::{ranking, Kind, Memory, Trust};
use crate::modes::verify::{self, VerdictKind, VerifyParams};
use crate::modes::CorrectiveMode;
use crate::traits::client::ModelClient;
use crate::traits::clock::TimeProvider;
use crate::traits::embedder::Embedder;
use crate::traits::storage::Storage;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    /// Generic input bound (`INPUT_MAX_CHARS`).
    pub input_max_chars: usize,
    /// Default recall top-k (`MEMORY_RECALL_LIMIT`).
    pub default_recall_limit: u8,
}

/// `save` input (contract: `contracts/save.tool.json`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SaveParams {
    /// The memory itself, self-contained.
    pub content: String,
    /// skill | lesson | fact.
    pub kind: Kind,
    /// Where this knowledge came from.
    pub origin: String,
    /// true if sourced from external content rather than first-hand experience.
    pub external: bool,
    /// Optional tags.
    pub tags: Option<Vec<String>>,
    /// Run independent verification before admitting an external memory as
    /// trusted.
    pub verify: Option<bool>,
}

/// `save` output.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SaveResult {
    /// The new memory's id.
    pub id: String,
    /// Derived trust standing.
    pub trust: Trust,
    /// Verification findings when verification ran; empty otherwise.
    pub findings: Vec<String>,
}

/// `recall` input (contract: `contracts/recall.tool.json`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecallParams {
    /// What you need, in natural language.
    pub query: String,
    /// Optional kind filter.
    pub kind: Option<Kind>,
    /// Optional result limit (1..=20; default from config).
    pub limit: Option<u32>,
}

/// One recalled memory.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RecalledMemory {
    /// Memory id (usable with `forget`).
    pub id: String,
    /// The memory content.
    pub content: String,
    /// skill | lesson | fact.
    pub kind: Kind,
    /// Stated provenance.
    pub origin: String,
    /// External-content provenance.
    pub external: bool,
    /// Trust standing.
    pub trust: Trust,
    /// RFC 3339 creation time.
    pub created_at: String,
    /// Raw relevance to the query.
    pub score: f32,
}

/// `recall` output — nested array is legal here (no model hop; D6).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RecallResult {
    /// Ranked memories, most relevant first.
    pub memories: Vec<RecalledMemory>,
}

/// `forget` input (contract: `contracts/forget.tool.json`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ForgetParams {
    /// The memory id to permanently delete.
    pub id: String,
}

/// `forget` output.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ForgetResult {
    /// Always true on success (an unknown id is an error, not `false`).
    pub forgotten: bool,
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
    };
    deps.storage.save_memory(&memory).await?;

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
        Some(n @ 1..=20) => n as usize,
        Some(n) => {
            return Err(AppError::InvalidInput(format!(
                "limit {n} is out of range 1..=20"
            )))
        }
    };

    let embedding = deps.embedder.embed_query(&params.query).await?;

    let mut memories = deps.storage.load_memories().await?;
    if let Some(kind) = params.kind {
        memories.retain(|m| m.kind == kind);
    }

    let ranked = ranking::rank(memories, &embedding.vector, deps.clock.now());
    let memories = ranked
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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

    async fn deps_with(embedder: MockEmbedder, model_client: MockModelClient) -> MemoryDeps {
        MemoryDeps {
            embedder: Arc::new(embedder),
            storage: Arc::new(SqliteStorage::connect(":memory:").await.unwrap()),
            clock: Arc::new(SystemClock),
            model_client: Arc::new(model_client),
            verify_mode: verify_mode(),
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
    async fn recall_respects_kind_filter_and_limit_and_empty_store() {
        let embedder = embedder_returning(vec![1.0, 0.0], vec![1.0, 0.0]);
        let deps = deps_with(embedder, no_model_calls()).await;

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

    // ---- contract sync ---------------------------------------------------------

    #[test]
    fn derived_schemas_match_the_contract_files() {
        let props = |schema: &Value, key: &str| -> Vec<String> {
            schema[key]["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };
        let derived_props = |schema: &Value| -> Vec<String> {
            schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };

        let save_contract: Value = serde_json::from_str(include_str!(
            "../../specs/003-memory-layer/contracts/save.tool.json"
        ))
        .unwrap();
        let save_in = serde_json::to_value(schemars::schema_for!(SaveParams)).unwrap();
        let save_out = serde_json::to_value(schemars::schema_for!(SaveResult)).unwrap();
        assert_eq!(
            derived_props(&save_in),
            props(&save_contract, "inputSchema")
        );
        assert_eq!(
            derived_props(&save_out),
            props(&save_contract, "outputSchema")
        );

        let recall_contract: Value = serde_json::from_str(include_str!(
            "../../specs/003-memory-layer/contracts/recall.tool.json"
        ))
        .unwrap();
        let recall_in = serde_json::to_value(schemars::schema_for!(RecallParams)).unwrap();
        let recall_out = serde_json::to_value(schemars::schema_for!(RecallResult)).unwrap();
        assert_eq!(
            derived_props(&recall_in),
            props(&recall_contract, "inputSchema")
        );
        assert_eq!(
            derived_props(&recall_out),
            props(&recall_contract, "outputSchema")
        );

        let forget_contract: Value = serde_json::from_str(include_str!(
            "../../specs/003-memory-layer/contracts/forget.tool.json"
        ))
        .unwrap();
        let forget_in = serde_json::to_value(schemars::schema_for!(ForgetParams)).unwrap();
        let forget_out = serde_json::to_value(schemars::schema_for!(ForgetResult)).unwrap();
        assert_eq!(
            derived_props(&forget_in),
            props(&forget_contract, "inputSchema")
        );
        assert_eq!(
            derived_props(&forget_out),
            props(&forget_contract, "outputSchema")
        );
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
