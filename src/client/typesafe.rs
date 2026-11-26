//! The TypeSafe (Jev) client (064).
//!
//! `POST /v1/systemone` with bearer auth, behind [`Classifier`]. A request
//! retries 429, 529 and 5xx within its deadline only when it sets `retry`
//! (065: `on` sites); shadow requests make one attempt. At most
//! [`MAX_IN_FLIGHT`] requests run at once, and a 401 turns Jev off for the
//! life of the client. Error bodies are never
//! kept or logged: an error holds the status, a known error code and the
//! request id only.

use crate::config::JevSettings;
use crate::error::{AppError, ConfigError};
use crate::jev::redact::RedactedState;
use crate::traits::classifier::{
    Classifier, JevAnswer, JevQuestion, JevRequest, JevResponse, QuestionKind,
};
use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::Instant;

/// TypeSafe's API base.
pub const TYPESAFE_API_BASE: &str = "https://api.typesafe.ai";

/// Deadline for a turn-review request.
pub const JEV_TURN_DEADLINE_MS: u64 = 1000;

/// Deadline for a research-claim request, capped by the run's time left.
pub const JEV_CLAIM_DEADLINE_MS: u64 = 2000;

/// Requests in flight at once. The server builds one client, so this holds
/// per process.
pub const MAX_IN_FLIGHT: usize = 4;

/// Error codes named in an error, when the body contains them. Nothing else
/// from the body is kept.
const KNOWN_CODES: [&str; 1] = ["max_tokens_exceeded"];

const REQUEST_ID_HEADER: &str = "x-typesafe-request-id";

/// Waits between retries when the response gives no `Retry-After` (065
/// research D5); after the last one the error is returned.
const BACKOFF_MS: [u64; 3] = [100, 200, 400];

/// A failed attempt: the error, and what a retry needs to know.
struct Failed {
    error: AppError,
    /// 429, 529 or 5xx.
    retryable: bool,
    /// The response's `Retry-After`, when it had one.
    retry_after: Option<Duration>,
}

impl From<AppError> for Failed {
    fn from(error: AppError) -> Self {
        Self {
            error,
            retryable: false,
            retry_after: None,
        }
    }
}

/// The TypeSafe client. Its `Debug` hides the key (064 review P).
pub struct TypesafeClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
    endpoint: String,
    permits: Semaphore,
    disabled: AtomicBool,
}

impl std::fmt::Debug for TypesafeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypesafeClient")
            .field("api_key", &"[hidden]")
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl TypesafeClient {
    /// The production client, from the Jev settings.
    ///
    /// # Errors
    ///
    /// [`AppError::Config`] when no key is set, or when the HTTP client
    /// cannot be built.
    pub fn new(settings: &JevSettings) -> Result<Self, AppError> {
        let key = settings.api_key.as_deref().ok_or_else(|| {
            AppError::Config(ConfigError::Jev("TYPESAFE_API_KEY is not set".into()))
        })?;
        Self::with_base_url(key, &settings.model, TYPESAFE_API_BASE)
    }

    /// A client against `base_url` (tests point this at a mock server).
    ///
    /// # Errors
    ///
    /// [`AppError::Config`] when the HTTP client cannot be built.
    pub fn with_base_url(api_key: &str, model: &str, base_url: &str) -> Result<Self, AppError> {
        let http = reqwest::Client::builder().build().map_err(|e| {
            AppError::Config(ConfigError::Jev(format!("TypeSafe HTTP client: {e}")))
        })?;
        Ok(Self {
            http,
            api_key: api_key.to_string(),
            model: model.to_string(),
            endpoint: format!("{}/v1/systemone", base_url.trim_end_matches('/')),
            permits: Semaphore::new(MAX_IN_FLIGHT),
            disabled: AtomicBool::new(false),
        })
    }

    async fn send(
        &self,
        body: String,
        deadline: Instant,
        timeout: Duration,
    ) -> Result<JevResponse, Failed> {
        let started = Instant::now();
        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .timeout(deadline.saturating_duration_since(started))
            .body(body)
            .send()
            .await
            .map_err(|e| Failed::from(transport_error(&e, timeout)))?;
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let retry_after = super::retry::retry_after(
            response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            chrono::Utc::now(),
        );
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Failed::from(transport_error(&e, timeout)))?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if (200..300).contains(&status) {
            return parse_response(&bytes, request_id, latency_ms).map_err(Failed::from);
        }
        Err(Failed {
            error: self.status_error(status, &bytes, request_id.as_deref()),
            retryable: status == 429 || status >= 500,
            retry_after,
        })
    }

    fn status_error(&self, status: u16, body: &[u8], request_id: Option<&str>) -> AppError {
        let code = KNOWN_CODES
            .iter()
            .find(|c| body.windows(c.len()).any(|w| w == c.as_bytes()))
            .map(|c| format!(" {c}"))
            .unwrap_or_default();
        let id = request_id
            .map(|id| format!(" (request id {id})"))
            .unwrap_or_default();
        let detail = format!("typesafe {status}{code}{id}");
        if status == 401 && !self.disabled.swap(true, Ordering::SeqCst) {
            tracing::warn!(
                "TypeSafe rejected the key (401); Jev is off for the rest of this process"
            );
        }
        if status == 429 || !(400..500).contains(&status) {
            AppError::ClassifierProvider(detail)
        } else {
            AppError::ProviderRejected { status, detail }
        }
    }
}

