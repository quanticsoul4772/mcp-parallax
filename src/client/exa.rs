//! Thin Exa client implementing the [`SearchProvider`] seam (059).
//!
//! Two read-only endpoints, `x-api-key` auth: `POST /search` returns URLs
//! and titles (no text requested, so text is not billed for candidates the
//! pipeline drops), and `POST /contents` returns the text of the kept
//! sources (research.md D1). Request and response shapes follow Exa's
//! OpenAPI spec and the user's exa-mcp-server.

use crate::config::Config;
use crate::error::AppError;
use crate::traits::search::{
    is_truncated, ContentsRequest, ContentsResults, Page, SearchHit, SearchProvider, SearchRequest,
    SearchResults,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const EXA_API_BASE: &str = "https://api.exa.ai";

/// Thin `reqwest` client implementing [`SearchProvider`] against Exa.
pub struct ExaClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    timeout_ms: u64,
}

impl ExaClient {
    /// Build a client from configuration, targeting Exa's endpoint.
    ///
    /// # Errors
    ///
    /// Errors when `exa_api_key` is absent — callers gate construction on
    /// key presence, so reaching this without a key is a wiring bug surfaced
    /// loudly.
    pub fn new(config: &Config) -> Result<Self, AppError> {
        let api_key = config.exa_api_key.as_deref().ok_or_else(|| {
            AppError::SearchProvider(
                "ExaClient constructed without EXA_API_KEY — gate on key presence".into(),
            )
        })?;
        Ok(Self::with_base_url(config, api_key, EXA_API_BASE))
    }

    /// Build a client against `base_url` with the given key (tests point
    /// this at a local wiremock server).
    #[must_use]
    pub fn with_base_url(config: &Config, api_key: &str, base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            timeout_ms: config.request_timeout_ms,
        }
    }

    /// The `POST /search` body: no `contents`, so no text is returned or
    /// billed; domain lists only when non-empty.
    fn search_body(request: &SearchRequest) -> Value {
        let mut body = json!({
            "query": request.query,
            "type": request.search_type.as_str(),
            "numResults": request.count,
        });
        if !request.include_domains.is_empty() {
            body["includeDomains"] = json!(request.include_domains);
        }
        if !request.exclude_domains.is_empty() {
            body["excludeDomains"] = json!(request.exclude_domains);
        }
        body
    }

    /// The `POST /contents` body for the kept sources.
    fn contents_body(request: &ContentsRequest) -> Value {
        json!({
            "urls": request.urls,
            "text": { "maxCharacters": request.max_characters },
            "maxAgeHours": request.max_age_hours,
        })
    }

    async fn post(&self, path: &str, body: &Value) -> Result<reqwest::Response, AppError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("Accept", "application/json")
            .json(body)
            .timeout(Duration::from_millis(self.timeout_ms))
            .send()
            .await
            .map_err(|e| self.transport_error(&e))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(AppError::SearchProvider(format!("HTTP {status}: {detail}")));
        }
        Ok(response)
    }

    async fn read<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, AppError> {
        response.json().await.map_err(|e| {
            if e.is_timeout() {
                self.timeout()
            } else {
                AppError::SearchProvider(format!("response body unreadable: {e}"))
            }
        })
    }

    fn transport_error(&self, e: &reqwest::Error) -> AppError {
        if e.is_timeout() {
            self.timeout()
        } else {
            AppError::SearchProvider(format!("transport: {e}"))
        }
    }

    const fn timeout(&self) -> AppError {
        AppError::Timeout {
            what: "request",
            ms: self.timeout_ms,
        }
    }
}

#[async_trait::async_trait]
impl SearchProvider for ExaClient {
    async fn search(&self, request: &SearchRequest) -> Result<SearchResults, AppError> {
        let response = self.post("/search", &Self::search_body(request)).await?;
        let payload: SearchPayload = self.read(response).await?;
        Ok(SearchResults {
            hits: payload
                .results
                .into_iter()
                .map(|r| SearchHit {
                    title: title_or_url(r.title, &r.url),
                    url: r.url,
                })
                .collect(),
            cost_usd: payload.cost_dollars.and_then(|c| c.total),
        })
    }

