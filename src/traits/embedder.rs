//! The embedding boundary (memory capability).
//!
//! Voyage embeddings are asymmetric: documents and queries embed with
//! different `input_type`s, and retrieval quality depends on using them
//! correctly — so the distinction is baked into the seam, making misuse
//! impossible (research.md 003 D2).

use crate::error::AppError;

/// One embedding: the vector plus billed usage.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedding {
    /// The embedding vector.
    pub vector: Vec<f32>,
    /// Input tokens billed for this call.
    pub input_tokens: u64,
}

/// An embedding backend.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Embed stored content (document side of the asymmetric space).
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] classified per the outcome taxonomy
    /// (`EmbeddingProvider`, `Timeout`, `RetriesExhausted`).
    async fn embed_document(&self, text: &str) -> Result<Embedding, AppError>;

    /// Embed a recall query (query side of the asymmetric space).
    ///
    /// # Errors
    ///
    /// Same classification as [`Embedder::embed_document`].
    async fn embed_query(&self, text: &str) -> Result<Embedding, AppError>;

    /// The embedding model id (recorded per memory row and on invocation
    /// records for cost attribution).
    fn model_id(&self) -> &str;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_embedder_distinguishes_document_and_query() {
        let mut mock = MockEmbedder::new();
        mock.expect_embed_document().returning(|_| {
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 7,
            })
        });
        mock.expect_embed_query().returning(|_| {
            Ok(Embedding {
                vector: vec![0.0, 1.0],
                input_tokens: 3,
            })
        });
        mock.expect_model_id().return_const("voyage-4".to_string());

        assert_eq!(
            mock.embed_document("d").await.unwrap().vector,
            vec![1.0, 0.0]
        );
        assert_eq!(mock.embed_query("q").await.unwrap().input_tokens, 3);
        assert_eq!(mock.model_id(), "voyage-4");
    }
}
