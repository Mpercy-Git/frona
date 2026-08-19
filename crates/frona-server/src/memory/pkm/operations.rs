use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, Weak};

use tokio::sync::{Mutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub(crate) struct PkmOperationCoordinator {
    users: Arc<StdMutex<HashMap<String, Arc<UserOperations>>>>,
}

struct UserOperations {
    state: StdMutex<UserOperationState>,
    consolidation: Arc<Mutex<()>>,
    page_edits: StdMutex<HashMap<String, Weak<Mutex<()>>>>,
    reset_barrier: Arc<RwLock<()>>,
}

#[derive(Default)]
struct UserOperationState {
    reset_blocked: bool,
    next_generation: u64,
    active_consolidation: Option<(u64, CancellationToken)>,
}

impl Default for UserOperations {
    fn default() -> Self {
        Self {
            state: StdMutex::new(UserOperationState::default()),
            consolidation: Arc::new(Mutex::new(())),
            page_edits: StdMutex::new(HashMap::new()),
            reset_barrier: Arc::new(RwLock::new(())),
        }
    }
}

impl PkmOperationCoordinator {
    fn user(&self, user_id: &str) -> Arc<UserOperations> {
        let mut users = self.users.lock().unwrap_or_else(|error| error.into_inner());
        users
            .entry(user_id.to_string())
            .or_insert_with(|| Arc::new(UserOperations::default()))
            .clone()
    }

    pub(crate) fn try_begin_consolidation(&self, user_id: &str) -> Option<ConsolidationGuard> {
        let operations = self.user(user_id);
        let mut state = operations
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.reset_blocked {
            return None;
        }
        let consolidation = operations.consolidation.clone().try_lock_owned().ok()?;
        let writer = operations.reset_barrier.clone().try_read_owned().ok()?;
        state.next_generation = state.next_generation.wrapping_add(1);
        let generation = state.next_generation;
        let cancellation = CancellationToken::new();
        state.active_consolidation = Some((generation, cancellation.clone()));
        drop(state);
        Some(ConsolidationGuard {
            operations,
            generation,
            cancellation,
            _consolidation: consolidation,
            _writer: writer,
        })
    }

    pub(crate) fn try_begin_write(&self, user_id: &str) -> Option<NormalWriteGuard> {
        let operations = self.user(user_id);
        let state = operations
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.reset_blocked {
            return None;
        }
        let writer = operations.reset_barrier.clone().try_read_owned().ok()?;
        drop(state);
        Some(NormalWriteGuard { _writer: writer })
    }

    /// Serialize the short commit-and-file-finalization section for one page. Model
    /// inference happens before this lock. Database compare-and-set remains the
    /// authority when another process writes the same page.
    pub(crate) async fn begin_page_edit(&self, user_id: &str, path: &str) -> OwnedMutexGuard<()> {
        let operations = self.user(user_id);
        let page = {
            let mut pages = operations
                .page_edits
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            pages.retain(|_, page| page.strong_count() > 0);
            match pages.get(path).and_then(Weak::upgrade) {
                Some(page) => page,
                None => {
                    let page = Arc::new(Mutex::new(()));
                    pages.insert(path.to_string(), Arc::downgrade(&page));
                    page
                }
            }
        };
        page.lock_owned().await
    }

    pub(crate) fn mark_reset_pending(&self, user_id: &str) {
        let operations = self.user(user_id);
        let mut state = operations
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.reset_blocked = true;
        if let Some((_, cancellation)) = &state.active_consolidation {
            cancellation.cancel();
        }
    }

    pub(crate) async fn begin_reset(&self, user_id: &str) -> ResetGuard {
        let operations = self.user(user_id);
        self.mark_reset_pending(user_id);
        let writer = operations.reset_barrier.clone().write_owned().await;
        ResetGuard { _writer: writer }
    }

    pub(crate) fn clear_reset(&self, user_id: &str) {
        let operations = self.user(user_id);
        let mut state = operations
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.reset_blocked = false;
    }
}

pub(crate) struct ConsolidationGuard {
    operations: Arc<UserOperations>,
    generation: u64,
    cancellation: CancellationToken,
    _consolidation: OwnedMutexGuard<()>,
    _writer: OwnedRwLockReadGuard<()>,
}

impl ConsolidationGuard {
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

impl Drop for ConsolidationGuard {
    fn drop(&mut self) {
        let mut state = self
            .operations
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .active_consolidation
            .as_ref()
            .is_some_and(|(generation, _)| *generation == self.generation)
        {
            state.active_consolidation = None;
        }
    }
}

pub(crate) struct NormalWriteGuard {
    _writer: OwnedRwLockReadGuard<()>,
}

pub(crate) struct ResetGuard {
    _writer: OwnedRwLockWriteGuard<()>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reset_cancels_one_user_and_does_not_block_another() {
        let coordinator = PkmOperationCoordinator::default();
        let first = coordinator.try_begin_consolidation("u1").unwrap();
        let other = coordinator.try_begin_consolidation("u2").unwrap();

        coordinator.mark_reset_pending("u1");
        assert!(first.cancellation().is_cancelled());
        assert!(!other.cancellation().is_cancelled());
        assert!(coordinator.try_begin_consolidation("u1").is_none());
        assert!(coordinator.try_begin_write("u1").is_none());
    }

    #[tokio::test]
    async fn reset_waits_for_existing_writer_in_the_background() {
        let coordinator = PkmOperationCoordinator::default();
        let writer = coordinator.try_begin_write("u1").unwrap();
        coordinator.mark_reset_pending("u1");
        let waiting = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.begin_reset("u1").await })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(writer);
        let reset = waiting.await.unwrap();
        drop(reset);
        coordinator.clear_reset("u1");
        assert!(coordinator.try_begin_write("u1").is_some());
    }
}