    async fn contents(&self, request: &ContentsRequest) -> Result<ContentsResults, AppError> {
        let response = self
            .post("/contents", &Self::contents_body(request))
            .await?;
        let payload: ContentsPayload = self.read(response).await?;
        let failed: Vec<String> = payload
            .statuses
            .into_iter()
            .filter(|s| s.status == "error")
            .map(|s| s.id)
            .collect();
        Ok(ContentsResults {
            pages: payload
                .results
                .into_iter()
                .filter(|r| {
                    !failed
                        .iter()
                        .any(|id| *id == r.url || Some(id) == r.id.as_ref())
                })
                .map(|r| {
                    let text = r.text.unwrap_or_default();
                    Page {
                        truncated: is_truncated(text.chars().count(), request.max_characters),
                        title: title_or_url(r.title, &r.url),
                        // Exa echoes the requested identifier as `id`.
                        requested_url: r.id.clone().unwrap_or_else(|| r.url.clone()),
                        url: r.url,
                        text,
                    }
                })
                .collect(),
            cost_usd: payload.cost_dollars.and_then(|c| c.total),
        })
    }
}

/// The provider's title, or the URL when it gives none.
fn title_or_url(title: Option<String>, url: &str) -> String {
    title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| url.to_string())
}

#[derive(Debug, Deserialize)]
struct SearchPayload {
    #[serde(default)]
    results: Vec<ResultPayload>,
    #[serde(rename = "costDollars")]
    cost_dollars: Option<CostPayload>,
}

#[derive(Debug, Deserialize)]
struct ContentsPayload {
    #[serde(default)]
    results: Vec<ResultPayload>,
    #[serde(default)]
    statuses: Vec<StatusPayload>,
    #[serde(rename = "costDollars")]
    cost_dollars: Option<CostPayload>,
}

#[derive(Debug, Deserialize)]
struct ResultPayload {
    id: Option<String>,
    url: String,
    title: Option<String>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatusPayload {
    id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct CostPayload {
    total: Option<f64>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::config::test_config;
    use crate::traits::search::SearchType;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(mock: &MockServer) -> ExaClient {
        ExaClient::with_base_url(&test_config(), "exa-test-key", &mock.uri())
    }

    fn search_request(include: &[&str], exclude: &[&str]) -> SearchRequest {
        SearchRequest {
            query: "rust".into(),
            count: 4,
            search_type: SearchType::DeepLite,
            include_domains: include.iter().map(|d| (*d).to_string()).collect(),
            exclude_domains: exclude.iter().map(|d| (*d).to_string()).collect(),
        }
    }

    async fn sent_body(mock: &MockServer) -> Value {
        let requests = mock.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        serde_json::from_slice(&requests[0].body).unwrap()
    }

    #[tokio::test]
    async fn search_posts_type_count_and_no_contents() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .and(header("x-api-key", "exa-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "requestId": "r1",
                "results": [
                    { "id": "a", "url": "https://example.com/a", "title": "A" },
                    { "id": "b", "url": "https://example.com/b", "title": "" }
                ]
            })))
            .mount(&mock)
            .await;

        let results = client_for(&mock)
            .search(&search_request(&[], &[]))
            .await
            .unwrap();
        assert_eq!(results.hits.len(), 2);
        assert_eq!(results.hits[0].title, "A");
        // No title: the URL stands in.
        assert_eq!(results.hits[1].title, "https://example.com/b");

