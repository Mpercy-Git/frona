use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Result of asking whether an identifier may attempt a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockStatus {
    Open,
    Locked { retry_after: Duration },
}

impl LockStatus {
    pub fn is_locked(&self) -> bool {
        matches!(self, Self::Locked { .. })
    }
}

/// Failure bookkeeping for one identifier.
///
/// `locked_until` is the authority on whether a login is refused — the cache
/// TTL only garbage-collects idle entries. Keeping the deadline explicit means
/// the lockout runs for its full duration from the moment the threshold is
/// crossed, rather than from the first failure in the burst.
#[derive(Debug)]
struct Attempt {
    failures: u32,
    locked_until: Option<Instant>,
}

#[derive(Clone)]
pub struct LoginAttemptTracker {
    cache: moka::future::Cache<String, Arc<Mutex<Attempt>>>,
    max_attempts: u32,
    lockout: Duration,
}

impl LoginAttemptTracker {
    /// `max_attempts` of 0 disables lockout entirely.
    pub fn new(max_attempts: u32, lockout_minutes: u64) -> Self {
        let lockout = Duration::from_secs(lockout_minutes.saturating_mul(60));
        // Idle rather than live expiry: the failure counter decays only after a
        // genuinely quiet window. Since an entry is touched by every attempt on
        // that identifier, and the lock deadline is at most `lockout` away from
        // the last touch, eviction can never cut a lockout short.
        let cache = moka::future::Cache::builder()
            .max_capacity(10_000)
            .time_to_idle(lockout.max(Duration::from_secs(60)))
            .build();
        Self {
            cache,
            max_attempts,
            lockout,
        }
    }

    /// Identifiers are normalized the same way the user lookups normalize them
    /// (`Handle::try_new` trims and lowercases; the email query lowercases), so
    /// case and whitespace variants can't each get their own budget of attempts.
    pub fn normalize(identifier: &str) -> String {
        identifier.trim().to_lowercase()
    }

    fn enabled(&self) -> bool {
        self.max_attempts > 0
    }

    /// Drops a lock that has run its course, resetting the counter with it —
    /// otherwise the next single failure would immediately re-lock.
    fn settle(attempt: &mut Attempt, now: Instant) {
        if let Some(until) = attempt.locked_until
            && until <= now
        {
            attempt.failures = 0;
            attempt.locked_until = None;
        }
    }

    pub async fn check(&self, identifier: &str) -> LockStatus {
        if !self.enabled() {
            return LockStatus::Open;
        }
        let Some(entry) = self.cache.get(&Self::normalize(identifier)).await else {
            return LockStatus::Open;
        };
        let now = Instant::now();
        let mut attempt = entry.lock().unwrap_or_else(PoisonError::into_inner);
        Self::settle(&mut attempt, now);
        match attempt.locked_until {
            Some(until) => LockStatus::Locked {
                retry_after: until.saturating_duration_since(now),
            },
            None => LockStatus::Open,
        }
    }

    /// Records one failed attempt. Returns `true` if this failure is the one
    /// that engaged the lock, so the caller can log/count that transition
    /// without re-reading the state.
    pub async fn record_failure(&self, identifier: &str) -> bool {
        if !self.enabled() {
            return false;
        }
        let entry = self
            .cache
            .get_with(Self::normalize(identifier), async {
                Arc::new(Mutex::new(Attempt {
                    failures: 0,
                    locked_until: None,
                }))
            })
            .await;

        let now = Instant::now();
        let mut attempt = entry.lock().unwrap_or_else(PoisonError::into_inner);
        Self::settle(&mut attempt, now);
        attempt.failures = attempt.failures.saturating_add(1);

        // Already-locked identifiers keep counting but don't push the deadline
        // out, so hammering a locked account can't extend its own lockout.
        if attempt.failures >= self.max_attempts && attempt.locked_until.is_none() {
            attempt.locked_until = Some(now + self.lockout);
            return true;
        }
        false
    }

