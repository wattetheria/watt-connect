//! Exponential backoff for rate-limited calls: at most two retries.

use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct BackoffPolicy {
    pub max_retries: u32,
    pub base: Duration,
    pub max_delay: Duration,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        }
    }
}

impl BackoffPolicy {
    /// Delay before retry number `attempt` (0-based), honouring a
    /// server-provided `Retry-After` when it is shorter than `max_delay`.
    #[must_use]
    pub fn delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        let exponential = self.base.saturating_mul(2_u32.saturating_pow(attempt));
        retry_after.unwrap_or(exponential).min(self.max_delay)
    }
}

/// Parse a `Retry-After` header given in seconds.
#[must_use]
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_and_capped() {
        let policy = BackoffPolicy::default();
        assert_eq!(policy.delay(0, None), Duration::from_millis(500));
        assert_eq!(policy.delay(1, None), Duration::from_secs(1));
        assert_eq!(
            policy.delay(0, Some(Duration::from_secs(3))),
            Duration::from_secs(3)
        );
        assert_eq!(
            policy.delay(0, Some(Duration::from_mins(1))),
            Duration::from_secs(10)
        );
        assert_eq!(parse_retry_after(" 7 "), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
    }
}
