//! The Jev classifier seam (064).
//!
//! TypeSafe's typed-answer model behind a mockable boundary: the site and
//! mode vocabularies, the request, question and answer types, and the
//! [`Classifier`] trait.

use crate::error::AppError;
use crate::jev::redact::RedactedState;
use std::time::Duration;

/// Where Parallax asks Jev (064). Each site has its own mode variable,
/// `PARALLAX_JEV_MODE_<SUFFIX>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JevSite {
    /// The end-of-turn watchdog review.
    TurnReview,
    /// Research claim verification.
    ResearchClaims,
    /// The injection screen on kept research pages (065).
    ResearchInjection,
}

impl JevSite {
    /// Every site, in declaration order.
    pub const ALL: [Self; 3] = [
        Self::TurnReview,
        Self::ResearchClaims,
        Self::ResearchInjection,
    ];

    /// The stable name recorded in `jev_records.site`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TurnReview => "turn_review",
            Self::ResearchClaims => "research_claims",
            Self::ResearchInjection => "research_injection",
        }
    }

    /// The suffix of the site's mode variable.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::TurnReview => "TURN_REVIEW",
            Self::ResearchClaims => "RESEARCH_CLAIMS",
            Self::ResearchInjection => "RESEARCH_INJECTION",
        }
    }
}

/// How a site uses Jev (064). `On` lets an answer change what Parallax
/// returns; config accepts it only for a site with a pinned threshold (065).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum JevMode {
    /// No Jev call.
    #[default]
    Off,
    /// Jev answers beside the big model; recorded only, nothing changes.
    Shadow,
    /// Jev's answer is used, at a pinned threshold (065).
    On,
}

impl JevMode {
    /// Every mode, in declaration order.
    pub const ALL: [Self; 3] = [Self::Off, Self::Shadow, Self::On];

    /// The stable name used in configuration and records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Shadow => "shadow",
            Self::On => "on",
        }
    }

    /// Parse a configuration value; `None` for anything not in [`Self::ALL`].
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value.trim())
    }
}

/// The kind of answer a Jev question asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    /// Yes/no: the answer is P(yes).
    Noul,
    /// Pick one of the declared options.
    Choice,
}

/// One versioned Jev question. Options keep their declared order, because
/// Jev leans toward the first option and the order is chosen on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JevQuestion {
    /// Answer key; also the stable question name in records.
    pub id: String,
    /// Wording version; bumped on every wording change.
    pub version: u32,
    /// Yes/no or choice.
    pub kind: QuestionKind,
    /// The question text, pointing at parts of the state by backticked path.
    pub instructions: String,
    /// Choice options in order, each with an optional description; empty for
    /// yes/no questions.
    pub options: Vec<(String, Option<String>)>,
}

/// One answer, as returned by Jev.
#[derive(Debug, Clone, PartialEq)]
pub enum JevAnswer {
    /// P(yes) for a yes/no question.
    Noul {
        /// The probability of yes.
        p: f64,
    },
    /// The top option of a choice question and every option's probability.
    Choice {
        /// The option Jev picked.
        choice: String,
        /// Every option's probability, as returned.
        probabilities: Vec<(String, f64)>,
    },
}

/// A Jev response: the answers keyed by question id, plus what the records
/// need.
#[derive(Debug, Clone, PartialEq)]
pub struct JevResponse {
    /// The versioned model that answered, read from the response.
    pub model_answered: String,
    /// Answers in the order returned, keyed by question id.
    pub answers: Vec<(String, JevAnswer)>,
    /// Billed input tokens, when reported.
    pub input_tokens: Option<u64>,
    /// `x-typesafe-request-id`, when present.
    pub request_id: Option<String>,
    /// Request wall time.
    pub latency_ms: u64,
}

/// One Jev request: every question of one site about one redacted state.
#[derive(Debug, Clone)]
pub struct JevRequest {
    /// The pinned model to ask.
    pub model: String,
    /// The state, already through the redactor; nothing else is accepted.
    pub state: RedactedState,
    /// The questions, in order.
    pub questions: Vec<JevQuestion>,
    /// Time left for this request, body included. Zero means nothing is sent.
    pub timeout: Duration,
}

/// A Jev backend.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait Classifier: Send + Sync {
    /// Ask every question in `request` in one call.
    ///
    /// # Errors
    ///
    /// Classified per research D4: `ProviderRejected` for a 4xx other than
    /// 429, `Timeout` at the deadline, `ClassifierProvider` for 429, 529,
    /// 5xx and connection failures.
    async fn classify(&self, request: &JevRequest) -> Result<JevResponse, AppError>;

    /// The pinned model id.
    fn model_id(&self) -> &str;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::jev::redact::Redactor;

    fn request() -> JevRequest {
        JevRequest {
            model: "jev-1.14.0".into(),
            state: Redactor::new(vec![]).redact("state").unwrap(),
            questions: crate::jev::questions::turn_review_questions(),
            timeout: Duration::from_millis(1000),
        }
    }

    #[tokio::test]
    async fn mock_classifier_answers_and_reports_its_model() {
        let mut mock = MockClassifier::new();
        mock.expect_classify().returning(|_| {
            Ok(JevResponse {
                model_answered: "jev-1.14.0".into(),
                answers: vec![("contradiction".into(), JevAnswer::Noul { p: 0.2 })],
                input_tokens: Some(40),
                request_id: Some("req-1".into()),
                latency_ms: 100,
            })
        });
        mock.expect_model_id()
            .return_const("jev-1.14.0".to_string());
        let response = mock.classify(&request()).await.unwrap();
        assert_eq!(response.request_id.as_deref(), Some("req-1"));
        assert_eq!(mock.model_id(), "jev-1.14.0");
    }

    #[test]
    fn the_injection_site_and_the_on_mode_are_listed() {
        assert!(JevSite::ALL.contains(&JevSite::ResearchInjection));
        assert_eq!(JevSite::ResearchInjection.as_str(), "research_injection");
        assert_eq!(JevSite::ResearchInjection.suffix(), "RESEARCH_INJECTION");
        assert!(JevMode::ALL.contains(&JevMode::On));
        assert_eq!(JevMode::parse("on"), Some(JevMode::On));
        assert_eq!(JevMode::On.as_str(), "on");
    }

    #[tokio::test]
    async fn mock_classifier_errors_come_back_as_classifier_provider() {
        let mut mock = MockClassifier::new();
        mock.expect_classify()
            .returning(|_| Err(AppError::ClassifierProvider("typesafe 529".into())));
        let err = mock.classify(&request()).await.unwrap_err();
        assert_eq!(err.outcome(), crate::error::Outcome::ClassifierProvider);
    }
}
