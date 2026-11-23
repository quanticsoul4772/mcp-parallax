//! The per-source claim-extraction model call (flat `{claims: []}`, spans
//! dropped — D4). Source text comes from the search provider (059); Parallax
//! no longer parses HTML.

use crate::error::AppError;
use crate::modes::CorrectiveMode;
use crate::research::MAX_CLAIMS_PER_SOURCE;
use crate::schema::validate;
use crate::traits::client::ModelClient;
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

/// One source's page: title (provider metadata or URL fallback) + text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadablePage {
    /// Page title.
    pub title: String,
    /// The provider's page text, capped at [`EXTRACT_TEXT_MAX_CHARS`].
    pub text: String,
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
                reported_cost_micros: None,
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
