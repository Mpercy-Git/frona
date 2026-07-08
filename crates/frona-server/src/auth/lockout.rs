use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone)]
pub struct LoginAttemptTracker {
    /// Maps identifier → failure count. Entries auto-expire after the lockout window (TTL).
    cache: moka::future::Cache<String, Arc<AtomicU32>>,
    max_attempts: u32,
}

use std::sync::Arc;

impl LoginAttemptTracker {
    pub fn new(max_attempts: u32, lockout_minutes: u64) -> Self {
        let cache = moka::future::Cache::builder()
            .max_capacity(10_000)
            .time_to_live(std::time::Duration::from_secs(lockout_minutes * 60))
            .build();
        Self {
            cache,
            max_attempts,
        }
    }

    pub async fn is_locked(&self, identifier: &str) -> bool {
        self.cache
            .get(identifier)
            .await
            .is_some_and(|c| c.load(Ordering::Relaxed) >= self.max_attempts)
    }

    pub async fn record_failure(&self, identifier: &str) {
        let counter = self
            .cache
            .get_with(identifier.to_string(), async { Arc::new(AtomicU32::new(0)) })
            .await;
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn clear(&self, identifier: &str) {
        self.cache.invalidate(identifier).await;
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
        assert!(!tracker.is_locked("user@test.com").await);
    }

    #[tokio::test]
    async fn locks_after_max_attempts() {
        let tracker = LoginAttemptTracker::new(3, 15);
        for _ in 0..3 {
            tracker.record_failure("user@test.com").await;
        }
        assert!(tracker.is_locked("user@test.com").await);
    }

    #[tokio::test]
    async fn clear_resets_lockout() {
        let tracker = LoginAttemptTracker::new(3, 15);
        for _ in 0..3 {
            tracker.record_failure("user@test.com").await;
        }
        tracker.clear("user@test.com").await;
        assert!(!tracker.is_locked("user@test.com").await);
    }

    #[tokio::test]
    async fn different_identifiers_tracked_separately() {
        let tracker = LoginAttemptTracker::new(2, 15);
        for _ in 0..2 {
            tracker.record_failure("a@test.com").await;
        }
        assert!(tracker.is_locked("a@test.com").await);
        assert!(!tracker.is_locked("b@test.com").await);
    }
}
