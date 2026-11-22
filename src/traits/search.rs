//! The search-provider boundary (research capability).
//!
//! Providers sit behind the seam the design doc calls for
//! (RESEARCH_PRIMITIVE.md §11). 059: a provider both finds sources
//! (`search`) and returns their text (`contents`), so research downloads no
//! page itself.

use crate::error::AppError;

/// The `Timeout` label for a call the run's deadline cut off (059 review).
pub const DEADLINE: &str = "research deadline";

/// The `Timeout` label for a call that sent nothing because no time was left:
/// it cost nothing, so it does not make the run's cost unknown.
pub const NOT_SENT: &str = "research deadline, nothing sent";

/// The provider's search mode (059 research.md D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    /// The provider's default mode.
    Auto,
    /// A lighter multi-step search, for deep research.
    DeepLite,
}

impl SearchType {
    /// The wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::DeepLite => "deep-lite",
        }
    }
}

/// One search query with the run's options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    /// The query text (one scoped angle).
    pub query: String,
    /// Maximum hits wanted.
    pub count: u8,
    /// Search mode, from the research depth.
    pub search_type: SearchType,
    /// Only these domains, when non-empty.
    pub include_domains: Vec<String>,
    /// Never these domains.
    pub exclude_domains: Vec<String>,
    /// Time left before the run's deadline; bounds the call when set
    /// (059 review finding 1).
    pub time_left: Option<std::time::Duration>,
}

/// One search hit: identity only — text comes from [`SearchProvider::contents`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Result URL as returned by the provider.
    pub url: String,
    /// Result title.
    pub title: String,
}

/// A search's hits and the request's reported cost.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResults {
    /// Hits in provider order.
    pub hits: Vec<SearchHit>,
    /// Cost in USD as reported by the provider; `None` when not reported.
    pub cost_usd: Option<f64>,
}

/// One contents request for the run's kept sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsRequest {
    /// The kept source URLs.
    pub urls: Vec<String>,
    /// Freshness limit in hours (`EXA_MAX_AGE_HOURS`).
    pub max_age_hours: i32,
    /// Per-page text cap in characters.
    pub max_characters: usize,
    /// Time left before the run's deadline; bounds the call when set
    /// (059 review finding 1).
    pub time_left: Option<std::time::Duration>,
}

/// One page's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The address that was requested for this page — what research pairs
    /// it with its source by, even when the provider followed a redirect.
    pub requested_url: String,
    /// The page's own address as the provider returned it.
    pub url: String,
    /// The page title (the URL when the provider gives none).
    pub title: String,
    /// The page text, possibly empty.
    pub text: String,
    /// The text reached the character cap ([`is_truncated`]).
    pub truncated: bool,
}

/// The pages a contents request returned and its reported cost. A URL the
/// provider could not read is absent.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentsResults {
    /// Pages in provider order.
    pub pages: Vec<Page>,
    /// Cost in USD as reported by the provider; `None` when not reported.
    pub cost_usd: Option<f64>,
}

/// Whether text of `chars` characters was cut at `max_characters`: the
/// provider does not report truncation, so text at or above 99% of the cap
/// counts as cut (059 research.md D4).
#[must_use]
pub const fn is_truncated(chars: usize, max_characters: usize) -> bool {
    chars > 0 && chars.saturating_mul(100) >= max_characters.saturating_mul(99)
}

/// A web-search backend that also returns page text.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait SearchProvider: Send + Sync {
    /// Run one search query.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] classified per the outcome taxonomy
    /// (`SearchProvider`, `Timeout`, `RetriesExhausted`).
    async fn search(&self, request: &SearchRequest) -> Result<SearchResults, AppError>;

    /// Fetch the text of the given pages.
    ///
    /// # Errors
    ///
    /// As for [`SearchProvider::search`].
    async fn contents(&self, request: &ContentsRequest) -> Result<ContentsResults, AppError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn request(count: u8) -> SearchRequest {
        SearchRequest {
            query: "q".into(),
            count,
            search_type: SearchType::Auto,
            include_domains: vec!["example.com".into()],
            exclude_domains: vec![],
            time_left: None,
        }
    }

    /// 059 T001: `search` takes the per-run request (type and domain lists)
    /// and returns hits plus the request's reported cost.
    #[tokio::test]
    async fn mock_search_provider_honors_the_contract() {
        let mut mock = MockSearchProvider::new();
        mock.expect_search().returning(|req| {
            assert_eq!(req.query, "q");
            assert_eq!(req.search_type, SearchType::Auto);
            assert_eq!(req.include_domains, vec!["example.com".to_string()]);
            Ok(SearchResults {
                hits: (0..req.count)
                    .map(|i| SearchHit {
                        url: format!("https://example.com/{i}"),
                        title: format!("hit {i}"),
                    })
                    .collect(),
                cost_usd: Some(0.007),
            })
        });

        let results = mock.search(&request(3)).await.unwrap();
        assert_eq!(results.hits.len(), 3);
        assert_eq!(results.hits[0].url, "https://example.com/0");
        assert_eq!(results.cost_usd, Some(0.007));
    }

    /// 059 T001: `contents` takes the kept URLs with the freshness limit and
    /// character cap, and returns each page's text, its truncated flag, and
    /// the request's cost (absent when the provider reports none).
    #[tokio::test]
    async fn mock_contents_honors_the_contract() {
        let mut mock = MockSearchProvider::new();
        mock.expect_contents().returning(|req| {
            assert_eq!(req.urls, vec!["https://example.com/a".to_string()]);
            assert_eq!(req.max_age_hours, 168);
            assert_eq!(req.max_characters, 16_000);
            Ok(ContentsResults {
                pages: vec![Page {
                    requested_url: req.urls[0].clone(),
                    url: req.urls[0].clone(),
                    title: "A".into(),
                    text: "body".into(),
                    truncated: false,
                }],
                cost_usd: None,
            })
        });

        let results = mock
            .contents(&ContentsRequest {
                urls: vec!["https://example.com/a".into()],
                max_age_hours: 168,
                max_characters: 16_000,
                time_left: None,
            })
            .await
            .unwrap();
        assert_eq!(results.pages[0].text, "body");
        assert!(!results.pages[0].truncated);
        assert_eq!(results.cost_usd, None);
    }

    /// 059 research.md D4: Exa does not report truncation, so text at or
    /// above 99% of the cap is treated as cut (the reference's rule).
    #[test]
    fn text_at_ninety_nine_percent_of_the_cap_is_truncated() {
        assert!(is_truncated(16_000, 16_000));
        assert!(is_truncated(15_840, 16_000));
        assert!(!is_truncated(15_839, 16_000));
        assert!(!is_truncated(0, 16_000));
    }
}
