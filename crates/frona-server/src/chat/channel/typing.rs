//! Per-chat "typing indicator" refresh loop, shared across channel adapters.
//!
//! Most platforms (Telegram, Signal, WhatsApp, Discord) show an ephemeral
//! "user is typing…" affordance that auto-fades after a few seconds. To keep
//! it lit during a long inference, the adapter must re-send the typing
//! action on a cadence. Sending it per streaming token would saturate the
//! platform's rate limits.
//!
//! `TypingIndicator` centralizes that pattern: each adapter composes it as
//! a field, calls [`TypingIndicator::start`] in `on_inference_start`, and
//! [`TypingIndicator::stop`] in `on_inference_done`. The per-chat refresh
//! loop runs in a tokio task until cancelled.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct TypingIndicator {
    /// chat_id → cancel token for the running refresh task.
    tasks: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl TypingIndicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a per-chat refresh loop. Fires `tick()` immediately, then once
    /// every `interval` until [`Self::stop`] is called for the same
    /// `chat_id` (or this `TypingIndicator` is dropped).
    ///
    /// Idempotent: calling `start` again for the same `chat_id` cancels the
    /// previous task before spawning the new one.
    pub async fn start<F, Fut>(&self, chat_id: String, interval: Duration, tick: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let cancel = CancellationToken::new();
        {
            let mut tasks = self.tasks.lock().await;
            if let Some(prev) = tasks.insert(chat_id, cancel.clone()) {
                prev.cancel();
            }
        }
        tokio::spawn(async move {
            loop {
                tick().await;
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {}
                }
            }
        });
    }

    /// Stop the refresh loop for `chat_id`. No-op if none is running.
    pub async fn stop(&self, chat_id: &str) {
        if let Some(cancel) = self.tasks.lock().await.remove(chat_id) {
            cancel.cancel();
        }
    }
}

impl Drop for TypingIndicator {
    fn drop(&mut self) {
        // Cancel all in-flight refresh tasks. Spawned tasks hold their own
        // clones of the tokens, so cancellation propagates without needing
        // to await each task individually.
        if let Ok(tasks) = self.tasks.try_lock() {
            for cancel in tasks.values() {
                cancel.cancel();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test(start_paused = true)]
    async fn fires_tick_immediately_then_at_interval() {
        let indicator = TypingIndicator::new();
        let count = Arc::new(AtomicUsize::new(0));

        let count_clone = count.clone();
        indicator
            .start("chat-1".into(), Duration::from_secs(4), move || {
                let count = count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::Relaxed);
                }
            })
            .await;

        // Immediate fire.
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(count.load(Ordering::Relaxed), 1);

        // After 4s → second fire.
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert_eq!(count.load(Ordering::Relaxed), 2);

        // After another 4s → third fire.
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert_eq!(count.load(Ordering::Relaxed), 3);

        indicator.stop("chat-1").await;
    }

    #[tokio::test(start_paused = true)]
    async fn stop_halts_the_loop() {
        let indicator = TypingIndicator::new();
        let count = Arc::new(AtomicUsize::new(0));

        let count_clone = count.clone();
        indicator
            .start("chat-1".into(), Duration::from_secs(4), move || {
                let count = count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::Relaxed);
                }
            })
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(count.load(Ordering::Relaxed), 1);

        indicator.stop("chat-1").await;

        // Several intervals later, still 1 — loop was cancelled.
        tokio::time::sleep(Duration::from_secs(20)).await;
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn restart_cancels_previous_task() {
        let indicator = TypingIndicator::new();
        let first_count = Arc::new(AtomicUsize::new(0));
        let second_count = Arc::new(AtomicUsize::new(0));

        let c = first_count.clone();
        indicator
            .start("chat-1".into(), Duration::from_secs(4), move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::Relaxed);
                }
            })
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(first_count.load(Ordering::Relaxed), 1);

        // Re-start with a different tick fn.
        let c = second_count.clone();
        indicator
            .start("chat-1".into(), Duration::from_secs(4), move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::Relaxed);
                }
            })
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(second_count.load(Ordering::Relaxed), 1);

        // Advance well past the interval — first fn must NOT have fired again.
        tokio::time::sleep(Duration::from_secs(20)).await;
        assert_eq!(first_count.load(Ordering::Relaxed), 1, "previous task should be cancelled");
        // Second fn keeps firing.
        assert!(second_count.load(Ordering::Relaxed) >= 4);

        indicator.stop("chat-1").await;
    }

    #[tokio::test(start_paused = true)]
    async fn independent_chats_run_independently() {
        let indicator = TypingIndicator::new();
        let count_a = Arc::new(AtomicUsize::new(0));
        let count_b = Arc::new(AtomicUsize::new(0));

        let c = count_a.clone();
        indicator
            .start("chat-a".into(), Duration::from_secs(4), move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::Relaxed);
                }
            })
            .await;

        let c = count_b.clone();
        indicator
            .start("chat-b".into(), Duration::from_secs(4), move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::Relaxed);
                }
            })
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(count_a.load(Ordering::Relaxed), 1);
        assert_eq!(count_b.load(Ordering::Relaxed), 1);

        // Stop A; B should keep firing.
        indicator.stop("chat-a").await;

        tokio::time::sleep(Duration::from_secs(20)).await;
        assert_eq!(count_a.load(Ordering::Relaxed), 1, "stopped chat should not refire");
        assert!(count_b.load(Ordering::Relaxed) >= 4, "unaffected chat should keep firing");

        indicator.stop("chat-b").await;
    }

    #[tokio::test]
    async fn stop_unknown_chat_is_no_op() {
        let indicator = TypingIndicator::new();
        indicator.stop("never-started").await; // does not panic
    }
}
