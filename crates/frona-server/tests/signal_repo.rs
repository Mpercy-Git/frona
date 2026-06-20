//! Integration tests for the Signal task persistence layer.
//!
//! Validates that `TaskService::create_signal` round-trips through SurrealDB
//! and that `find_pending_signal_tasks` correctly filters by `kind.Signal` +
//! `status.Pending`. These are the queries `SignalService::rebuild_from_db`
//! relies on at startup.

use chrono::{Duration, Utc};
use frona::agent::task::models::{TaskKind, TaskStatus};
use frona::agent::task::service::TaskService;
use frona::db::init::setup_schema;
use frona::db::repo::generic::SurrealRepo;
use surrealdb::Surreal;
use surrealdb::engine::local::Mem;

async fn build_task_service() -> (Surreal<surrealdb::engine::local::Db>, TaskService) {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();
    setup_schema(&db).await.unwrap();
    let repo = SurrealRepo::new(db.clone());
    (db, TaskService::new(repo, frona::chat::broadcast::BroadcastService::new()))
}

#[tokio::test]
async fn create_signal_persists_with_signal_kind() {
    let (_db, svc) = build_task_service().await;

    let task = svc
        .create_signal(
            "user-1",
            "agent-1".into(),
            "chat-A".into(),
            "Wait for: code".into(),
            "Wait for: code".into(),
            true,
            frona::agent::task::models::SignalMode::Once,
            vec!["verification_code".into()],
            vec!["sms".into()],
            vec![],
            None,
            50,
        None,
        )
        .await
        .expect("create_signal");

    assert_eq!(task.user_id, "user-1");
    assert_eq!(task.agent_id, "agent-1");
    assert_eq!(task.status, TaskStatus::Pending);
    match task.kind {
        TaskKind::Signal {
            ref source_chat_id,
            resume_parent,
            ref expected_categories,
            ref expected_channels,
            max_evaluations,
            evaluation_count,
            ..
        } => {
            assert_eq!(source_chat_id, "chat-A");
            assert!(resume_parent);
            assert_eq!(expected_categories, &vec!["verification_code".to_string()]);
            assert_eq!(expected_channels, &vec!["sms".to_string()]);
            assert_eq!(max_evaluations, 50);
            assert_eq!(evaluation_count, 0);
        }
        _ => panic!("expected Signal kind"),
    }
}

#[tokio::test]
async fn find_pending_signal_tasks_returns_only_signals() {
    let (db, svc) = build_task_service().await;

    // Two signal tasks, one direct task — only the signals should come back.
    let signal1 = svc
        .create_signal(
            "user-1",
            "agent-1".into(),
            "chat-A".into(),
            "s1".into(),
            "s1".into(),
            false,
            frona::agent::task::models::SignalMode::Once,
            vec!["t".into()],
            vec![],
            vec![],
            None,
            10,
        None,
        )
        .await
        .unwrap();

    let signal2 = svc
        .create_signal(
            "user-2",
            "agent-2".into(),
            "chat-B".into(),
            "s2".into(),
            "s2".into(),
            true,
            frona::agent::task::models::SignalMode::Once,
            vec![],
            vec!["sms".into()],
            vec![],
            Some(Utc::now() + Duration::hours(1)),
            5,
        None,
        )
        .await
        .unwrap();

    svc.create(
        "user-1",
        frona::agent::task::models::CreateTaskRequest {
            agent_id: "agent-1".into(),
            space_id: None,
            chat_id: None,
            title: "direct".into(),
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

    let pending = svc.list_pending_signal_tasks().await.unwrap();
    let ids: Vec<&str> = pending.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&signal1.id.as_str()));
    assert!(ids.contains(&signal2.id.as_str()));
    assert_eq!(pending.len(), 2);

    let _ = db; // hold the db handle alive
}

#[tokio::test]
async fn find_pending_signal_tasks_excludes_completed() {
    let (_db, svc) = build_task_service().await;

    let task = svc
        .create_signal(
            "user-1",
            "agent-1".into(),
            "chat-A".into(),
            "s".into(),
            "s".into(),
            true,
            frona::agent::task::models::SignalMode::Once,
            vec!["t".into()],
            vec![],
            vec![],
            None,
            5,
        None,
        )
        .await
        .unwrap();

    svc.mark_completed(&task.id, Some("done".into()))
        .await
        .unwrap();

    let pending = svc.list_pending_signal_tasks().await.unwrap();
    assert!(pending.iter().all(|t| t.id != task.id));
}

