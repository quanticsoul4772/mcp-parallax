//! Thin Voyage AI embeddings client implementing the [`Embedder`] seam.
//!
//! Hand-rolled over `reqwest` for the same reason as the Anthropic client:
//! the surface is one endpoint (`POST /v1/embeddings`), and the asymmetric
//! `input_type` (document vs query) is the only knob retrieval quality hangs
//! on (research.md 003 D2).
//!
//! Retry policy mirrors [`crate::client::AnthropicClient`]: HTTP 429/5xx and
//! transport errors retry with exponential backoff up to `MAX_RETRIES`; a
//! per-request timeout is terminal (`Timeout`); other 4xx are terminal
//! (`EmbeddingProvider` — the request itself is wrong).

use crate::config::Config;
use crate::error::AppError;
use crate::traits::embedder::{Embedder, Embedding};
use crate::traits::rerank::{RerankResult, Reranker};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const VOYAGE_API_BASE: &str = "https://api.voyageai.com";

/// The reranker research uses (061 research.md D2): the provider's current
/// model; `rerank-2.5` is listed among its older models.
pub const RERANK_MODEL: &str = "rerank-3";

/// Thin `reqwest` client implementing [`Embedder`] against the Voyage API.
pub struct VoyageClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    timeout_ms: u64,
    max_retries: u32,
    backoff_base_ms: u64,
}

impl VoyageClient {
    /// Build a client from configuration, targeting the production endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Config`]-class error when `voyage_api_key` is
    /// absent — callers gate construction on key presence (FR-007), so
    /// reaching this without a key is a wiring bug surfaced loudly.
    pub fn new(config: &Config) -> Result<Self, AppError> {
        Self::with_base_url(config, VOYAGE_API_BASE)
    }

    /// Build a client against a custom endpoint (tests point this at a local
    /// wiremock server; nothing else should override it).
    ///
    /// # Errors
    ///
    /// Same as [`VoyageClient::new`].
    pub fn with_base_url(config: &Config, base_url: &str) -> Result<Self, AppError> {
        let api_key = config.voyage_api_key.clone().ok_or_else(|| {
            AppError::EmbeddingProvider(
                "VoyageClient constructed without VOYAGE_API_KEY — gate on key presence".into(),
            )
        })?;
        Ok(Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model: config.voyage_model.clone(),
            timeout_ms: config.request_timeout_ms,
            max_retries: config.max_retries,
            backoff_base_ms: 200,
        })
    }

    /// Shrink the retry backoff base (test-only speedup).
    #[doc(hidden)]
    #[must_use]
    pub const fn with_backoff_base_ms(mut self, ms: u64) -> Self {
        self.backoff_base_ms = ms;
        self
    }

    async fn send_once(&self, endpoint: &str, body: &Value) -> Result<reqwest::Response, AppError> {
        self.http
            .post(format!("{}{endpoint}", self.base_url))
            .bearer_auth(&self.api_key)
            .timeout(Duration::from_millis(self.timeout_ms))
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        what: "request",
                        ms: self.timeout_ms,
                    }
                } else {
                    // Transport-level failure (connect refused, reset) — retryable.
                    AppError::EmbeddingProvider(format!("transport: {e}"))
                }
            })
    }

    async fn embed(&self, text: &str, input_type: &str) -> Result<Embedding, AppError> {
        let body = json!({
            "input": [text],
            "model": self.model,
            "input_type": input_type,
        });
        let payload: EmbeddingsResponse = self.post("/v1/embeddings", &body).await?;
        interpret(payload)
    }

    /// POST `body` to `endpoint` under the shared retry policy and decode the
    /// 2xx reply.
    async fn post<T: DeserializeOwned>(&self, endpoint: &str, body: &Value) -> Result<T, AppError> {
        let attempts_max = self.max_retries.saturating_add(1);
        let mut last_error = String::new();

        for attempt in 1..=attempts_max {
            if attempt > 1 {
                let backoff = self
                    .backoff_base_ms
                    .saturating_mul(1 << (attempt - 2).min(8));
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }

            let response = match self.send_once(endpoint, body).await {
                Ok(r) => r,
                // A timeout consumed the full per-request budget — terminal.
                Err(timeout @ AppError::Timeout { .. }) => return Err(timeout),
                Err(e) => {
                    last_error = e.to_string();
                    continue;
                }
            };

            let status = response.status();
            if status.as_u16() == 429 || status.is_server_error() {
                last_error = format!("HTTP {status}");
                continue;
            }
            if !status.is_success() {
                let detail = response.text().await.unwrap_or_default();
                return Err(AppError::EmbeddingProvider(format!(
                    "HTTP {status}: {detail}"
                )));
            }

            let payload: T = response.json().await.map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        what: "request",
                        ms: self.timeout_ms,
                    }
                } else {
                    AppError::EmbeddingProvider(format!("response body unreadable: {e}"))
                }
            })?;
            return Ok(payload);
        }

        Err(AppError::RetriesExhausted {
            attempts: attempts_max,
            last: last_error,
        })
    }
}

