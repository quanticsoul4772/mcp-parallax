//! Time abstraction, so time-dependent logic is deterministically testable.

use chrono::{DateTime, Utc};

/// Provides the current time. Abstracted behind a trait so logic that depends on
/// "now" can be tested against a fixed clock.
#[cfg_attr(test, mockall::automock)]
pub trait TimeProvider: Send + Sync {
    /// Return the current UTC time.
    fn now(&self) -> DateTime<Utc>;
}

/// Production [`TimeProvider`] backed by the system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl TimeProvider for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic_nondecreasing() {
        let clock = SystemClock;
        let a = clock.now();
        let b = clock.now();
        assert!(b >= a);
    }

    #[test]
    fn mock_time_provider_is_controllable() {
        let fixed = DateTime::parse_from_rfc3339("2026-06-11T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut mock = MockTimeProvider::new();
        mock.expect_now().return_const(fixed);
        assert_eq!(mock.now(), fixed);
    }
}
