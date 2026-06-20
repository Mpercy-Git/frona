//! Asserts every TaskService mutation emits a `task_update` SSE event.

use chrono::Utc;
use frona::agent::task::models::{
    CreateTaskRequest, CronConcurrency, CronMode, Task, TaskKind, TaskStatus,
};
use frona::agent::task::service::TaskService;
use frona::chat::broadcast::BroadcastService;
use frona::core::repository::Repository;
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::tool::task::next_cron_occurrence;
use std::time::Duration;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;
use tokio::sync::mpsc;

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

async fn setup() -> (
    TaskService,
    Surreal<Db>,
    mpsc::UnboundedReceiver<Result<axum::response::sse::Event, std::convert::Infallible>>,
) {
    let db = test_db().await;
    let broadcast = BroadcastService::new();
    let (tx, rx) = mpsc::unbounded_channel();
    broadcast.register_session("user-1", tx).await;
    let svc = TaskService::new(SurrealRepo::new(db.clone()), broadcast);
    (svc, db, rx)
}

/// SSE dispatch is async; sleep briefly so events land before we count.
async fn drain_events(
    rx: &mut mpsc::UnboundedReceiver<Result<axum::response::sse::Event, std::convert::Infallible>>,
) -> usize {
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut count = 0;
    while rx.try_recv().is_ok() {
        count += 1;
    }
    count
}

#[tokio::test]
async fn create_broadcasts_pending() {
    let (svc, _db, mut rx) = setup().await;

    svc.create(
        "user-1",
        CreateTaskRequest {
            agent_id: "agent-1".into(),
            space_id: None,
            chat_id: None,
            title: "T".into(),
            description: None,
            source_agent_id: None,
            source_chat_id: None,
            resume_parent: None,
            run_at: None,
            quarantined: false,
            result_schema: None,
            result_description: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(drain_events(&mut rx).await, 1, "create must broadcast once");
}

#[tokio::test]
async fn mark_in_progress_broadcasts() {
    let (svc, db, mut rx) = setup().await;
    let task = seed_direct_task(&db, "user-1").await;
    let _ = drain_events(&mut rx).await;

    svc.mark_in_progress(&task.id, Some("chat-1")).await.unwrap();
    assert_eq!(drain_events(&mut rx).await, 1, "mark_in_progress must broadcast");
}

#[tokio::test]
async fn mark_completed_broadcasts() {
    let (svc, db, mut rx) = setup().await;
    let task = seed_direct_task(&db, "user-1").await;
    let _ = drain_events(&mut rx).await;

    svc.mark_completed(&task.id, Some("done".into())).await.unwrap();
    assert_eq!(drain_events(&mut rx).await, 1, "mark_completed must broadcast");
}

#[tokio::test]
async fn mark_failed_broadcasts() {
    let (svc, db, mut rx) = setup().await;
    let task = seed_direct_task(&db, "user-1").await;
    let _ = drain_events(&mut rx).await;

    svc.mark_failed(&task.id, "boom".into()).await.unwrap();
    assert_eq!(drain_events(&mut rx).await, 1, "mark_failed must broadcast");
}

#[tokio::test]
async fn mark_cancelled_broadcasts() {
    let (svc, db, mut rx) = setup().await;
    let task = seed_direct_task(&db, "user-1").await;
    let _ = drain_events(&mut rx).await;

    svc.mark_cancelled(&task.id).await.unwrap();
    assert_eq!(drain_events(&mut rx).await, 1, "mark_cancelled must broadcast");
}

#[tokio::test]
async fn mark_deferred_broadcasts() {
    let (svc, db, mut rx) = setup().await;
    let task = seed_direct_task(&db, "user-1").await;
    let _ = drain_events(&mut rx).await;

    svc.mark_deferred(&task.id, Utc::now() + chrono::Duration::minutes(5), "retry later")
        .await
        .unwrap();
    assert_eq!(drain_events(&mut rx).await, 1, "mark_deferred must broadcast");
}

#[tokio::test]
async fn cancel_cron_template_broadcasts_template_plus_each_active_child() {
    let (svc, _db, mut rx) = setup().await;

    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = svc
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::PerInstance, CronConcurrency::Allow, false, None, None)
        .await
        .unwrap();
    svc.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();
    svc.spawn_cron_run(&template, Utc::now(), 2).await.unwrap();

    assert_eq!(drain_events(&mut rx).await, 3, "create + 2 spawns");

    svc.cancel("user-1", &template.id).await.unwrap();
    let cancelled_events = drain_events(&mut rx).await;
    assert_eq!(
        cancelled_events, 3,
        "cancel must broadcast for template + 2 child runs (got {cancelled_events})"
    );
}

#[tokio::test]
async fn create_cron_template_broadcasts_pending() {
    let (svc, _db, mut rx) = setup().await;
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();

    svc.create_cron_template(
        "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
        next, None, None, None, None,
        CronMode::Singleton, CronConcurrency::Replace, false, None, None)
    .await
    .unwrap();

    assert_eq!(drain_events(&mut rx).await, 1, "create_cron_template must broadcast");
}

#[tokio::test]
async fn spawn_cron_run_broadcasts_pending() {
    let (svc, _db, mut rx) = setup().await;
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = svc
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::Singleton, CronConcurrency::Replace, false, None, None)
        .await
        .unwrap();
    let _ = drain_events(&mut rx).await;

    svc.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();
    assert_eq!(drain_events(&mut rx).await, 1, "spawn_cron_run must broadcast");
}

async fn seed_direct_task(db: &Surreal<Db>, user_id: &str) -> Task {
    let now = Utc::now();
    let task = Task {
        id: frona::core::repository::new_id(),
        user_id: user_id.to_string(),
        agent_id: "agent-1".into(),
        space_id: None,
        chat_id: None,
        title: "T".into(),
        description: "d".into(),
        status: TaskStatus::Pending,
        kind: TaskKind::Direct { source_chat_id: None },
        run_at: None,
        result_summary: None,
        error_message: None,
        quarantined: false,
        result_schema: None,
        result_description: None,
        created_at: now,
        updated_at: now,
    };
    let repo: SurrealRepo<Task> = SurrealRepo::new(db.clone());
    repo.create(&task).await.unwrap()
}
