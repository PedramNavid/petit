//! Retry policy configuration for tasks.
//!
//! Supports fixed delay retry with configurable max attempts.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Retry policy for a task.
///
/// Defines how a task should be retried on failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts, not including the initial attempt (0 = no retries).
    /// For example, `max_attempts = 3` means up to 3 retries after the initial failure,
    /// for a total of 4 attempts (1 initial + 3 retries).
    pub max_attempts: u32,

    /// Fixed delay between retry attempts.
    #[serde(with = "serde_duration")]
    pub delay: Duration,

    /// Condition for when to retry.
    pub retry_on: RetryCondition,
}

/// Conditions under which a task should be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryCondition {
    /// Retry on any error.
    Always,

    /// Retry only on transient errors (timeouts, resource unavailable).
    TransientOnly,

    /// Never retry, regardless of max_attempts.
    Never,
}

impl RetryPolicy {
    /// Create a policy with no retries.
    pub fn none() -> Self {
        Self {
            max_attempts: 0,
            delay: Duration::ZERO,
            retry_on: RetryCondition::Never,
        }
    }

    /// Create a policy with fixed delay retries.
    ///
    /// # Arguments
    /// * `max_attempts` - Maximum retry attempts (not including initial try)
    /// * `delay` - Fixed delay between retries
    pub fn fixed(max_attempts: u32, delay: Duration) -> Self {
        Self {
            max_attempts,
            delay,
            retry_on: RetryCondition::Always,
        }
    }

    /// Builder: set the retry condition.
    pub fn with_condition(mut self, condition: RetryCondition) -> Self {
        self.retry_on = condition;
        self
    }

    /// Check if retries are enabled.
    pub fn is_enabled(&self) -> bool {
        self.max_attempts > 0 && self.retry_on != RetryCondition::Never
    }

    /// Check if we should retry given the current attempt count.
    ///
    /// # Arguments
    /// * `attempts` - Number of attempts already made (including failed ones)
    pub fn should_retry(&self, attempts: u32) -> bool {
        if self.retry_on == RetryCondition::Never {
            return false;
        }
        attempts < self.max_attempts
    }

    /// Get the delay before the next retry.
    pub fn get_delay(&self) -> Duration {
        self.delay
    }
}

impl Default for RetryPolicy {
    /// Default policy: no retries.
    fn default() -> Self {
        Self::none()
    }
}

impl Default for RetryCondition {
    fn default() -> Self {
        RetryCondition::Always
    }
}

/// Serde helper for Duration serialization.
///
/// Serializes Duration as seconds (matching YAML config format).
mod serde_duration {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy_has_no_retries() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.max_attempts, 0);
        assert!(!policy.is_enabled());
    }

    #[test]
    fn test_none_policy() {
        let policy = RetryPolicy::none();

        assert_eq!(policy.max_attempts, 0);
        assert!(!policy.should_retry(0));
        assert!(!policy.should_retry(1));
    }

    #[test]
    fn test_fixed_delay_policy() {
        let policy = RetryPolicy::fixed(3, Duration::from_secs(5));

        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.delay, Duration::from_secs(5));
        assert!(policy.is_enabled());
    }

    #[test]
    fn test_should_retry_respects_max_attempts() {
        let policy = RetryPolicy::fixed(3, Duration::from_secs(1));

        // Initial attempt failed (attempts=1), should allow first retry
        assert!(policy.should_retry(1));

        // First retry failed (attempts=2), should allow second retry
        assert!(policy.should_retry(2));

        // Second retry failed (attempts=3), should NOT retry (reached max_attempts)
        assert!(!policy.should_retry(3));

        // Third attempt would exceed max_attempts
        assert!(!policy.should_retry(4));
    }

    #[test]
    fn test_retry_condition_never() {
        let policy =
            RetryPolicy::fixed(3, Duration::from_secs(1)).with_condition(RetryCondition::Never);

        // Even with max_attempts > 0, Never condition disables retries
        assert!(!policy.should_retry(1));
    }

    #[test]
    fn test_retry_condition_transient_only() {
        let policy = RetryPolicy::fixed(3, Duration::from_secs(1))
            .with_condition(RetryCondition::TransientOnly);

        // TransientOnly still allows retries (actual filtering happens at execution)
        assert!(policy.should_retry(1));
    }

    #[test]
    fn test_get_delay() {
        let delay = Duration::from_millis(500);
        let policy = RetryPolicy::fixed(2, delay);

        assert_eq!(policy.get_delay(), delay);
    }

    #[test]
    fn test_policy_serialization() {
        let policy = RetryPolicy::fixed(3, Duration::from_secs(10));
        let json = serde_json::to_string(&policy).expect("serialize");
        let deserialized: RetryPolicy = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(policy, deserialized);
    }
}