        let body = sent_body(&mock).await;
        assert_eq!(body["query"], "rust");
        assert_eq!(body["type"], "deep-lite");
        assert_eq!(body["numResults"], 4);
        assert!(body.get("contents").is_none(), "no text on search: {body}");
        assert!(body.get("includeDomains").is_none(), "{body}");
        assert!(body.get("excludeDomains").is_none(), "{body}");
    }

    #[tokio::test]
    async fn search_sends_domain_lists_when_given() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
            .mount(&mock)
            .await;

        client_for(&mock)
            .search(&search_request(&["docs.rs"], &["spam.example"]))
            .await
            .unwrap();
        let body = sent_body(&mock).await;
        assert_eq!(body["includeDomains"], json!(["docs.rs"]));
        assert_eq!(body["excludeDomains"], json!(["spam.example"]));
    }

    #[tokio::test]
    async fn contents_posts_urls_cap_and_freshness_and_maps_pages() {
        let mock = MockServer::start().await;
        let long = "x".repeat(100);
        Mock::given(method("POST"))
            .and(path("/contents"))
            .and(header("x-api-key", "exa-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [
                    { "id": "https://example.com/a", "url": "https://example.com/a", "title": "A", "text": "short" },
                    { "id": "https://example.com/b", "url": "https://example.com/b", "title": "B", "text": long },
                    { "id": "https://example.com/c", "url": "https://example.com/c", "title": "C" },
                    { "id": "https://example.com/d", "url": "https://example.com/d", "title": "D", "text": "" }
                ],
                "statuses": [
                    { "id": "https://example.com/a", "status": "success", "source": "cached" },
                    { "id": "https://example.com/d", "status": "error", "error": { "tag": "CRAWL_NOT_FOUND" } }
                ]
            })))
            .mount(&mock)
            .await;

        let request = ContentsRequest {
            urls: vec![
                "https://example.com/a".into(),
                "https://example.com/b".into(),
                "https://example.com/c".into(),
                "https://example.com/d".into(),
            ],
            max_age_hours: 168,
            max_characters: 100,
        };
        let results = client_for(&mock).contents(&request).await.unwrap();

        let body = sent_body(&mock).await;
        assert_eq!(body["urls"].as_array().unwrap().len(), 4);
        assert_eq!(body["text"]["maxCharacters"], 100);
        assert_eq!(body["maxAgeHours"], 168);

        // d failed and is absent; c has no text and comes back empty.
        let urls: Vec<&str> = results.pages.iter().map(|p| p.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://example.com/a",
                "https://example.com/b",
                "https://example.com/c"
            ]
        );
        assert!(!results.pages[0].truncated);
        assert!(results.pages[1].truncated, "text at the cap is truncated");
        assert_eq!(results.pages[2].text, "");
        assert!(!results.pages[2].truncated);
    }

    /// A page Exa returns under another address (a redirect) keeps the
    /// address that was requested (`id`), so research can still pair it with
    /// its source; `url` is the page's own address.
    #[tokio::test]
    async fn a_redirected_page_keeps_the_requested_address() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/contents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [
                    { "id": "http://example.com/a", "url": "https://example.com/a-moved", "title": "A", "text": "body" },
                    { "url": "https://example.com/b", "title": "B", "text": "body" }
                ]
            })))
            .mount(&mock)
            .await;

        let request = ContentsRequest {
            urls: vec![
                "http://example.com/a".into(),
                "https://example.com/b".into(),
            ],
            max_age_hours: 168,
            max_characters: 100,
        };
        let pages = client_for(&mock).contents(&request).await.unwrap().pages;
        assert_eq!(pages[0].requested_url, "http://example.com/a");
        assert_eq!(pages[0].url, "https://example.com/a-moved");
        // No id: the URL stands in for it.
        assert_eq!(pages[1].requested_url, "https://example.com/b");
    }

    #[tokio::test]
    async fn a_client_error_is_a_search_provider_error() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .search(&search_request(&[], &[]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::SearchProvider(m) if m.contains("400")),
            "{err:?}"
        );
    }
}
