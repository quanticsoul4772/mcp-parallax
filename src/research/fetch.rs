//! The hygiene-enforcing [`Fetcher`] implementation (research.md 004 D5).
//!
//! Every guard from RESEARCH_PRIMITIVE.md §6: per-fetch timeout, redirect
//! cap, streaming size cap (never trusts Content-Length), content-type
//! allowlist, per-domain politeness (one in-flight request per domain plus
//! minimum spacing), robots.txt (fail-open on robots *fetch* errors,
//! fail-closed on explicit disallow), and allow/deny domain lists — the
//! pipeline pre-filters candidates by domain, and this fetcher re-checks
//! both the requested and the post-redirect URL, so a redirect cannot escape
//! into a denied domain.

use crate::error::AppError;
use crate::traits::fetcher::{FetchedPage, Fetcher};
use robotstxt::matcher::{LongestMatchRobotsMatchStrategy, RobotsMatcher};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Maximum response body bytes (enforced while streaming).
pub const FETCH_MAX_BYTES: usize = 2_000_000;
/// Redirect cap.
pub const FETCH_MAX_REDIRECTS: usize = 5;
/// Minimum spacing between requests to the same domain.
pub const DOMAIN_SPACING_MS: u64 = 300;
/// The user agent presented to sites and matched against robots.txt rules.
pub const USER_AGENT: &str = "parallax-research";
/// Content types we accept (prefix match against the `Content-Type` header).
const CONTENT_TYPE_ALLOW: &[&str] = &["text/html", "text/plain", "application/xhtml+xml"];

/// Per-run fetch policy.
#[derive(Debug, Clone)]
pub struct FetchPolicy {
    /// Per-fetch timeout (config `FETCH_TIMEOUT_MS`).
    pub timeout_ms: u64,
    /// Restrict to these registrable domains when non-empty.
    pub domains_allow: Vec<String>,
    /// Never fetch these domains. Absolute.
    pub domains_deny: Vec<String>,
    /// Minimum spacing between same-domain requests (tests set 0).
    pub domain_spacing_ms: u64,
}

#[derive(Default)]
struct DomainState {
    last_request: Option<Instant>,
}

/// The production [`Fetcher`]: one per research run (the robots cache is
/// run-scoped — research.md D5).
pub struct HygieneFetcher {
    http: reqwest::Client,
    policy: FetchPolicy,
    /// host → robots.txt body (None: robots fetch failed → fail-open).
    robots: Mutex<HashMap<String, Option<Arc<str>>>>,
    /// host → politeness state; the per-domain mutex serializes in-flight
    /// requests to one per domain.
    domains: Mutex<HashMap<String, Arc<Mutex<DomainState>>>>,
}

