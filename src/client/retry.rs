//! Retry helpers shared by the HTTP clients (moved from `exa.rs` in 064 so
//! later clients reuse one copy rather than a third).

use std::time::Duration;

/// The wait a `Retry-After` header asks for: whole seconds or an HTTP date
/// (RFC 9110); a date already past is no wait. `None` when absent or
/// unreadable, so the caller falls back to its own backoff.
pub fn retry_after(value: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    let value = value?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let wait = at.with_timezone(&chrono::Utc) - now;
    Some(wait.to_std().unwrap_or(Duration::ZERO))
}

/// Time left until `deadline`.
pub fn time_until(deadline: std::time::Instant) -> Duration {
    deadline.saturating_duration_since(std::time::Instant::now())
}

/// Most of a rejection body kept in the error (059 review finding 7).
pub const DETAIL_MAX_CHARS: usize = 2_000;

/// A rejection body cut to [`DETAIL_MAX_CHARS`], marking the cut.
pub fn cap_detail(body: &str) -> String {
    if body.chars().count() <= DETAIL_MAX_CHARS {
        return body.to_string();
    }
    let mut kept: String = body.chars().take(DETAIL_MAX_CHARS).collect();
    kept.push_str("… (truncated)");
    kept
}
