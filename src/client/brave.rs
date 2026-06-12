//! Thin Brave Search client implementing the [`SearchProvider`] seam.
//!
//! One endpoint (`GET /res/v1/web/search`, `X-Subscription-Token` auth),
//! response shape `web.results[].{url, title, description}` (research.md 004
//! D1, pinned by `examples/spike_brave.rs`).
//!
//! Retry policy mirrors the Anthropic/Voyage clients: HTTP 429/5xx and
//! transport errors retry with exponential backoff up to `MAX_RETRIES`; a
//! per-request timeout is terminal (`Timeout`); other 4xx are terminal
//! (`SearchProvider` — the request or credential is wrong).

use crate::config::Config;
use crate::error::AppError;
use crate::traits::search::{SearchHit, SearchProvider};
use serde::Deserialize;
use std::time::Duration;

const BRAVE_API_BASE: &str = "https://api.search.brave.com";

/// Thin `reqwest` client implementing [`SearchProvider`] against Brave.
pub struct BraveClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    timeout_ms: u64,
    max_retries: u32,
    backoff_base_ms: u64,
}

impl BraveClient {
    /// Build a client from configuration, targeting the production endpoint.
    ///
    /// # Errors
    ///
    /// Errors when `brave_api_key` is absent — callers gate construction on
    /// key presence (FR-008), so reaching this without a key is a wiring bug
    /// surfaced loudly.
    pub fn new(config: &Config) -> Result<Self, AppError> {
        Self::with_base_url(config, BRAVE_API_BASE)
    }

    /// Build a client against a custom endpoint (tests point this at a local
    /// wiremock server; nothing else should override it).
    ///
    /// # Errors
    ///
    /// Same as [`BraveClient::new`].
    pub fn with_base_url(config: &Config, base_url: &str) -> Result<Self, AppError> {
        let api_key = config.brave_api_key.clone().ok_or_else(|| {
            AppError::SearchProvider(
                "BraveClient constructed without BRAVE_API_KEY — gate on key presence".into(),
            )
        })?;
        Ok(Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
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

    async fn send_once(&self, query: &str, count: u8) -> Result<reqwest::Response, AppError> {
        self.http
            .get(format!("{}/res/v1/web/search", self.base_url))
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .query(&[("q", query), ("count", &count.to_string())])
            .timeout(Duration::from_millis(self.timeout_ms))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        ms: self.timeout_ms,
                    }
                } else {
                    // Transport-level failure (connect refused, reset) — retryable.
                    AppError::SearchProvider(format!("transport: {e}"))
                }
            })
    }
}

