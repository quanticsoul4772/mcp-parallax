//! The rerank boundary (061): score documents against a query, so research
//! extracts claims only from its most relevant pages.

use crate::error::AppError;

/// One rerank: a relevance score per document, plus billed usage.
#[derive(Debug, Clone, PartialEq)]
pub struct RerankResult {
    /// One score per input document, in input order.
    pub scores: Vec<f32>,
    /// Tokens the provider billed for the call.
    pub total_tokens: u64,
}

/// A rerank backend.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait Reranker: Send + Sync {
    /// Score each document against `query`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] classified per the outcome taxonomy
    /// (`EmbeddingProvider`, `Timeout`, `RetriesExhausted`), or a client
    /// error for a malformed reply.
    async fn rerank(&self, query: &str, documents: &[String]) -> Result<RerankResult, AppError>;

    /// The rerank model id (usage and cost attribution).
    fn model_id(&self) -> &str;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::AppError;

    /// 061 T001: a rerank returns one score per document, in input order,
    /// and the tokens the provider billed.
    #[tokio::test]
    async fn a_rerank_scores_every_document_in_input_order() {
        let mut mock = MockReranker::new();
        mock.expect_rerank()
            .withf(|query, documents| query == "q" && documents == ["a", "b", "c"])
            .returning(|_, _| {
                Ok(RerankResult {
                    scores: vec![0.2, 0.9, 0.5],
                    total_tokens: 42,
                })
            });
        mock.expect_model_id().return_const("rerank-3".to_string());
        let documents = ["a".to_string(), "b".to_string(), "c".to_string()];
        let out = mock.rerank("q", &documents).await.unwrap();
        assert_eq!(out.scores, vec![0.2, 0.9, 0.5]);
        assert_eq!(out.total_tokens, 42);
        assert_eq!(mock.model_id(), "rerank-3");
    }

    /// 061 T001: a provider failure is an error, not a result.
    #[tokio::test]
    async fn a_failed_rerank_is_an_error() {
        let mut mock = MockReranker::new();
        mock.expect_rerank()
            .returning(|_, _| Err(AppError::EmbeddingProvider("down".into())));
        let err = mock.rerank("q", &["a".to_string()]).await.unwrap_err();
        assert!(matches!(err, AppError::EmbeddingProvider(_)), "{err:?}");
    }
}
