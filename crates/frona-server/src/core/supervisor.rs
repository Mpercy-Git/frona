use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::core::error::AppError;
use crate::notification::models::NotificationLevel;
use crate::notification::service::NotificationService;
use crate::chat::broadcast::BroadcastService;

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub health_check_interval: Duration,
    pub max_restart_attempts: u32,
    pub hibernate_after: Option<Duration>,
}

#[async_trait]
pub trait Supervisor: Send + Sync + 'static {
    async fn find_running(&self) -> Result<Vec<String>, AppError>;
    async fn start(&self, id: &str) -> Result<(), AppError>;
    async fn stop(&self, id: &str) -> Result<(), AppError>;
    async fn find_dead(&self) -> Result<Vec<String>, AppError>;
    async fn restart_count(&self, id: &str) -> u32;
    async fn mark_failed(&self, id: &str, reason: &str) -> Result<(), AppError>;
    async fn record_access(&self, id: &str);
    async fn find_idle(&self, idle_threshold: Duration) -> Result<Vec<String>, AppError>;
    async fn mark_hibernated(&self, id: &str) -> Result<(), AppError>;
    async fn owner_of(&self, id: &str) -> Result<String, AppError>;
    async fn display_name(&self, id: &str) -> String;

    async fn attempt_auto_fix(&self, _id: &str) -> bool {
        false
    }

    async fn notification_data(&self, id: &str, action: &str)
        -> crate::notification::models::NotificationData;

    fn label(&self) -> &'static str;
}

pub async fn run<S: Supervisor>(
    supervisor: Arc<S>,
    shutdown_token: CancellationToken,
    notification_service: NotificationService,
    broadcast_service: BroadcastService,
    config: SupervisorConfig,
) {
    let label = supervisor.label();

    match restore(&supervisor, &notification_service, &broadcast_service, label).await {
        Ok(count) => info!(label, count, "restoration complete"),
        Err(e) => warn!(label, error = %e, "restoration failed"),
    }

    loop {
        tokio::select! {
            () = tokio::time::sleep(config.health_check_interval) => {}
            () = shutdown_token.cancelled() => {
                info!(label, "supervisor stopping for shutdown");
                if let Ok(ids) = supervisor.find_running().await {
                    for id in &ids {
                        let _ = supervisor.stop(id).await;
                    }
                }
                return;
            }
        }

        health_tick(
            &supervisor,
            &notification_service,
            &broadcast_service,
            &config,
            label,
        )
        .await;

        if let Some(threshold) = config.hibernate_after {
            hibernate_tick(&supervisor, threshold, label).await;
        }
    }
}

async fn restore<S: Supervisor>(
    supervisor: &Arc<S>,
    notification_service: &NotificationService,
    broadcast_service: &BroadcastService,
    label: &str,
) -> Result<usize, AppError> {
    let ids = supervisor.find_running().await?;
    let count = ids.len();
    info!(label, count, "found entities to restore");
    for id in &ids {
        let name = supervisor.display_name(id).await;
        match supervisor.start(id).await {
            Ok(()) => info!(label, name = %name, "restored"),
            Err(e) => {
                warn!(label, name = %name, error = %e, "failed to restore");
                let _ = supervisor.mark_failed(id, &e.to_string()).await;
                send_notification(
                    supervisor,
                    notification_service,
                    broadcast_service,
                    id,
                    "restore",
                    NotificationLevel::Error,
                    &format!("{label} '{}' failed to start", supervisor.display_name(id).await),
                    &e.to_string(),
                )
                .await;
            }
        }
    }
    Ok(count)
}

async fn health_tick<S: Supervisor>(
    supervisor: &Arc<S>,
    notification_service: &NotificationService,
    broadcast_service: &BroadcastService,
    config: &SupervisorConfig,
    label: &str,
) {
    let dead = match supervisor.find_dead().await {
        Ok(d) => d,
        Err(e) => {
            warn!(label, error = %e, "health check failed");
            return;
        }
    };

    for id in &dead {
        let name = supervisor.display_name(id).await;
        let restarts = supervisor.restart_count(id).await;
        if restarts >= config.max_restart_attempts {
            let reason = format!("exceeded {max} restart attempts", max = config.max_restart_attempts);
            let _ = supervisor.mark_failed(id, &reason).await;
            warn!(label, name = %name, restarts, "exceeded max restarts");
            send_notification(
                supervisor,
                notification_service,
                broadcast_service,
                id,
                "crash",
                NotificationLevel::Error,
                &format!("{label} '{name}' crashed"),
                &reason,
            )
            .await;

            supervisor.attempt_auto_fix(id).await;
            continue;
        }

        warn!(label, name = %name, restarts, "process died, restarting");
        match supervisor.start(id).await {
            Ok(()) => info!(label, name = %name, "restarted after crash"),
            Err(e) => {
                warn!(label, name = %name, error = %e, "restart failed");
            }
        }
    }
}

