//! The hygiene-enforcing [`Fetcher`] implementation (research.md 004 D5).
//!
//! Every guard from RESEARCH_PRIMITIVE.md §6: per-fetch timeout, manual
//! redirect hops (each hop re-checked against the domain lists, robots.txt,
//! the address guard, and politeness — a redirect cannot escape into a
//! denied domain or skip robots), streaming size caps (never trusts
//! Content-Length, robots.txt included), content-type allowlist, per-domain
//! politeness (one in-flight request per domain plus minimum spacing),
//! robots.txt (fail-open on robots *fetch* errors, fail-closed on explicit
//! disallow), allow/deny domain lists, and a non-global address guard
//! (loopback / private / link-local literals rejected unless
//! `FETCH_ALLOW_PRIVATE` is set — SSRF defense for a server running inside a
//! developer network).

use crate::error::AppError;
use crate::traits::fetcher::{FetchedPage, Fetcher};
use robotstxt::matcher::{LongestMatchRobotsMatchStrategy, RobotsMatcher};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Maximum response body bytes (enforced while streaming).
pub const FETCH_MAX_BYTES: usize = 2_000_000;
/// Maximum robots.txt bytes (same streaming enforcement).
pub const ROBOTS_MAX_BYTES: usize = 512_000;
/// Redirect cap (manual hops — each hop re-runs every guard).
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
    /// Permit loopback/private/link-local targets (`FETCH_ALLOW_PRIVATE`;
    /// off by default — needed only by tests fetching a local mock server).
    pub allow_private: bool,
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
    /// origin → robots.txt body (None: robots fetch failed → fail-open).
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
            // Redirects are followed MANUALLY so every hop re-runs the
            // domain/robots/address/politeness guards.
            .redirect(reqwest::redirect::Policy::none())
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

    /// Reject non-global targets (SSRF defense): loopback, RFC1918,
    /// link-local (incl. 169.254.169.254 metadata endpoints), unspecified,
    /// and `localhost` names. Literal-IP and name-based checks only —
    /// resolver-level pinning is named future hardening (research.md D5).
    fn check_address(&self, host: &str) -> Result<(), AppError> {
        if self.policy.allow_private {
            return Ok(());
        }
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = bare.parse::<IpAddr>() {
            let non_global = match ip {
                IpAddr::V4(v4) => {
                    v4.is_loopback()
                        || v4.is_private()
                        || v4.is_link_local()
                        || v4.is_unspecified()
                        || v4.is_broadcast()
                }
                IpAddr::V6(v6) => {
                    v6.is_loopback()
                        || v6.is_unspecified()
                        // Unique-local fc00::/7 and link-local fe80::/10.
                        || (v6.segments()[0] & 0xfe00) == 0xfc00
                        || (v6.segments()[0] & 0xffc0) == 0xfe80
                }
            };
            if non_global {
                return Err(AppError::SearchProvider(format!(
                    "address {host} is not globally reachable (FETCH_ALLOW_PRIVATE is off)"
                )));
            }
        } else if bare.eq_ignore_ascii_case("localhost")
            || bare.to_lowercase().ends_with(".localhost")
        {
            return Err(AppError::SearchProvider(format!(
                "host {host} is local (FETCH_ALLOW_PRIVATE is off)"
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
    /// not least under test), fetched once per run, size-capped. `None` = the
    /// robots fetch itself failed → fail-open (treat as allowed). Callers
    /// hold the per-domain politeness lock, so the robots request is itself
    /// polite and never duplicated concurrently.
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
            Ok(response) if response.status().is_success() => self
                .read_capped(response, ROBOTS_MAX_BYTES)
                .await
                .ok()
                .map(|bytes| Arc::<str>::from(String::from_utf8_lossy(&bytes).into_owned())),
            // 404/4xx/5xx/redirected/transport: no readable robots → fail-open.
            _ => None,
        };
        self.robots
            .lock()
            .await
            .insert(origin.to_string(), body.clone());
        body
    }

    async fn read_capped(
        &self,
        response: reqwest::Response,
        cap: usize,
    ) -> Result<Vec<u8>, AppError> {
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
            if body.len() + chunk.len() > cap {
                return Err(AppError::SearchProvider(format!(
                    "body exceeds the {cap}-byte cap"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// Run every pre-request guard for one hop, then send it. Returns the
    /// response with redirects NOT followed.
    // The per-domain lock is deliberately held across robots and the request
    // — that IS the one-in-flight-per-domain politeness guarantee.
    #[allow(clippy::significant_drop_tightening)]
    async fn guarded_request(&self, target: &reqwest::Url) -> Result<reqwest::Response, AppError> {
        if !matches!(target.scheme(), "http" | "https") {
            return Err(AppError::SearchProvider(format!(
                "unsupported scheme {:?}",
                target.scheme()
            )));
        }
        let host = target
            .host_str()
            .ok_or_else(|| AppError::SearchProvider(format!("url {target} has no host")))?
            .to_string();
        self.check_domain(&host, "requested")?;
        self.check_address(&host)?;

        // Politeness: one in-flight request per domain + minimum spacing.
        {
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

            // robots.txt: fail-closed on explicit disallow, fail-open when no
            // robots could be read — checked for EVERY hop's origin.
            let origin = target.origin().ascii_serialization();
            if let Some(body) = self.robots_body(&origin).await {
                let allowed = RobotsMatcher::<LongestMatchRobotsMatchStrategy>::default()
                    .one_agent_allowed_by_robots(&body, USER_AGENT, target.as_str());
                if !allowed {
                    return Err(AppError::SearchProvider(format!(
                        "robots.txt disallows {target}"
                    )));
                }
            }

            self.http
                .get(target.clone())
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
                })
        }
    }
}

#[async_trait::async_trait]
impl Fetcher for HygieneFetcher {
    async fn fetch(&self, url: &str) -> Result<FetchedPage, AppError> {
        let mut target = reqwest::Url::parse(url)
            .map_err(|e| AppError::SearchProvider(format!("unfetchable url {url:?}: {e}")))?;

        // Manual redirect hops: every hop re-runs domain, address, robots,
        // and politeness — no internal redirect following.
        for _hop in 0..=FETCH_MAX_REDIRECTS {
            let response = self.guarded_request(&target).await?;
            let status = response.status();

            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        AppError::SearchProvider(format!(
                            "HTTP {status} without a Location header for {target}"
                        ))
                    })?;
                target = target.join(location).map_err(|e| {
                    AppError::SearchProvider(format!("unfollowable redirect {location:?}: {e}"))
                })?;
                continue;
            }
            if !status.is_success() {
                return Err(AppError::SearchProvider(format!(
                    "HTTP {status} for {target}"
                )));
            }

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

            let body = self.read_capped(response, FETCH_MAX_BYTES).await?;
            let html = String::from_utf8_lossy(&body).into_owned();
            return Ok(FetchedPage {
                url: target.to_string(),
                html,
            });
        }

        Err(AppError::SearchProvider(format!(
            "more than {FETCH_MAX_REDIRECTS} redirects for {url}"
        )))
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
            // Tests fetch a local wiremock server.
            allow_private: true,
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
    async fn redirect_target_is_robots_checked_too() {
        // The landing page is allowed; it redirects into a disallowed path.
        let mock = MockServer::start().await;
        serve_robots(&mock, "User-agent: *\nDisallow: /private\n").await;
        Mock::given(method("GET"))
            .and(path("/landing"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/private/doc"))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/private/doc"))
            .respond_with(html_response("<html>secret</html>"))
            .expect(0) // the redirect target must never be requested
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(2_000)).unwrap();
        let err = fetcher
            .fetch(&format!("{}/landing", mock.uri()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("robots.txt disallows"), "{err}");
    }

    #[tokio::test]
    async fn redirects_are_followed_with_a_cap() {
        let mock = MockServer::start().await;
        serve_robots(&mock, "").await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(ResponseTemplate::new(301).insert_header("Location", "/end"))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/end"))
            .respond_with(html_response("<html>arrived</html>"))
            .mount(&mock)
            .await;
        // An endless loop trips the cap.
        Mock::given(method("GET"))
            .and(path("/loop"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/loop"))
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(2_000)).unwrap();
        let page = fetcher
            .fetch(&format!("{}/start", mock.uri()))
            .await
            .unwrap();
        assert!(page.html.contains("arrived"));
        assert!(page.url.ends_with("/end"));

        let err = fetcher
            .fetch(&format!("{}/loop", mock.uri()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("redirects"), "{err}");
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
    async fn non_global_addresses_are_rejected_unless_allowed() {
        let fetcher = HygieneFetcher::new(FetchPolicy {
            allow_private: false,
            ..policy(2_000)
        })
        .unwrap();
        for target in [
            "http://127.0.0.1/x",
            "http://10.0.0.8/x",
            "http://192.168.1.1/x",
            "http://169.254.169.254/latest/meta-data",
            "http://localhost/x",
            "http://[::1]/x",
        ] {
            let err = fetcher.fetch(target).await.unwrap_err();
            assert!(
                err.to_string().contains("FETCH_ALLOW_PRIVATE"),
                "{target}: {err}"
            );
        }
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
    async fn oversized_robots_is_capped_and_fails_open() {
        let mock = MockServer::start().await;
        // A hostile, huge robots.txt: capped read fails → treated as
        // unreadable → fail-open, the page still fetches.
        serve_robots(&mock, &"x".repeat(ROBOTS_MAX_BYTES + 1)).await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(html_response("<html>ok</html>"))
            .mount(&mock)
            .await;

        let fetcher = HygieneFetcher::new(policy(5_000)).unwrap();
        assert!(fetcher.fetch(&format!("{}/page", mock.uri())).await.is_ok());
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
