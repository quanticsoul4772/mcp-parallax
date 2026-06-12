//! The search-provider boundary (research capability).
//!
//! One provider in v1 (Brave) behind the seam the design doc itself calls
//! for (RESEARCH_PRIMITIVE.md §11) — providers are swappable and differ in
//! rate/cost/latency.

use crate::error::AppError;

/// One search hit: identity and snippet only — fetching is the
/// [`crate::traits::fetcher::Fetcher`]'s job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Result URL as returned by the provider.
    pub url: String,
    /// Result title.
    pub title: String,
    /// Provider snippet/description (may be empty).
    pub snippet: String,
}

/// A web-search backend.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait SearchProvider: Send + Sync {
    /// Run one search query, returning up to `count` hits.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] classified per the outcome taxonomy
    /// (`SearchProvider`, `Timeout`, `RetriesExhausted`).
    async fn search(&self, query: &str, count: u8) -> Result<Vec<SearchHit>, AppError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_search_provider_honors_the_contract() {
        let mut mock = MockSearchProvider::new();
        mock.expect_search().returning(|query, count| {
            assert_eq!(query, "q");
            Ok((0..count)
                .map(|i| SearchHit {
                    url: format!("https://example.com/{i}"),
                    title: format!("hit {i}"),
                    snippet: String::new(),
                })
                .collect())
        });

        let hits = mock.search("q", 3).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].url, "https://example.com/0");
    }
}
