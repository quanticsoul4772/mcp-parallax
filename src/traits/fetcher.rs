//! The page-fetch boundary (research capability).
//!
//! The implementation owns all fetch hygiene (timeouts, size caps,
//! content-type guards, robots.txt, per-domain politeness — research.md 004
//! D5); the seam returns readable input or a classified error, so the
//! pipeline never touches HTTP.

use crate::error::AppError;

/// One fetched page, reduced to what extraction needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    /// Final URL after redirects.
    pub url: String,
    /// Raw HTML (bounded by the fetcher's size cap).
    pub html: String,
}

/// A page-fetch backend with hygiene enforced inside the implementation.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait Fetcher: Send + Sync {
    /// Fetch one URL.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::SearchProvider`]-class errors for hygiene
    /// rejections (denied domain, robots disallow, size/content-type) and
    /// `Timeout` for elapsed budgets. Callers treat any error as
    /// drop-and-count (FR-013) — one bad URL never fails a run.
    async fn fetch(&self, url: &str) -> Result<FetchedPage, AppError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_fetcher_honors_the_contract() {
        let mut mock = MockFetcher::new();
        mock.expect_fetch().returning(|url| {
            Ok(FetchedPage {
                url: url.to_string(),
                html: "<html><body>hi</body></html>".to_string(),
            })
        });

        let page = mock.fetch("https://example.com/a").await.unwrap();
        assert_eq!(page.url, "https://example.com/a");
        assert!(page.html.contains("hi"));
    }
}