#[async_trait::async_trait]
impl SearchProvider for BraveClient {
    async fn search(&self, query: &str, count: u8) -> Result<Vec<SearchHit>, AppError> {
        let attempts_max = self.max_retries.saturating_add(1);
        let mut last_error = String::new();

        for attempt in 1..=attempts_max {
            if attempt > 1 {
                let backoff = self
                    .backoff_base_ms
                    .saturating_mul(1 << (attempt - 2).min(8));
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }

            let response = match self.send_once(query, count).await {
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
                return Err(AppError::SearchProvider(format!("HTTP {status}: {detail}")));
            }

            let payload: SearchResponse = response.json().await.map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        ms: self.timeout_ms,
                    }
                } else {
                    AppError::SearchProvider(format!("response body unreadable: {e}"))
                }
            })?;
            // An absent `web` key is a valid empty result (e.g. a query with
            // no web hits), not an error — the pipeline reports honest gaps.
            return Ok(payload
                .web
                .map(|w| w.results)
                .unwrap_or_default()
                .into_iter()
                .map(|r| SearchHit {
                    url: r.url,
                    title: r.title,
                    snippet: r.description,
                })
                .collect());
        }

        Err(AppError::RetriesExhausted {
            attempts: attempts_max,
            last: last_error,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    web: Option<WebResults>,
}

#[derive(Debug, Deserialize)]
struct WebResults {
    #[serde(default)]
    results: Vec<WebResult>,
}

#[derive(Debug, Deserialize)]
struct WebResult {
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config() -> Config {
        Config {
            anthropic_api_key: "test-key".into(),
            anthropic_model: "claude-opus-4-8".into(),
            verify_ensemble_k: 3,
            input_max_chars: 50_000,
            voyage_api_key: None,
            voyage_model: "voyage-4".into(),
            memory_recall_limit: 5,
            brave_api_key: Some("brave-test-key".into()),
            fetch_timeout_ms: 10_000,
            research_concurrency: 8,
            database_path: ":memory:".into(),
            log_level: "info".into(),
            request_timeout_ms: 2_000,
            max_retries: 2,
        }
    }

    fn client_for(mock: &MockServer) -> BraveClient {
        BraveClient::with_base_url(&test_config(), &mock.uri())
            .unwrap()
            .with_backoff_base_ms(1)
    }

    fn ok_body() -> serde_json::Value {
        json!({
            "type": "search",
            "web": {
                "results": [
                    { "url": "https://example.com/a", "title": "A", "description": "first" },
                    { "url": "https://example.com/b", "title": "B", "description": "" }
                ]
            }
        })
    }

    #[tokio::test]
    async fn missing_key_is_a_loud_construction_error() {
        let mut config = test_config();
        config.brave_api_key = None;
        let Err(err) = BraveClient::with_base_url(&config, "http://localhost") else {
            panic!("expected construction to fail without a key");
        };
        assert!(matches!(err, AppError::SearchProvider(_)), "got: {err}");
    }

    #[tokio::test]
    async fn happy_path_carries_auth_and_parses_hits() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .and(header("x-subscription-token", "brave-test-key"))
            .and(query_param("q", "rust"))
            .and(query_param("count", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
            .mount(&mock)
            .await;

        let hits = client_for(&mock).search("rust", 2).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://example.com/a");
        assert_eq!(hits[0].snippet, "first");
        assert_eq!(hits[1].title, "B");
    }

    #[tokio::test]
    async fn absent_web_key_is_an_empty_result_not_an_error() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "type": "search" })))
            .mount(&mock)
            .await;

        let hits = client_for(&mock).search("obscure", 5).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn persistent_5xx_exhausts_retries_with_attempt_count() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3) // max_retries = 2 → 3 attempts total
            .mount(&mock)
            .await;

        let err = client_for(&mock).search("q", 5).await.unwrap_err();
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
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
            .mount(&mock)
            .await;

        let hits = client_for(&mock).search("q", 2).await.unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn slow_provider_is_a_timeout_not_a_retry() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(ok_body())
                    .set_delay(Duration::from_secs(10)),
            )
            .expect(1) // terminal: no second attempt
            .mount(&mock)
            .await;

        let err = client_for(&mock).search("q", 2).await.unwrap_err();
        assert!(matches!(err, AppError::Timeout { ms: 2_000 }), "got: {err}");
    }

    #[tokio::test]
    async fn non_retryable_4xx_is_terminal_and_classified() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(422).set_body_string("SUBSCRIPTION_TOKEN_INVALID"))
            .expect(1)
            .mount(&mock)
            .await;

        let err = client_for(&mock).search("q", 2).await.unwrap_err();
        match err {
            AppError::SearchProvider(msg) => {
                assert!(
                    msg.contains("422") && msg.contains("SUBSCRIPTION_TOKEN_INVALID"),
                    "{msg}"
                );
            }
            other => panic!("expected SearchProvider, got {other}"),
        }
    }

    #[tokio::test]
    async fn unreadable_body_is_out_of_contract() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock)
            .await;

        let err = client_for(&mock).search("q", 2).await.unwrap_err();
        assert!(matches!(err, AppError::SearchProvider(_)), "got: {err}");
        assert!(err.to_string().contains("unreadable"));
    }
}