async fn hibernate_tick<S: Supervisor>(
    supervisor: &Arc<S>,
    threshold: Duration,
    label: &str,
) {
    let idle = match supervisor.find_idle(threshold).await {
        Ok(i) => i,
        Err(e) => {
            warn!(label, error = %e, "idle check failed");
            return;
        }
    };
    for id in &idle {
        let name = supervisor.display_name(id).await;
        info!(label, name = %name, "hibernating idle entity");
        let _ = supervisor.stop(id).await;
        let _ = supervisor.mark_hibernated(id).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_notification<S: Supervisor>(
    supervisor: &Arc<S>,
    notification_service: &NotificationService,
    broadcast_service: &BroadcastService,
    id: &str,
    action: &str,
    level: NotificationLevel,
    title: &str,
    body: &str,
) {
    let Ok(user_id) = supervisor.owner_of(id).await else {
        return;
    };
    let data = supervisor.notification_data(id, action).await;
    if let Ok(notification) = notification_service
        .create(&user_id, data, level, title.to_string(), body.to_string())
        .await
    {
        broadcast_service.send_notification(&user_id, notification);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::notification::models::NotificationData;
    use tokio::sync::Mutex;

    struct FakeSupervisor {
        running: Mutex<Vec<String>>,
        dead: Mutex<Vec<String>>,
        idle: Mutex<Vec<String>>,
        restart_counts: Mutex<HashMap<String, u32>>,
        started: Mutex<Vec<String>>,
        stopped: Mutex<Vec<String>>,
        failed: Mutex<Vec<(String, String)>>,
        hibernated: Mutex<Vec<String>>,
        start_should_fail: Mutex<bool>,
    }

    impl FakeSupervisor {
        fn new() -> Self {
            Self {
                running: Mutex::new(vec![]),
                dead: Mutex::new(vec![]),
                idle: Mutex::new(vec![]),
                restart_counts: Mutex::new(HashMap::new()),
                started: Mutex::new(vec![]),
                stopped: Mutex::new(vec![]),
                failed: Mutex::new(vec![]),
                hibernated: Mutex::new(vec![]),
                start_should_fail: Mutex::new(false),
            }
        }
    }

    #[async_trait]
    impl Supervisor for FakeSupervisor {
        fn label(&self) -> &'static str { "fake" }

        async fn find_running(&self) -> Result<Vec<String>, AppError> {
            Ok(self.running.lock().await.clone())
        }
        async fn start(&self, id: &str) -> Result<(), AppError> {
            if *self.start_should_fail.lock().await {
                return Err(AppError::Tool("start failed".into()));
            }
            self.started.lock().await.push(id.to_string());
            Ok(())
        }
        async fn stop(&self, id: &str) -> Result<(), AppError> {
            self.stopped.lock().await.push(id.to_string());
            Ok(())
        }
        async fn find_dead(&self) -> Result<Vec<String>, AppError> {
            Ok(std::mem::take(&mut *self.dead.lock().await))
        }
        async fn restart_count(&self, id: &str) -> u32 {
            self.restart_counts.lock().await.get(id).copied().unwrap_or(0)
        }
        async fn mark_failed(&self, id: &str, reason: &str) -> Result<(), AppError> {
            self.failed.lock().await.push((id.to_string(), reason.to_string()));
            Ok(())
        }
        async fn record_access(&self, _id: &str) {}
        async fn find_idle(&self, _threshold: Duration) -> Result<Vec<String>, AppError> {
            Ok(std::mem::take(&mut *self.idle.lock().await))
        }
        async fn mark_hibernated(&self, id: &str) -> Result<(), AppError> {
            self.hibernated.lock().await.push(id.to_string());
            Ok(())
        }
        async fn owner_of(&self, _id: &str) -> Result<String, AppError> {
            Ok("user1".into())
        }
        async fn display_name(&self, id: &str) -> String {
            format!("Fake {id}")
        }
        async fn notification_data(&self, id: &str, action: &str) -> NotificationData {
            NotificationData::App { app_handle: id.to_string(), action: action.to_string() }
        }
    }

    fn test_notif(db: &surrealdb::Surreal<surrealdb::engine::local::Db>) -> NotificationService {
        use crate::db::repo::generic::SurrealRepo;
        NotificationService::new(SurrealRepo::new(db.clone()))
    }

    async fn mem_db() -> surrealdb::Surreal<surrealdb::engine::local::Db> {
        let db = surrealdb::Surreal::new::<surrealdb::engine::local::Mem>(()).await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn restore_starts_all_running_entities() {
        let db = mem_db().await;
        let fake = Arc::new(FakeSupervisor::new());
        *fake.running.lock().await = vec!["srv1".into(), "srv2".into()];

        restore(&fake, &test_notif(&db), &BroadcastService::new(), "test")
            .await
            .unwrap();

        assert_eq!(*fake.started.lock().await, vec!["srv1", "srv2"]);
        assert!(fake.failed.lock().await.is_empty());
    }

    #[tokio::test]
    async fn restore_marks_failed_when_start_errors() {
        let db = mem_db().await;
        let fake = Arc::new(FakeSupervisor::new());
        *fake.running.lock().await = vec!["srv1".into()];
        *fake.start_should_fail.lock().await = true;

        restore(&fake, &test_notif(&db), &BroadcastService::new(), "test")
            .await
            .unwrap();

        assert!(fake.started.lock().await.is_empty());
        assert_eq!(fake.failed.lock().await.len(), 1);
        assert_eq!(fake.failed.lock().await[0].0, "srv1");
    }

    #[tokio::test]
    async fn health_tick_restarts_dead_under_max() {
        let db = mem_db().await;
        let fake = Arc::new(FakeSupervisor::new());
        *fake.dead.lock().await = vec!["srv1".into()];
        fake.restart_counts.lock().await.insert("srv1".into(), 1);
        let config = SupervisorConfig {
            health_check_interval: Duration::from_secs(1),
            max_restart_attempts: 3,
            hibernate_after: None,
        };

        health_tick(&fake, &test_notif(&db), &BroadcastService::new(), &config, "test").await;

        assert_eq!(*fake.started.lock().await, vec!["srv1"]);
        assert!(fake.failed.lock().await.is_empty());
    }

    #[tokio::test]
    async fn health_tick_marks_failed_at_max_restarts() {
        let db = mem_db().await;
        let fake = Arc::new(FakeSupervisor::new());
        *fake.dead.lock().await = vec!["srv1".into()];
        fake.restart_counts.lock().await.insert("srv1".into(), 3);
        let config = SupervisorConfig {
            health_check_interval: Duration::from_secs(1),
            max_restart_attempts: 3,
            hibernate_after: None,
        };

        health_tick(&fake, &test_notif(&db), &BroadcastService::new(), &config, "test").await;

        assert!(fake.started.lock().await.is_empty());
        assert_eq!(fake.failed.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn hibernate_tick_stops_and_marks_idle() {
        let fake = Arc::new(FakeSupervisor::new());
        *fake.idle.lock().await = vec!["srv1".into(), "srv2".into()];

        hibernate_tick(&fake, Duration::from_secs(60), "test").await;

        assert_eq!(*fake.stopped.lock().await, vec!["srv1", "srv2"]);
        assert_eq!(*fake.hibernated.lock().await, vec!["srv1", "srv2"]);
    }

    #[tokio::test]
    async fn run_stops_all_on_shutdown() {
        let db = mem_db().await;
        let fake = Arc::new(FakeSupervisor::new());
        *fake.running.lock().await = vec!["srv1".into(), "srv2".into()];
        let shutdown = CancellationToken::new();
        let config = SupervisorConfig {
            health_check_interval: Duration::from_millis(50),
            max_restart_attempts: 3,
            hibernate_after: None,
        };

        let shutdown_clone = shutdown.clone();
        let fake_clone = fake.clone();
        let handle = tokio::spawn(async move {
            run(fake_clone, shutdown_clone, test_notif(&db), BroadcastService::new(), config).await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown.cancel();
        handle.await.unwrap();

        let stopped = fake.stopped.lock().await;
        assert!(stopped.contains(&"srv1".to_string()));
        assert!(stopped.contains(&"srv2".to_string()));
    }
}
