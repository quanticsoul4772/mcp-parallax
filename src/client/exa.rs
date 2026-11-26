//! Thin Exa client implementing the [`SearchProvider`] seam (059).
//!
//! Two read-only endpoints, `x-api-key` auth: `POST /search` returns URLs
//! and titles (no text requested, so text is not billed for candidates the
//! pipeline drops), and `POST /contents` returns the text of the kept
//! sources (research.md D1). Request and response shapes follow Exa's
//! OpenAPI spec and an earlier Exa integration.

use super::retry::{cap_detail, retry_after, time_until};
use crate::config::Config;
use crate::error::AppError;
use crate::traits::search::{
    is_truncated, ContentsRequest, ContentsResults, Page, SearchHit, SearchProvider, SearchRequest,
    SearchResults, DEADLINE, NOT_SENT,
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
    max_retries: u32,
    backoff_base_ms: u64,
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
            max_retries: config.max_retries,
            backoff_base_ms: 200,
        }
    }

    /// Shrink the retry backoff base (test-only speedup).
    #[doc(hidden)]
    #[must_use]
    pub const fn with_backoff_base_ms(mut self, ms: u64) -> Self {
        self.backoff_base_ms = ms;
        self
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

    /// POST to a read-only endpoint (059 D9). Network errors, 429 and 5xx
    /// are retried up to `MAX_RETRIES`, waiting `Retry-After` when the
    /// provider gives one, else an exponential backoff; any other 4xx is the
    /// provider refusing the request (057's `provider_rejected`), terminal,
    /// with at most [`super::retry::DETAIL_MAX_CHARS`] of its body. A timeout used the whole
    /// per-request budget and is terminal too.
    ///
    /// 059 review finding 1: `time_left` (the run's remaining deadline)
    /// bounds every attempt and every wait, and a `Retry-After` longer than
    /// the request timeout ends the retries instead of being slept. Returns
    /// whether a network error was retried on the way (finding 4: the lost
    /// attempt may have been billed).
    async fn post(
        &self,
        path: &str,
        body: &Value,
        time_left: Option<Duration>,
    ) -> Result<Sent, AppError> {
        let deadline = time_left.map(|left| std::time::Instant::now() + left);
        let deadline_ms = time_left.map_or(0, |left| {
            u64::try_from(left.as_millis()).unwrap_or(u64::MAX)
        });
        let request_timeout = Duration::from_millis(self.timeout_ms);
        let attempts_max = self.max_retries.saturating_add(1);
        let mut last_error = String::new();
        let mut retried_transport_error = false;
        for attempt in 1..=attempts_max {
            let budget = deadline
                .map(time_until)
                .map_or(request_timeout, |left| left.min(request_timeout));
            if budget.is_zero() {
                // L3: nothing has gone out yet on the first attempt, so the
                // call cost nothing; after that, earlier attempts were sent.
                return Err(if attempt == 1 {
                    AppError::Timeout {
                        what: NOT_SENT,
                        ms: deadline_ms,
                    }
                } else {
                    Self::deadline_reached(deadline_ms)
                });
            }
            let response = match self
                .http
                .post(format!("{}{path}", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("Accept", "application/json")
                .json(body)
                .timeout(budget)
                .send()
                .await
            {
                Ok(response) => response,
                Err(e) if e.is_timeout() => {
                    return Err(if budget < request_timeout {
                        Self::deadline_reached(deadline_ms)
                    } else {
                        self.timeout()
                    });
                }
                Err(e) => {
                    last_error = self.transport_error(&e).to_string();
                    retried_transport_error = true;
                    self.wait_before_retry(attempt, attempts_max, None, deadline, deadline_ms)
                        .await?;
                    continue;
                }
            };
            let status = response.status();
            if status.is_success() {
                return Ok(Sent {
                    response,
                    retried_transport_error,
                    deadline_ms: (budget < request_timeout).then_some(deadline_ms),
                });
            }
            if status.as_u16() == 429 || status.is_server_error() {
                let asked = retry_after(
                    response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok()),
                    chrono::Utc::now(),
                );
                last_error = format!("HTTP {status}");
                if asked.is_some_and(|wait| wait > request_timeout) {
                    return Err(AppError::RetriesExhausted {
                        attempts: attempt,
                        last: format!(
                            "{last_error} (Retry-After longer than the request timeout, not waited)"
                        ),
                    });
                }
                self.wait_before_retry(attempt, attempts_max, asked, deadline, deadline_ms)
                    .await?;
                continue;
            }
            let detail = cap_detail(&response.text().await.unwrap_or_default());
            return Err(AppError::ProviderRejected {
                status: status.as_u16(),
                detail,
            });
        }
        Err(AppError::RetriesExhausted {
            attempts: attempts_max,
            last: last_error,
        })
    }

    /// Sleep before the next attempt — the provider's `Retry-After`, else the
    /// backoff — unless this was the last attempt. A wait that would outlast
    /// the run's deadline sleeps only until the deadline and then reports it
    /// (second review M1), so the pipeline's clock sees the deadline passed
    /// and stops the run early rather than failing it.
    async fn wait_before_retry(
        &self,
        attempt: u32,
        attempts_max: u32,
        asked: Option<Duration>,
        deadline: Option<std::time::Instant>,
        deadline_ms: u64,
    ) -> Result<(), AppError> {
        if attempt >= attempts_max {
            return Ok(());
        }
        let backoff = Duration::from_millis(
            self.backoff_base_ms
                .saturating_mul(1 << (attempt - 1).min(8)),
        );
        let wait = asked.unwrap_or(backoff);
        if let Some(left) = deadline.map(time_until).filter(|left| wait >= *left) {
            tokio::time::sleep(left).await;
            return Err(Self::deadline_reached(deadline_ms));
        }
        tokio::time::sleep(wait).await;
        Ok(())
    }

    /// The run's deadline arrived during an Exa call; `ms` is the time the
    /// call was given.
    const fn deadline_reached(ms: u64) -> AppError {
        AppError::Timeout { what: DEADLINE, ms }
    }

    async fn read<T: serde::de::DeserializeOwned>(&self, sent: Sent) -> Result<T, AppError> {
        let deadline_ms = sent.deadline_ms;
        sent.response.json().await.map_err(|e| {
            if e.is_timeout() {
                // Second review L2: the deadline bounded this reply, so the
                // deadline is what cut the body off.
                deadline_ms.map_or_else(|| self.timeout(), Self::deadline_reached)
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
        let sent = self
            .post("/search", &Self::search_body(request), request.time_left)
            .await?;
        let retried = sent.retried_transport_error;
        let payload: SearchPayload = self.read(sent).await?;
        Ok(SearchResults {
            hits: payload
                .results
                .into_iter()
                .map(|r| SearchHit {
                    title: title_or_url(r.title, &r.url),
                    url: r.url,
                })
                .collect(),
            cost_usd: reported_cost(payload.cost_dollars.and_then(|c| c.total), retried),
        })
    }

    async fn contents(&self, request: &ContentsRequest) -> Result<ContentsResults, AppError> {
        let sent = self
            .post(
                "/contents",
                &Self::contents_body(request),
                request.time_left,
            )
            .await?;
        let retried = sent.retried_transport_error;
        let payload: ContentsPayload = self.read(sent).await?;
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
            cost_usd: reported_cost(payload.cost_dollars.and_then(|c| c.total), retried),
        })
    }
}

/// The cost a reply can be credited with (059 review finding 4): what it
/// reported, unless a network error was retried on the way — the provider may
/// have billed the lost attempt, so the call's true cost is unknown.
const fn reported_cost(parsed: Option<f64>, retried_transport_error: bool) -> Option<f64> {
    if retried_transport_error {
        None
    } else {
        parsed
    }
}

/// A reply that arrived, with what the cost and error labels need to know.
struct Sent {
    response: reqwest::Response,
    /// A network error was retried on the way (finding 4).
    retried_transport_error: bool,
    /// The run's deadline bounded this reply's time: the time the call was
    /// given, in ms (second review L2).
    deadline_ms: Option<u64>,
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
            .with_backoff_base_ms(1)
    }

    fn search_request(include: &[&str], exclude: &[&str]) -> SearchRequest {
        SearchRequest {
            query: "rust".into(),
            count: 4,
            search_type: SearchType::DeepLite,
            include_domains: include.iter().map(|d| (*d).to_string()).collect(),
            exclude_domains: exclude.iter().map(|d| (*d).to_string()).collect(),
            time_left: None,
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
            time_left: None,
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
            time_left: None,
        };
        let pages = client_for(&mock).contents(&request).await.unwrap().pages;
        assert_eq!(pages[0].requested_url, "http://example.com/a");
        assert_eq!(pages[0].url, "https://example.com/a-moved");
        // No id: the URL stands in for it.
        assert_eq!(pages[1].requested_url, "https://example.com/b");
    }

    /// 059 D9 (057's rule): a 4xx other than 429 is the provider refusing
    /// the request — terminal, not retried, recorded as provider_rejected.
    #[tokio::test]
    async fn a_client_error_is_provider_rejected_and_not_retried() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
            .expect(1)
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .search(&search_request(&[], &[]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::ProviderRejected { status: 400, detail } if detail.contains("bad request")),
            "{err:?}"
        );
    }

    /// A 429 is retried (here with `Retry-After: 0`) and the request then
    /// succeeds — on both endpoints.
    #[tokio::test]
    async fn a_rate_limit_is_retried_then_succeeds() {
        let mock = MockServer::start().await;
        for endpoint in ["/search", "/contents"] {
            Mock::given(method("POST"))
                .and(path(endpoint))
                .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
                .up_to_n_times(1)
                .with_priority(1)
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path(endpoint))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
                .with_priority(2)
                .mount(&mock)
                .await;
        }
        let client = client_for(&mock);
        client.search(&search_request(&[], &[])).await.unwrap();
        client
            .contents(&ContentsRequest {
                urls: vec!["https://example.com/a".into()],
                max_age_hours: 168,
                max_characters: 100,
                time_left: None,
            })
            .await
            .unwrap();
        assert_eq!(mock.received_requests().await.unwrap().len(), 4);
    }

    /// A server error is retried up to MAX_RETRIES (2 in the test config:
    /// three attempts), then reported as exhausted with the attempt count.
    #[tokio::test]
    async fn a_persistent_server_error_exhausts_the_retries() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3)
            .mount(&mock)
            .await;

        let err = client_for(&mock)
            .search(&search_request(&[], &[]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::RetriesExhausted { attempts: 3, last } if last.contains("503")),
            "{err:?}"
        );
    }

    /// 059 review finding 1: a `Retry-After` longer than the request timeout
    /// is not slept — the call fails at once with the last error.
    #[tokio::test]
    async fn a_retry_after_beyond_the_request_timeout_fails_without_waiting() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "3600"))
            .expect(1)
            .mount(&mock)
            .await;
        let started = std::time::Instant::now();
        let err = client_for(&mock)
            .search(&search_request(&[], &[]))
            .await
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5), "it waited");
        assert!(
            matches!(&err, AppError::RetriesExhausted { attempts: 1, last } if last.contains("Retry-After")),
            "{err:?}"
        );
    }

    /// 059 review finding 1: the time left before the run's deadline bounds
    /// the call, so a slow reply cannot carry the run past it.
    #[tokio::test]
    async fn the_time_left_bounds_the_call() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "results": [] }))
                    .set_delay(Duration::from_secs(5)),
            )
            .mount(&mock)
            .await;
        // A long request timeout, so only the time left can cut the call short.
        let config = crate::config::Config {
            request_timeout_ms: 30_000,
            ..test_config()
        };
        let client = ExaClient::with_base_url(&config, "exa-test-key", &mock.uri());
        let started = std::time::Instant::now();
        let mut request = search_request(&[], &[]);
        request.time_left = Some(Duration::from_millis(300));
        let err = client.search(&request).await.unwrap_err();
        assert!(
            started.elapsed() < Duration::from_millis(1_500),
            "the deadline did not bound it"
        );
        assert!(
            matches!(&err, AppError::Timeout { what, .. } if *what == DEADLINE),
            "{err:?}"
        );
    }

    /// Second review M1: when the next retry wait would pass the deadline, the
    /// client waits until the deadline and only then reports it — so the
    /// pipeline's clock sees the deadline passed and stops the run early.
    #[tokio::test]
    async fn a_wait_past_the_deadline_sleeps_until_it_then_reports_it() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "2"))
            .mount(&mock)
            .await;
        let started = std::time::Instant::now();
        let mut request = search_request(&[], &[]);
        request.time_left = Some(Duration::from_millis(300));
        let err = client_for(&mock).search(&request).await.unwrap_err();
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(250),
            "gave up early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1_500),
            "overran: {elapsed:?}"
        );
        assert!(
            matches!(&err, AppError::Timeout { what, ms: 300 } if *what == DEADLINE),
            "{err:?}"
        );
    }

    /// Second review L2: a deadline that cuts off reading the body is reported
    /// as the deadline, not as a request timeout. The stand-in sends headers
    /// and then stalls, which wiremock cannot do.
    #[tokio::test]
    async fn a_body_cut_off_by_the_deadline_is_reported_as_the_deadline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{",
                )
                .await;
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        let config = crate::config::Config {
            request_timeout_ms: 30_000,
            ..test_config()
        };
        let client =
            ExaClient::with_base_url(&config, "exa-test-key", &format!("http://{address}"));
        let mut request = search_request(&[], &[]);
        request.time_left = Some(Duration::from_millis(300));
        let err = client.search(&request).await.unwrap_err();
        assert!(
            matches!(&err, AppError::Timeout { what, .. } if *what == DEADLINE),
            "{err:?}"
        );
    }

    /// Second review L3: with no time left the client sends nothing, and says
    /// so, so the run's cost can count the call as free rather than unknown.
    #[tokio::test]
    async fn with_no_time_left_nothing_is_sent() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
            .expect(0)
            .mount(&mock)
            .await;
        let mut request = search_request(&[], &[]);
        request.time_left = Some(Duration::ZERO);
        let err = client_for(&mock).search(&request).await.unwrap_err();
        assert!(
            matches!(&err, AppError::Timeout { what, .. } if *what == NOT_SENT),
            "{err:?}"
        );
    }

    /// 059 review finding 4: a reply that followed a retried network error
    /// may have been billed twice, so its cost is unknown, not the one figure.
    #[test]
    fn a_cost_after_a_retried_network_error_is_unknown() {
        assert_eq!(reported_cost(Some(0.007), false), Some(0.007));
        assert_eq!(reported_cost(Some(0.007), true), None);
        assert_eq!(reported_cost(None, false), None);
    }

    /// 059 review finding 7: a rejection keeps at most 2,000 characters of the
    /// provider's body, marking the cut.
    #[tokio::test]
    async fn a_long_rejection_body_is_cut_and_marked() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("x".repeat(5_000)))
            .mount(&mock)
            .await;
        let err = client_for(&mock)
            .search(&search_request(&[], &[]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::ProviderRejected { detail, .. }
                if detail.ends_with("… (truncated)")
                    && detail.chars().filter(|c| *c == 'x').count() == 2_000),
            "{err:?}"
        );
    }

    /// `Retry-After` is seconds or an HTTP date (RFC 9110); a date in the
    /// past means no wait; anything else is not a delay.
    #[test]
    fn retry_after_reads_seconds_and_http_dates() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(retry_after(Some("2"), now), Some(Duration::from_secs(2)));
        assert_eq!(
            retry_after(Some("Sun, 22 Nov 2026 12:00:03 GMT"), now),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            retry_after(Some("Sun, 22 Nov 2026 11:59:00 GMT"), now),
            Some(Duration::ZERO)
        );
        assert_eq!(retry_after(Some("soon"), now), None);
        assert_eq!(retry_after(None, now), None);
    }
}
