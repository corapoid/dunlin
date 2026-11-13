//! Reconnect policy for [`super::HostRuntime`](super::runtime::HostRuntime).

use std::time::Duration;

/// Backoff schedule between reconnect attempts.
#[derive(Debug, Clone, PartialEq)]
pub enum Backoff {
    /// Retry immediately, with no delay between attempts.
    Immediate,
    /// Wait a fixed amount of time between attempts.
    Constant {
        /// Per-attempt delay.
        delay: Duration,
    },
    /// Exponential backoff: `delay = min(initial * multiplier^(attempt-1), max)`.
    Exponential {
        /// First attempt's delay.
        initial: Duration,
        /// Cap on the per-attempt delay.
        max: Duration,
        /// Multiplier between attempts. Must be >= 1.0.
        multiplier: f64,
    },
}

impl Backoff {
    /// Compute the delay before the `attempt`-th retry (1-based).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        match self {
            Backoff::Immediate => Duration::ZERO,
            Backoff::Constant { delay } => *delay,
            Backoff::Exponential {
                initial,
                max,
                multiplier,
            } => {
                let attempt = attempt.max(1);
                let exp = (attempt - 1) as i32;
                let scaled = (initial.as_secs_f64()) * multiplier.max(1.0).powi(exp);
                let bounded = scaled.min(max.as_secs_f64());
                Duration::from_secs_f64(bounded)
            }
        }
    }
}

/// Reconnect behaviour for a single host.
///
/// The supervisor enters a reconnect loop whenever an established
/// connection drops unexpectedly. Use [`ReconnectPolicy::disabled`] to
/// opt out entirely (a single failure leaves the host in
/// [`super::HostState::Failed`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ReconnectPolicy {
    /// Backoff schedule between attempts.
    pub backoff: Backoff,
    /// Random jitter applied to each computed backoff. The actual delay
    /// is uniformly sampled from `[delay * (1 - jitter), delay * (1 + jitter)]`.
    /// `0.0` disables jitter; values are clamped to `[0.0, 1.0]`.
    pub jitter: f64,
    /// Maximum number of attempts before giving up. `None` retries forever.
    pub max_attempts: Option<u32>,
    /// When `true`, the attempt counter resets to zero after a successful
    /// connection so the next reconnect starts at the initial backoff.
    pub reset_on_success: bool,
}

impl ReconnectPolicy {
    /// Disable reconnects entirely. Use this when the consumer wants to
    /// drive reconnect logic itself.
    pub fn disabled() -> Self {
        Self {
            backoff: Backoff::Immediate,
            jitter: 0.0,
            max_attempts: Some(0),
            reset_on_success: true,
        }
    }

    /// Retry forever with no backoff. Almost certainly not what you want
    /// in production — useful for tests.
    pub fn immediate_forever() -> Self {
        Self {
            backoff: Backoff::Immediate,
            jitter: 0.0,
            max_attempts: None,
            reset_on_success: true,
        }
    }

    /// Sensible default: exponential backoff from 250 ms up to 30 s,
    /// 25 % jitter, retry forever, reset on success.
    pub fn exponential() -> Self {
        Self {
            backoff: Backoff::Exponential {
                initial: Duration::from_millis(250),
                max: Duration::from_secs(30),
                multiplier: 2.0,
            },
            jitter: 0.25,
            max_attempts: None,
            reset_on_success: true,
        }
    }

    /// Compute the delay before the `attempt`-th retry (1-based),
    /// applying jitter via the supplied random sample in `[0.0, 1.0]`.
    ///
    /// Exposed so tests can drive it deterministically; the runtime
    /// passes a real random sample.
    pub fn delay_with_jitter(&self, attempt: u32, sample: f64) -> Duration {
        let base = self.backoff.delay_for_attempt(attempt);
        if self.jitter <= 0.0 || base.is_zero() {
            return base;
        }
        let jitter = self.jitter.clamp(0.0, 1.0);
        // Sample maps to [-jitter, +jitter]
        let factor = 1.0 + (sample.clamp(0.0, 1.0) * 2.0 - 1.0) * jitter;
        Duration::from_secs_f64(base.as_secs_f64() * factor.max(0.0))
    }

    /// Whether `attempt` exceeds [`ReconnectPolicy::max_attempts`].
    pub fn attempts_exhausted(&self, attempt: u32) -> bool {
        self.max_attempts.is_some_and(|cap| attempt > cap)
    }
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self::exponential()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_backoff_caps_at_max() {
        let backoff = Backoff::Exponential {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(10),
            multiplier: 2.0,
        };
        assert_eq!(backoff.delay_for_attempt(1), Duration::from_secs(1));
        assert_eq!(backoff.delay_for_attempt(2), Duration::from_secs(2));
        assert_eq!(backoff.delay_for_attempt(3), Duration::from_secs(4));
        assert_eq!(backoff.delay_for_attempt(4), Duration::from_secs(8));
        // capped
        assert_eq!(backoff.delay_for_attempt(5), Duration::from_secs(10));
        assert_eq!(backoff.delay_for_attempt(50), Duration::from_secs(10));
    }

    #[test]
    fn jitter_zero_returns_base_delay() {
        let policy = ReconnectPolicy {
            backoff: Backoff::Constant {
                delay: Duration::from_secs(5),
            },
            jitter: 0.0,
            max_attempts: None,
            reset_on_success: true,
        };
        assert_eq!(policy.delay_with_jitter(1, 0.5), Duration::from_secs(5));
    }

    #[test]
    fn jitter_at_extremes_scales_delay() {
        let policy = ReconnectPolicy {
            backoff: Backoff::Constant {
                delay: Duration::from_secs(10),
            },
            jitter: 0.5,
            max_attempts: None,
            reset_on_success: true,
        };
        // sample 0 -> -50% -> 5 s
        assert_eq!(policy.delay_with_jitter(1, 0.0), Duration::from_secs(5));
        // sample 1 -> +50% -> 15 s
        assert_eq!(policy.delay_with_jitter(1, 1.0), Duration::from_secs(15));
        // sample 0.5 -> +0% -> 10 s
        assert_eq!(policy.delay_with_jitter(1, 0.5), Duration::from_secs(10));
    }

    #[test]
    fn disabled_policy_exhausts_immediately() {
        let policy = ReconnectPolicy::disabled();
        assert!(policy.attempts_exhausted(1));
    }

    #[test]
    fn unbounded_policy_never_exhausts() {
        let policy = ReconnectPolicy::exponential();
        assert!(!policy.attempts_exhausted(1_000_000));
    }
}