#[async_trait::async_trait]
impl Embedder for VoyageClient {
    async fn embed_document(&self, text: &str) -> Result<Embedding, AppError> {
        self.embed(text, "document").await
    }

    async fn embed_query(&self, text: &str) -> Result<Embedding, AppError> {
        self.embed(text, "query").await
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

#[async_trait::async_trait]
impl Reranker for VoyageClient {
    async fn rerank(&self, query: &str, documents: &[String]) -> Result<RerankResult, AppError> {
        let body = json!({
            "query": query,
            "documents": documents,
            "model": RERANK_MODEL,
            "truncation": true,
        });
        let payload: RerankResponse = self.post("/v1/rerank", &body).await?;
        interpret_rerank(payload, documents.len())
    }

    fn model_id(&self) -> &str {
        RERANK_MODEL
    }
}

/// Map a 2xx rerank reply to a score per document in input order. Every
/// input index exactly once, or the reply is out of contract (061 contract).
fn interpret_rerank(payload: RerankResponse, count: usize) -> Result<RerankResult, AppError> {
    let mut scores: Vec<Option<f32>> = vec![None; count];
    for datum in payload.data {
        let slot = scores.get_mut(datum.index).ok_or_else(|| {
            AppError::EmbeddingProvider(format!(
                "out-of-contract rerank reply: index {} out of range",
                datum.index
            ))
        })?;
        if slot.replace(datum.relevance_score).is_some() {
            return Err(AppError::EmbeddingProvider(format!(
                "out-of-contract rerank reply: index {} scored twice",
                datum.index
            )));
        }
    }
    let scores = scores
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            s.ok_or_else(|| {
                AppError::EmbeddingProvider(format!(
                    "out-of-contract rerank reply: index {i} not scored"
                ))
            })
        })
        .collect::<Result<Vec<f32>, AppError>>()?;
    Ok(RerankResult {
        scores,
        total_tokens: payload.usage.total_tokens,
    })
}

#[derive(Debug, Deserialize)]
struct RerankResponse {
    #[serde(default)]
    data: Vec<RerankDatum>,
    #[serde(default)]
    usage: VoyageUsage,
}

#[derive(Debug, Deserialize)]
struct RerankDatum {
    index: usize,
    relevance_score: f32,
}

/// Map a 2xx embeddings response to an [`Embedding`] or its outcome class.
fn interpret(payload: EmbeddingsResponse) -> Result<Embedding, AppError> {
    let vector = payload
        .data
        .into_iter()
        .next()
        .map(|d| d.embedding)
        .ok_or_else(|| {
            AppError::EmbeddingProvider(
                "out-of-contract provider response: empty data array".into(),
            )
        })?;
    if vector.is_empty() {
        return Err(AppError::EmbeddingProvider(
            "out-of-contract provider response: empty embedding vector".into(),
        ));
    }
    Ok(Embedding {
        vector,
        input_tokens: payload.usage.total_tokens,
    })
}