    pub async fn clear(&self, identifier: &str) {
        self.cache.invalidate(&Self::normalize(identifier)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_login_before_max_attempts() {
        let tracker = LoginAttemptTracker::new(3, 15);
        tracker.record_failure("user@test.com").await;
        tracker.record_failure("user@test.com").await;
        assert_eq!(tracker.check("user@test.com").await, LockStatus::Open);
    }

    #[tokio::test]
    async fn locks_after_max_attempts() {
        let tracker = LoginAttemptTracker::new(3, 15);
        for _ in 0..3 {
            tracker.record_failure("user@test.com").await;
        }
        assert!(tracker.check("user@test.com").await.is_locked());
    }

    #[tokio::test]
    async fn clear_resets_lockout() {
        let tracker = LoginAttemptTracker::new(3, 15);
        for _ in 0..3 {
            tracker.record_failure("user@test.com").await;
        }
        tracker.clear("user@test.com").await;
        assert_eq!(tracker.check("user@test.com").await, LockStatus::Open);
    }

    #[tokio::test]
    async fn different_identifiers_tracked_separately() {
        let tracker = LoginAttemptTracker::new(2, 15);
        for _ in 0..2 {
            tracker.record_failure("a@test.com").await;
        }
        assert!(tracker.check("a@test.com").await.is_locked());
        assert_eq!(tracker.check("b@test.com").await, LockStatus::Open);
    }

    #[tokio::test]
    async fn case_and_whitespace_variants_share_one_bucket() {
        let tracker = LoginAttemptTracker::new(3, 15);
        tracker.record_failure("User@Test.com").await;
        tracker.record_failure("  user@test.com  ").await;
        tracker.record_failure("USER@TEST.COM").await;
        assert!(tracker.check("user@test.com").await.is_locked());
    }

    #[tokio::test]
    async fn clear_is_case_insensitive() {
        let tracker = LoginAttemptTracker::new(2, 15);
        for _ in 0..2 {
            tracker.record_failure("user@test.com").await;
        }
        tracker.clear("USER@test.com ").await;
        assert_eq!(tracker.check("user@test.com").await, LockStatus::Open);
    }

    #[tokio::test]
    async fn reports_the_transition_into_lockout_once() {
        let tracker = LoginAttemptTracker::new(2, 15);
        assert!(!tracker.record_failure("user@test.com").await);
        assert!(tracker.record_failure("user@test.com").await);
        // Still locked, but the transition has already been reported.
        assert!(!tracker.record_failure("user@test.com").await);
    }

    #[tokio::test]
    async fn retry_after_counts_down_from_the_lock_moment() {
        let tracker = LoginAttemptTracker::new(1, 15);
        tracker.record_failure("user@test.com").await;
        let LockStatus::Locked { retry_after } = tracker.check("user@test.com").await else {
            panic!("expected lockout");
        };
        assert!(retry_after <= Duration::from_secs(15 * 60));
        assert!(retry_after > Duration::from_secs(14 * 60));
    }

    #[tokio::test]
    async fn lock_expires_and_resets_the_counter() {
        // Zero-length window: the lock is already expired when observed, which
        // exercises the settle path without sleeping.
        let tracker = LoginAttemptTracker::new(2, 0);
        assert!(!tracker.record_failure("user@test.com").await);
        assert!(tracker.record_failure("user@test.com").await);
        assert_eq!(tracker.check("user@test.com").await, LockStatus::Open);
        // Counter was reset with the lock, so a single further failure — the
        // third overall — must not immediately re-lock.
        assert!(!tracker.record_failure("user@test.com").await);
    }

    #[tokio::test]
    async fn zero_max_attempts_disables_lockout() {
        let tracker = LoginAttemptTracker::new(0, 15);
        for _ in 0..50 {
            assert!(!tracker.record_failure("user@test.com").await);
        }
        assert_eq!(tracker.check("user@test.com").await, LockStatus::Open);
    }
}