impl HygieneFetcher {
    /// Build a fetcher for one run.
    ///
    /// # Errors
    ///
    /// `SearchProvider`-class when the HTTP client cannot be constructed.
    pub fn new(policy: FetchPolicy) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::limited(FETCH_MAX_REDIRECTS))
            .build()
            .map_err(|e| AppError::SearchProvider(format!("fetch client: {e}")))?;
        Ok(Self {
            http,
            policy,
            robots: Mutex::new(HashMap::new()),
            domains: Mutex::new(HashMap::new()),
        })
    }

    fn check_domain(&self, host: &str, what: &str) -> Result<(), AppError> {
        if self
            .policy
            .domains_deny
            .iter()
            .any(|d| crate::research::domain_matches(host, d))
        {
            return Err(AppError::SearchProvider(format!(
                "{what} domain {host} is denied"
            )));
        }
        if !self.policy.domains_allow.is_empty()
            && !self
                .policy
                .domains_allow
                .iter()
                .any(|d| crate::research::domain_matches(host, d))
        {
            return Err(AppError::SearchProvider(format!(
                "{what} domain {host} is outside the allow list"
            )));
        }
        Ok(())
    }

    async fn domain_state(&self, host: &str) -> Arc<Mutex<DomainState>> {
        Arc::clone(
            self.domains
                .lock()
                .await
                .entry(host.to_string())
                .or_default(),
        )
    }

    /// robots.txt body for an origin (scheme://host:port — the port matters,
    /// not least under test), fetched once per run. `None` = the robots fetch
    /// itself failed → fail-open (treat as allowed).
    async fn robots_body(&self, origin: &str) -> Option<Arc<str>> {
        if let Some(cached) = self.robots.lock().await.get(origin) {
            return cached.clone();
        }
        let url = format!("{origin}/robots.txt");
        let body = match self
            .http
            .get(&url)
            .timeout(Duration::from_millis(self.policy.timeout_ms))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                response.text().await.ok().map(Arc::<str>::from)
            }
            // 404/4xx/5xx/transport: no readable robots → fail-open.
            _ => None,
        };
        self.robots
            .lock()
            .await
            .insert(origin.to_string(), body.clone());
        body
    }

    async fn read_capped(&self, response: reqwest::Response) -> Result<Vec<u8>, AppError> {
        use futures::StreamExt;
        let mut stream = response.bytes_stream();
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        ms: self.policy.timeout_ms,
                    }
                } else {
                    AppError::SearchProvider(format!("body read: {e}"))
                }
            })?;
            if body.len() + chunk.len() > FETCH_MAX_BYTES {
                return Err(AppError::SearchProvider(format!(
                    "body exceeds the {FETCH_MAX_BYTES}-byte cap"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

#[async_trait::async_trait]
impl Fetcher for HygieneFetcher {
    // The per-domain lock is deliberately held across the request — that IS
    // the one-in-flight-per-domain politeness guarantee.
    #[allow(clippy::significant_drop_tightening)]
    async fn fetch(&self, url: &str) -> Result<FetchedPage, AppError> {
        let parsed = reqwest::Url::parse(url)
            .map_err(|e| AppError::SearchProvider(format!("unfetchable url {url:?}: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(AppError::SearchProvider(format!(
                "unsupported scheme {:?}",
                parsed.scheme()
            )));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| AppError::SearchProvider(format!("url {url:?} has no host")))?
            .to_string();
        self.check_domain(&host, "requested")?;

        // robots.txt: fail-closed on explicit disallow, fail-open when no
        // robots could be read.
        let origin = parsed.origin().ascii_serialization();
        if let Some(body) = self.robots_body(&origin).await {
            let allowed = RobotsMatcher::<LongestMatchRobotsMatchStrategy>::default()
                .one_agent_allowed_by_robots(&body, USER_AGENT, url);
            if !allowed {
                return Err(AppError::SearchProvider(format!(
                    "robots.txt disallows {url}"
                )));
            }
        }

        // Politeness: one in-flight request per domain + minimum spacing.
        let state = self.domain_state(&host).await;
        let mut state = state.lock().await;
        if let Some(last) = state.last_request {
            let spacing = Duration::from_millis(self.policy.domain_spacing_ms);
            let elapsed = last.elapsed();
            if elapsed < spacing {
                tokio::time::sleep(spacing.saturating_sub(elapsed)).await;
            }
        }
        state.last_request = Some(Instant::now());

        let response = self
            .http
            .get(parsed)
            .timeout(Duration::from_millis(self.policy.timeout_ms))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout {
                        ms: self.policy.timeout_ms,
                    }
                } else {
                    AppError::SearchProvider(format!("fetch failed: {e}"))
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(AppError::SearchProvider(format!("HTTP {status} for {url}")));
        }

        // The redirect cap is the client's; the final URL must still satisfy
        // the domain lists — a redirect cannot escape into a denied domain.
        let final_url = response.url().clone();
        let final_host = final_url
            .host_str()
            .ok_or_else(|| AppError::SearchProvider("redirected to a hostless url".to_string()))?
            .to_string();
        self.check_domain(&final_host, "redirected-to")?;

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if !CONTENT_TYPE_ALLOW
            .iter()
            .any(|t| content_type.starts_with(t))
        {
            return Err(AppError::SearchProvider(format!(
                "content-type {content_type:?} is not extractable"
            )));
        }

        let body = self.read_capped(response).await?;
        let html = String::from_utf8_lossy(&body).into_owned();
        Ok(FetchedPage {
            url: final_url.to_string(),
            html,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn policy(timeout_ms: u64) -> FetchPolicy {
        FetchPolicy {
            timeout_ms,
            domains_allow: vec![],
            domains_deny: vec![],
            domain_spacing_ms: 0,
        }
    }

    fn html_response(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.to_string(), "text/html; charset=utf-8")
    }

    async fn serve_robots(mock: &MockServer, body: &str) {
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body.to_string(), "text/plain"))
            .mount(mock)
            .await;
    }

    #[tokio::test]
    async fn fetches_html_and_reports_the_final_url() {
        let mock = MockServer::start().await;
        serve_robots(&mock, "User-agent: *\nAllow: /\n").await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(html_response("<html><body>content</body></html>"))
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(2_000)).unwrap();
        let page = fetcher
            .fetch(&format!("{}/page", mock.uri()))
            .await
            .unwrap();
        assert!(page.html.contains("content"));
        assert!(page.url.ends_with("/page"));
    }

    #[tokio::test]
    async fn robots_disallow_is_fail_closed() {
        let mock = MockServer::start().await;
        serve_robots(&mock, "User-agent: *\nDisallow: /private\n").await;
        Mock::given(method("GET"))
            .and(path("/private/doc"))
            .respond_with(html_response("<html>secret</html>"))
            .expect(0) // the page itself is never requested
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(2_000)).unwrap();
        let err = fetcher
            .fetch(&format!("{}/private/doc", mock.uri()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("robots.txt disallows"), "{err}");
    }

    #[tokio::test]
    async fn missing_robots_is_fail_open() {
        let mock = MockServer::start().await;
        // No robots.txt mount → 404 → fail-open.
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(html_response("<html>ok</html>"))
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(2_000)).unwrap();
        assert!(fetcher.fetch(&format!("{}/page", mock.uri())).await.is_ok());
    }

    #[tokio::test]
    async fn denied_domain_is_rejected_before_any_connection() {
        // No server at all — a connection attempt would error differently.
        let fetcher = HygieneFetcher::new(FetchPolicy {
            domains_deny: vec!["evil.example".into()],
            ..policy(2_000)
        })
        .unwrap();
        let err = fetcher
            .fetch("https://sub.evil.example/page")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is denied"), "{err}");
    }

    #[tokio::test]
    async fn outside_the_allow_list_is_rejected_before_any_connection() {
        let fetcher = HygieneFetcher::new(FetchPolicy {
            domains_allow: vec!["good.example".into()],
            ..policy(2_000)
        })
        .unwrap();
        let err = fetcher
            .fetch("https://other.example/page")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("outside the allow list"), "{err}");
    }

    #[tokio::test]
    async fn wrong_content_type_is_rejected() {
        let mock = MockServer::start().await;
        serve_robots(&mock, "").await;
        Mock::given(method("GET"))
            .and(path("/data.bin"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(vec![0u8; 16], "application/octet-stream"),
            )
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(2_000)).unwrap();
        let err = fetcher
            .fetch(&format!("{}/data.bin", mock.uri()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not extractable"), "{err}");
    }

    #[tokio::test]
    async fn oversized_body_is_cut_off_mid_stream() {
        let mock = MockServer::start().await;
        serve_robots(&mock, "").await;
        let big = "x".repeat(FETCH_MAX_BYTES + 1);
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(big, "text/html"))
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(5_000)).unwrap();
        let err = fetcher
            .fetch(&format!("{}/big", mock.uri()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("byte cap"), "{err}");
    }

    #[tokio::test]
    async fn slow_response_is_a_timeout() {
        let mock = MockServer::start().await;
        serve_robots(&mock, "").await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(html_response("<html>late</html>").set_delay(Duration::from_secs(10)))
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(300)).unwrap();
        let err = fetcher
            .fetch(&format!("{}/slow", mock.uri()))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Timeout { .. }), "{err}");
    }

    #[tokio::test]
    async fn non_success_status_is_dropped_with_the_status_named() {
        let mock = MockServer::start().await;
        serve_robots(&mock, "").await;
        Mock::given(method("GET"))
            .and(path("/gone"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(2_000)).unwrap();
        let err = fetcher
            .fetch(&format!("{}/gone", mock.uri()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("404"), "{err}");
    }

    #[tokio::test]
    async fn same_domain_requests_are_spaced() {
        let mock = MockServer::start().await;
        serve_robots(&mock, "").await;
        Mock::given(method("GET"))
            .respond_with(html_response("<html>ok</html>"))
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(FetchPolicy {
            domain_spacing_ms: 150,
            ..policy(2_000)
        })
        .unwrap();
        let start = std::time::Instant::now();
        fetcher.fetch(&format!("{}/a", mock.uri())).await.unwrap();
        fetcher.fetch(&format!("{}/b", mock.uri())).await.unwrap();
        // Second same-domain request waits out the spacing window.
        assert!(start.elapsed() >= Duration::from_millis(150));
    }
}