#[derive(Debug, Deserialize)]
struct EmbeddingsResponse {
    #[serde(default)]
    data: Vec<EmbeddingDatum>,
    #[serde(default)]
    usage: VoyageUsage,
}

#[derive(Debug, Deserialize)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct VoyageUsage {
    #[serde(default)]
    total_tokens: u64,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn test_config() -> Config {
        Config {
            voyage_api_key: Some("voyage-test-key".into()),
            ..crate::config::test_config()
        }
    }

    fn client_for(mock: &MockServer) -> VoyageClient {
        VoyageClient::with_base_url(&test_config(), &mock.uri())
            .unwrap()
            .with_backoff_base_ms(1)
    }

    fn ok_body(vector: &[f32], total_tokens: u64) -> serde_json::Value {
        json!({
            "object": "list",
            "data": [{ "object": "embedding", "embedding": vector, "index": 0 }],
            "model": "voyage-4",
            "usage": { "total_tokens": total_tokens }
        })
    }

    fn docs() -> Vec<String> {
        vec!["alpha".into(), "beta".into(), "gamma".into()]
    }

    /// 061 T006 (contract): the rerank request is exactly the contract's,
    /// with the bearer key; reply indices map scores back to input order.
    #[tokio::test]
    async fn rerank_sends_the_contract_request_and_maps_scores_to_input_order() {
        use crate::traits::rerank::Reranker;
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/rerank"))
            .and(header("authorization", "Bearer voyage-test-key"))
            .and(wiremock::matchers::body_json(json!({
                "query": "which is best?",
                "documents": ["alpha", "beta", "gamma"],
                "model": "rerank-3",
                "truncation": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [
                    {"index": 2, "relevance_score": 0.9},
                    {"index": 0, "relevance_score": 0.4},
                    {"index": 1, "relevance_score": 0.1}
                ],
                "model": "rerank-3",
                "usage": {"total_tokens": 321}
            })))
            .expect(1)
            .mount(&mock)
            .await;
        let out = client_for(&mock)
            .rerank("which is best?", &docs())
            .await
            .unwrap();
        assert_eq!(out.scores, vec![0.4, 0.1, 0.9]);
        assert_eq!(out.total_tokens, 321);
        assert_eq!(Reranker::model_id(&client_for(&mock)), "rerank-3");
    }

    /// 061 T006 (contract): a reply that does not score every input index
    /// exactly once, or carries a non-numeric score, is an error.
    #[tokio::test]
    async fn a_malformed_rerank_reply_is_an_error() {
        use crate::traits::rerank::Reranker;
        let bad_replies = [
            json!({"data": [{"index": 0, "relevance_score": 0.4}, {"index": 1, "relevance_score": 0.1}], "usage": {"total_tokens": 1}}),
            json!({"data": [{"index": 0, "relevance_score": 0.4}, {"index": 0, "relevance_score": 0.3}, {"index": 1, "relevance_score": 0.1}], "usage": {"total_tokens": 1}}),
            json!({"data": [{"index": 0, "relevance_score": 0.4}, {"index": 1, "relevance_score": 0.1}, {"index": 7, "relevance_score": 0.2}], "usage": {"total_tokens": 1}}),
            json!({"data": [{"index": 0, "relevance_score": "high"}, {"index": 1, "relevance_score": 0.1}, {"index": 2, "relevance_score": 0.2}], "usage": {"total_tokens": 1}}),
        ];
        for reply in bad_replies {
            let mock = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/rerank"))
                .respond_with(ResponseTemplate::new(200).set_body_json(reply.clone()))
                .mount(&mock)
                .await;
            let err = client_for(&mock).rerank("q", &docs()).await.unwrap_err();
            assert!(
                matches!(err, AppError::EmbeddingProvider(_)),
                "{reply}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn missing_key_is_a_loud_construction_error() {
        let mut config = test_config();
        config.voyage_api_key = None;
        // No Debug derive on the client (it holds the API key), so no unwrap_err.
        let Err(err) = VoyageClient::with_base_url(&config, "http://localhost") else {
            panic!("expected construction to fail without a key");
        };
        assert!(matches!(err, AppError::EmbeddingProvider(_)), "got: {err}");
    }

    #[tokio::test]
    async fn document_and_query_carry_their_input_types() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer voyage-test-key"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                assert_eq!(body["model"], "voyage-4");
                assert_eq!(body["input"], json!(["some text"]));
                let input_type = body["input_type"].as_str().unwrap();
                assert!(input_type == "document" || input_type == "query");
                // Encode the input_type in the vector so the caller-side
                // assertion can tell which path was taken.
                let v = if input_type == "document" { 1.0 } else { 2.0 };
                ResponseTemplate::new(200).set_body_json(ok_body(&[v], 7))
            })
            .mount(&mock)
            .await;

        let client = client_for(&mock);
        let doc = client.embed_document("some text").await.unwrap();
        assert_eq!(doc.vector, vec![1.0]);
        assert_eq!(doc.input_tokens, 7);
        let query = client.embed_query("some text").await.unwrap();
        assert_eq!(query.vector, vec![2.0]);
        assert_eq!(Embedder::model_id(&client), "voyage-4");
    }

    #[tokio::test]
    async fn persistent_5xx_exhausts_retries_with_attempt_count() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3) // max_retries = 2 → 3 attempts total
            .mount(&mock)
            .await;

        let err = client_for(&mock).embed_document("t").await.unwrap_err();
        match err {
            AppError::RetriesExhausted { attempts, ref last } => {
                assert_eq!(attempts, 3);
                assert!(last.contains("503"), "last error: {last}");
            }
            other => panic!("expected RetriesExhausted, got {other}"),
        }
    }

    #[tokio::test]
    async fn recovers_when_a_retry_succeeds() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body(&[0.5, -0.5], 3)))
            .mount(&mock)
            .await;

        let out = client_for(&mock).embed_query("t").await.unwrap();
        assert_eq!(out.vector, vec![0.5, -0.5]);
    }

    #[tokio::test]
    async fn slow_provider_is_a_timeout_not_a_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(ok_body(&[1.0], 1))
                    .set_delay(Duration::from_secs(10)),
            )
            .expect(1) // terminal: no second attempt
            .mount(&mock)
            .await;

        let err = client_for(&mock).embed_document("t").await.unwrap_err();
        assert!(
            matches!(err, AppError::Timeout { ms: 2_000, .. }),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn non_retryable_4xx_is_terminal_and_classified() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad input"))
            .expect(1)
            .mount(&mock)
            .await;

        let err = client_for(&mock).embed_document("t").await.unwrap_err();
        match err {
            AppError::EmbeddingProvider(msg) => {
                assert!(msg.contains("400") && msg.contains("bad input"), "{msg}");
            }
            other => panic!("expected EmbeddingProvider, got {other}"),
        }
    }

    #[tokio::test]
    async fn empty_data_array_is_out_of_contract() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [],
                "usage": { "total_tokens": 0 }
            })))
            .mount(&mock)
            .await;

        let err = client_for(&mock).embed_document("t").await.unwrap_err();
        assert!(matches!(err, AppError::EmbeddingProvider(_)), "got: {err}");
        assert!(err.to_string().contains("empty data"));
    }

    #[tokio::test]
    async fn empty_vector_is_out_of_contract() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body(&[], 1)))
            .mount(&mock)
            .await;

        let err = client_for(&mock).embed_query("t").await.unwrap_err();
        assert!(matches!(err, AppError::EmbeddingProvider(_)), "got: {err}");
        assert!(err.to_string().contains("empty embedding vector"));
    }
}
