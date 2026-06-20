//! Lock-free fan-out event bus.
//!
//! Each subscriber gets its own unbounded mpsc; `publish` fans out to every
//! live subscriber. Reads of the subscriber list are atomic (`arc-swap`), so
//! multiple producers can publish concurrently without contending. Writes
//! (subscribe / dead-sender cleanup) clone the Vec and atomic-swap it.
//!
//! Semantics:
//! - **Lossless** as long as receivers' `recv()` is drained — unbounded mpsc
//!   never drops, never returns `Lagged`. Memory grows under sustained slow
//!   consumers.
//! - **Lock-free fanout** — publish doesn't take a mutex. Concurrent publishes
//!   run in parallel.
//! - **Lazy cleanup** — closed senders are filtered out at the next publish
//!   that observes a send failure.

use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

#[derive(Clone)]
pub struct EventBus<T: Clone + Send + Sync + 'static> {
    subscribers: Arc<ArcSwap<Vec<UnboundedSender<T>>>>,
}

impl<T: Clone + Send + Sync + 'static> EventBus<T> {
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(ArcSwap::from_pointee(Vec::new())),
        }
    }

    pub fn subscribe(&self) -> UnboundedReceiver<T> {
        let (tx, rx) = mpsc::unbounded_channel::<T>();
        self.subscribers.rcu(|current| {
            let mut new = (**current).clone();
            new.push(tx.clone());
            Arc::new(new)
        });
        rx
    }

    pub fn publish(&self, event: T) {
        let snapshot = self.subscribers.load();
        let mut had_dead = false;
        for tx in snapshot.iter() {
            if tx.send(event.clone()).is_err() {
                had_dead = true;
            }
        }
        if had_dead {
            self.subscribers.rcu(|current| {
                let alive: Vec<UnboundedSender<T>> = current
                    .iter()
                    .filter(|tx| !tx.is_closed())
                    .cloned()
                    .collect();
                Arc::new(alive)
            });
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.load().len()
    }
}

impl<T: Clone + Send + Sync + 'static> Default for EventBus<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn single_subscriber_receives_event() {
        let bus = EventBus::<i32>::new();
        let mut rx = bus.subscribe();
        bus.publish(42);
        assert_eq!(rx.recv().await, Some(42));
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let bus = EventBus::<i32>::new();
        let mut r1 = bus.subscribe();
        let mut r2 = bus.subscribe();
        let mut r3 = bus.subscribe();
        bus.publish(7);
        assert_eq!(r1.recv().await, Some(7));
        assert_eq!(r2.recv().await, Some(7));
        assert_eq!(r3.recv().await, Some(7));
    }

    #[tokio::test]
    async fn dropped_receiver_is_cleaned_up_on_next_publish() {
        let bus = EventBus::<i32>::new();
        let _r1 = bus.subscribe();
        let r2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
        drop(r2);
        bus.publish(1);
        assert_eq!(bus.subscriber_count(), 1);
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_is_a_noop() {
        let bus = EventBus::<i32>::new();
        bus.publish(99);
    }

    #[tokio::test]
    async fn many_publishes_buffered_in_order() {
        let bus = EventBus::<u32>::new();
        let mut rx = bus.subscribe();
        for i in 0..1000 {
            bus.publish(i);
        }
        for i in 0..1000 {
            assert_eq!(rx.recv().await, Some(i));
        }
    }

    #[tokio::test]
    async fn concurrent_publishes_fan_out_without_loss() {
        let bus = EventBus::<u32>::new();
        let mut rx = bus.subscribe();
        let bus2 = bus.clone();
        let bus3 = bus.clone();
        let h1 = tokio::spawn(async move {
            for i in 0..500 {
                bus2.publish(i);
            }
        });
        let h2 = tokio::spawn(async move {
            for i in 500..1000 {
                bus3.publish(i);
            }
        });
        h1.await.unwrap();
        h2.await.unwrap();
        let mut seen: Vec<u32> = Vec::new();
        while let Ok(v) = rx.try_recv() {
            seen.push(v);
        }
        assert_eq!(seen.len(), 1000);
        seen.sort_unstable();
        assert_eq!(seen, (0..1000).collect::<Vec<_>>());
    }
}