#[tokio::test]
async fn find_expired_signal_tasks_returns_only_past_expires_at() {
    let (_db, svc) = build_task_service().await;

    let now = Utc::now();

    let expired = svc
        .create_signal(
            "user-1",
            "agent-1".into(),
            "chat-A".into(),
            "expired".into(),
            "s".into(),
            true,
            frona::agent::task::models::SignalMode::Once,
            vec!["t".into()],
            vec![],
            vec![],
            Some(now - Duration::minutes(5)),
            5,
        None,
        )
        .await
        .unwrap();

    let _future = svc
        .create_signal(
            "user-1",
            "agent-1".into(),
            "chat-A".into(),
            "future".into(),
            "s".into(),
            true,
            frona::agent::task::models::SignalMode::Once,
            vec!["t".into()],
            vec![],
            vec![],
            Some(now + Duration::hours(1)),
            5,
        None,
        )
        .await
        .unwrap();

    let _no_expiry = svc
        .create_signal(
            "user-1",
            "agent-1".into(),
            "chat-A".into(),
            "no-expiry".into(),
            "s".into(),
            true,
            frona::agent::task::models::SignalMode::Once,
            vec!["t".into()],
            vec![],
            vec![],
            None,
            5,
        None,
        )
        .await
        .unwrap();

    let result = svc.find_expired_signal_tasks().await.unwrap();
    let ids: Vec<&str> = result.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec![expired.id.as_str()]);
}

#[tokio::test]
async fn find_expired_signal_tasks_excludes_non_pending() {
    let (_db, svc) = build_task_service().await;

    let task = svc
        .create_signal(
            "user-1",
            "agent-1".into(),
            "chat-A".into(),
            "expired-but-completed".into(),
            "s".into(),
            true,
            frona::agent::task::models::SignalMode::Once,
            vec!["t".into()],
            vec![],
            vec![],
            Some(Utc::now() - Duration::minutes(5)),
            5,
        None,
        )
        .await
        .unwrap();

    // Once completed, an expired Signal task must NOT show up — the sweeper
    // would otherwise re-process tasks that are already done.
    svc.mark_completed(&task.id, Some("done".into()))
        .await
        .unwrap();

    let expired = svc.find_expired_signal_tasks().await.unwrap();
    assert!(expired.iter().all(|t| t.id != task.id));
}

#[tokio::test]
async fn find_expired_signal_tasks_excludes_non_signal_kinds() {
    let (db, svc) = build_task_service().await;

    // Direct task with run_at in the past — should NOT be returned by the
    // signal-specific expiry query.
    svc.create(
        "user-1",
        frona::agent::task::models::CreateTaskRequest {
            agent_id: "agent-1".into(),
            space_id: None,
            chat_id: None,
            title: "stale direct".into(),
            description: None,
            source_agent_id: None,
            source_chat_id: None,
            resume_parent: None,
            run_at: Some(Utc::now() - Duration::hours(1)),
        quarantined: false,
        result_schema: None,
        result_description: None,
        },
    )
    .await
    .unwrap();

    let expired = svc.find_expired_signal_tasks().await.unwrap();
    assert!(expired.is_empty());
    let _ = db;
}

#[tokio::test]
async fn save_persists_signal_evaluation_count() {
    let (_db, svc) = build_task_service().await;

    let mut task = svc
        .create_signal(
            "user-1",
            "agent-1".into(),
            "chat-A".into(),
            "s".into(),
            "s".into(),
            true,
            frona::agent::task::models::SignalMode::Once,
            vec!["t".into()],
            vec![],
            vec![],
            None,
            5,
        None,
        )
        .await
        .unwrap();

    if let TaskKind::Signal {
        ref mut evaluation_count,
        ..
    } = task.kind
    {
        *evaluation_count = 3;
    }
    svc.save(&task).await.unwrap();

    let reloaded = svc.find_by_id(&task.id).await.unwrap().unwrap();
    match reloaded.kind {
        TaskKind::Signal {
            evaluation_count, ..
        } => {
            assert_eq!(evaluation_count, 3);
        }
        _ => panic!("expected Signal"),
    }
}