#[async_trait::async_trait]
impl Classifier for TypesafeClient {
    async fn classify(&self, request: &JevRequest) -> Result<JevResponse, AppError> {
        if self.disabled.load(Ordering::SeqCst) {
            return Err(AppError::ProviderRejected {
                status: 401,
                detail: "typesafe disabled after an earlier 401".into(),
            });
        }
        let timeout = request.timeout;
        if timeout.is_zero() {
            return Err(timed_out(timeout));
        }
        let deadline = Instant::now() + timeout;
        let body = serde_json::to_string(&Body(request))
            .map_err(|e| AppError::ValidationFailure(format!("typesafe request body: {e}")))?;
        // Timing out here sends nothing, so it is named apart from a request
        // cut off mid-flight (064 review K).
        let _permit = tokio::time::timeout_at(deadline, self.permits.acquire())
            .await
            .map_err(|_| AppError::Timeout {
                what: "jev request slot",
                ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            })?
            .map_err(|_| AppError::ClassifierProvider("typesafe client closed".into()))?;
        let mut backoff = BACKOFF_MS.iter();
        loop {
            let failed =
                match tokio::time::timeout_at(deadline, self.send(body.clone(), deadline, timeout))
                    .await
                {
                    Err(_) => return Err(timed_out(timeout)),
                    Ok(Ok(response)) => return Ok(response),
                    Ok(Err(failed)) => failed,
                };
            if !(request.retry && failed.retryable) {
                return Err(failed.error);
            }
            // Every retry uses a backoff step, so there are at most
            // `BACKOFF_MS.len()` retries; `Retry-After` can lengthen a wait,
            // never shorten it below the step.
            let Some(step) = backoff.next().map(|ms| Duration::from_millis(*ms)) else {
                return Err(failed.error);
            };
            let wait = failed.retry_after.map_or(step, |asked| asked.max(step));
            // Compared with the time left, never added to an `Instant`: a huge
            // `Retry-After` would overflow it.
            if wait >= deadline.saturating_duration_since(Instant::now()) {
                return Err(failed.error);
            }
            tokio::time::sleep(wait).await;
        }
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

fn timed_out(timeout: Duration) -> AppError {
    AppError::Timeout {
        what: "jev request",
        ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
    }
}

fn transport_error(error: &reqwest::Error, timeout: Duration) -> AppError {
    if error.is_timeout() {
        timed_out(timeout)
    } else if error.is_connect() {
        AppError::ClassifierProvider("typesafe connection failed".into())
    } else {
        AppError::ClassifierProvider("typesafe transport error".into())
    }
}

fn unreadable(request_id: Option<&str>, what: &str) -> AppError {
    let id = request_id
        .map(|id| format!(" (request id {id})"))
        .unwrap_or_default();
    AppError::ValidationFailure(format!("typesafe response {what}{id}"))
}

fn parse_response(
    bytes: &[u8],
    request_id: Option<String>,
    latency_ms: u64,
) -> Result<JevResponse, AppError> {
    let id = request_id.as_deref();
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| unreadable(id, "is not JSON"))?;
    let model_answered = value["model"]
        .as_str()
        .ok_or_else(|| unreadable(id, "has no model"))?
        .to_string();
    let answers_in = value["answers"]
        .as_object()
        .ok_or_else(|| unreadable(id, "has no answers"))?;
    let mut answers = Vec::with_capacity(answers_in.len());
    for (key, answer) in answers_in {
        let parsed = match answer["type"].as_str() {
            Some("noul") => JevAnswer::Noul {
                p: answer["noul"]
                    .as_f64()
                    .ok_or_else(|| unreadable(id, "has a noul answer without a number"))?,
            },
            Some("choice") => JevAnswer::Choice {
                choice: answer["choice"]
                    .as_str()
                    .ok_or_else(|| unreadable(id, "has a choice answer without a choice"))?
                    .to_string(),
                probabilities: answer["probabilities"]
                    .as_object()
                    .ok_or_else(|| unreadable(id, "has a choice answer without probabilities"))?
                    .iter()
                    .map(|(option, p)| {
                        p.as_f64()
                            .map(|p| (option.clone(), p))
                            .ok_or_else(|| unreadable(id, "has a probability that is not a number"))
                    })
                    .collect::<Result<_, _>>()?,
            },
            _ => return Err(unreadable(id, "has an answer of unknown type")),
        };
        answers.push((key.clone(), parsed));
    }
    Ok(JevResponse {
        model_answered,
        answers,
        input_tokens: value["usage"]["input_tokens"].as_u64(),
        request_id,
        latency_ms,
    })
}

/// The request body, written in declared order: questions, and each choice
/// question's options, keep the order they were declared in.
struct Body<'a>(&'a JevRequest);

