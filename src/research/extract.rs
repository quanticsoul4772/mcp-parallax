//! Fetch+extract's second half: readable text (local, rs-trafilatura — D2)
//! and the per-source claim-extraction model call (flat `{claims: []}`,
//! spans dropped — D4).

use crate::error::AppError;
use crate::modes::CorrectiveMode;
use crate::research::MAX_CLAIMS_PER_SOURCE;
use crate::schema::validate;
use crate::traits::client::ModelClient;
use crate::traits::fetcher::FetchedPage;
use serde::Deserialize;

/// Cap on the readable text handed to the extraction call — bounds the
/// per-source token cost; pages are articles, not books, and claims
/// concentrate early.
pub const EXTRACT_TEXT_MAX_CHARS: usize = 16_000;

/// The extraction mode's prompt template (placeholders: title, text).
pub const EXTRACT_PROMPT_TEMPLATE: &str = "\
You extract falsifiable claims from one web page. A falsifiable claim is a \
single, self-contained factual statement that could in principle be proven \
wrong — not an opinion, recommendation, or vague generality. Rewrite each \
claim to stand alone without the page context. Extract only claims the page \
actually asserts. If the page asserts nothing falsifiable, return an empty \
list.\n\nPage title: <<title>>\n\nPage text:\n<<text>>";

/// One readable page: title (metadata or URL fallback) + main text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadablePage {
    /// Page title.
    pub title: String,
    /// Boilerplate-stripped main text, capped at [`EXTRACT_TEXT_MAX_CHARS`].
    pub text: String,
}

/// Reduce raw HTML to readable main text (pure, local). `None` when
/// extraction fails or yields nothing — the caller drops and counts the
/// source (FR-013).
#[must_use]
pub fn readable_text(page: &FetchedPage) -> Option<ReadablePage> {
    let extracted = rs_trafilatura::extract(&page.html).ok()?;
    let text = extracted.content_text;
    if text.trim().is_empty() {
        return None;
    }
    let title = extracted
        .metadata
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| page.url.clone());
    let text = if text.chars().count() > EXTRACT_TEXT_MAX_CHARS {
        text.chars().take(EXTRACT_TEXT_MAX_CHARS).collect()
    } else {
        text
    };
    Some(ReadablePage { title, text })
}

/// The extraction call's constrained output (flat + closed).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExtractOut {
    /// Falsifiable claims, each self-contained.
    pub claims: Vec<String>,
}

/// Extract falsifiable claims from one readable page: one model call,
/// constrained to `{claims: [string]}`, capped at
/// [`MAX_CLAIMS_PER_SOURCE`], blank entries dropped.
///
/// # Errors
///
/// Provider/validation classes from the model call — callers treat any error
/// as drop-and-count for this source (FR-013).
pub async fn extract_claims(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    page: &ReadablePage,
) -> Result<(Vec<String>, u64, u64), AppError> {
    let prompt = mode
        .prompt_template
        .replace("<<title>>", &page.title)
        .replace("<<text>>", &page.text);

    let completion = client.complete(&prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let out: ExtractOut = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("extraction shape: {e}")))?;

    let claims: Vec<String> = out
        .claims
        .into_iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .take(MAX_CLAIMS_PER_SOURCE)
        .collect();

    Ok((claims, completion.input_tokens, completion.output_tokens))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::modes::ModeRegistry;
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::json;

    fn extract_mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        crate::research::pipeline::register(&mut registry).unwrap();
        registry
            .get(crate::research::pipeline::EXTRACT_MODE_ID)
            .unwrap()
            .clone()
    }

    #[test]
    fn fixture_article_reduces_to_main_text_with_title() {
        let page = FetchedPage {
            url: "https://example.com/post".into(),
            html: include_str!("../../tests/fixtures/article.html").into(),
        };
        let readable = readable_text(&page).unwrap();
        assert!(readable.text.contains("single-digit milliseconds"));
        assert!(!readable.text.contains("Subscribe to our newsletter"));
        assert!(readable.title.contains("Brute-Force"));
    }

    #[test]
    fn empty_or_unextractable_html_is_none_not_an_error() {
        let page = FetchedPage {
            url: "https://example.com/empty".into(),
            html: "<html><body></body></html>".into(),
        };
        assert!(readable_text(&page).is_none());
    }

    #[test]
    fn oversized_text_is_capped() {
        let body = "word ".repeat(20_000);
        let page = FetchedPage {
            url: "https://example.com/long".into(),
            html: format!("<html><body><article><h1>T</h1><p>{body}</p></article></body></html>"),
        };
        let readable = readable_text(&page).unwrap();
        assert!(readable.text.chars().count() <= EXTRACT_TEXT_MAX_CHARS);
    }

    #[tokio::test]
    async fn claims_are_trimmed_filtered_and_capped() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            assert!(prompt.contains("Page title: T"));
            let many: Vec<String> = (0..20).map(|i| format!("claim {i}")).collect();
            let mut claims = vec![" padded ".to_string(), "  ".to_string()];
            claims.extend(many);
            Ok(Completion {
                value: json!({ "claims": claims }),
                input_tokens: 50,
                output_tokens: 20,
            })
        });

        let page = ReadablePage {
            title: "T".into(),
            text: "body".into(),
        };
        let (claims, inp, out) = extract_claims(&client, &extract_mode(), &page)
            .await
            .unwrap();
        assert_eq!(claims.len(), MAX_CLAIMS_PER_SOURCE);
        assert_eq!(claims[0], "padded");
        assert!(!claims.iter().any(String::is_empty));
        assert_eq!((inp, out), (50, 20));
    }
}