impl Serialize for Body<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("model", &self.0.model)?;
        map.serialize_entry("state", &State(&self.0.state))?;
        map.serialize_entry("questions", &Questions(&self.0.questions))?;
        map.end()
    }
}

struct State<'a>(&'a RedactedState);

impl Serialize for State<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0.as_json() {
            Some(tree) => tree.serialize(serializer),
            None => serializer.serialize_str(self.0.as_str()),
        }
    }
}

struct Questions<'a>(&'a [JevQuestion]);

impl Serialize for Questions<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for question in self.0 {
            map.serialize_entry(&question.id, &Question(question))?;
        }
        map.end()
    }
}

struct Question<'a>(&'a JevQuestion);

impl Serialize for Question<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let q = self.0;
        match q.kind {
            // A yes/no question sends criteria only when it declares them, so
            // a question with none is sent exactly as before.
            QuestionKind::Noul => {
                let with_criteria = !q.options.is_empty();
                let mut map = serializer.serialize_map(Some(2 + usize::from(with_criteria)))?;
                map.serialize_entry("type", "noul")?;
                map.serialize_entry("instructions", &q.instructions)?;
                if with_criteria {
                    map.serialize_entry("criteria", &Criteria(&q.options))?;
                }
                map.end()
            }
            QuestionKind::Choice => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("type", "choice")?;
                map.serialize_entry("instructions", &q.instructions)?;
                map.serialize_entry("criteria", &Criteria(&q.options))?;
                map.end()
            }
        }
    }
}

struct Criteria<'a>(&'a [(String, Option<String>)]);

impl Serialize for Criteria<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (option, description) in self.0 {
            map.serialize_entry(option, description)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests;
