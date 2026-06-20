//! Service+repo tests for the CronRun model.

use chrono::Utc;
use frona::agent::task::models::{CronConcurrency, CronMode, TaskKind, TaskStatus};
use frona::agent::task::service::TaskService;
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::tool::task::next_cron_occurrence;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

fn svc(db: Surreal<Db>) -> TaskService {
    TaskService::new(SurrealRepo::new(db), frona::chat::broadcast::BroadcastService::new())
}

#[tokio::test]
async fn spawn_cron_run_links_back_to_template() {
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1",
            "agent-1",
            "ping",
            "do a thing",
            "* * * * *",
            "UTC".into(),
            next,
            None,
            None,
            Some("chat-caller".into()),
            None,
            CronMode::Singleton,
            CronConcurrency::Replace,
            false, None, None)
        .await
        .unwrap();

    let run = s.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();

    match &run.kind {
        TaskKind::CronRun {
            source_cron_id,
            source_chat_id,
            sequence_num,
            ..
        } => {
            assert_eq!(source_cron_id, &template.id);
            assert_eq!(source_chat_id.as_deref(), Some("chat-caller"));
            assert_eq!(*sequence_num, 1);
        }
        _ => panic!("Expected CronRun variant"),
    }
    assert_eq!(run.status, TaskStatus::Pending);
    assert!(run.chat_id.is_none());
}

#[tokio::test]
async fn find_runs_by_cron_returns_all_runs_for_template() {
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::PerInstance, CronConcurrency::Forbid, false, None, None)
        .await
        .unwrap();

    for seq in 1..=3 {
        s.spawn_cron_run(&template, Utc::now(), seq).await.unwrap();
    }

    let runs = s.find_runs_by_cron(&template.id).await.unwrap();
    assert_eq!(runs.len(), 3);
    let nums: Vec<u64> = runs
        .iter()
        .map(|r| match &r.kind {
            TaskKind::CronRun { sequence_num, .. } => *sequence_num,
            _ => panic!("expected CronRun"),
        })
        .collect();
    assert_eq!(nums, vec![3, 2, 1]);
}

#[tokio::test]
async fn find_active_runs_excludes_completed() {
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::Singleton, CronConcurrency::Replace, false, None, None)
        .await
        .unwrap();

    let r1 = s.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();
    let r2 = s.spawn_cron_run(&template, Utc::now(), 2).await.unwrap();
    let _r3 = s.spawn_cron_run(&template, Utc::now(), 3).await.unwrap();

    s.mark_completed(&r1.id, Some("done".into())).await.unwrap();
    s.mark_failed(&r2.id, "boom".into()).await.unwrap();

    let active = s.find_active_runs_by_cron(&template.id).await.unwrap();
    assert_eq!(active.len(), 1, "only the still-Pending run should be active");
}

#[tokio::test]
async fn find_orphaned_cron_runs_returns_only_in_progress() {
    // Pending CronRuns were spawned by the scheduler but the executor never
    // picked them up — they're not crash orphans, just queued work. Only
    // InProgress runs represent "crashed mid-execution" and should be matched.
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::Singleton, CronConcurrency::Replace, false, None, None)
        .await
        .unwrap();

    let pending_run = s.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();
    let in_flight_run = s.spawn_cron_run(&template, Utc::now(), 2).await.unwrap();

    s.mark_in_progress(&in_flight_run.id, Some("chat-x")).await.unwrap();

    let orphans = s.find_orphaned_cron_runs().await.unwrap();
    let ids: Vec<String> = orphans.iter().map(|t| t.id.clone()).collect();
    assert!(
        !ids.contains(&pending_run.id),
        "Pending CronRun must not be treated as orphan"
    );
    assert!(
        ids.contains(&in_flight_run.id),
        "InProgress CronRun must be returned as orphan"
    );
    assert_eq!(orphans.len(), 1, "exactly one orphan expected");
}

#[tokio::test]
async fn legacy_cron_deserializes_with_defaults() {
    let json = serde_json::json!({
        "type": "Cron",
        "cron_expression": "0 9 * * *",
        "timezone": "UTC",
        "next_run_at": "2030-01-01T09:00:00Z",
        "source_agent_id": null,
        "source_chat_id": null,
    });
    let kind: TaskKind = serde_json::from_value(json).unwrap();
    match kind {
        TaskKind::Cron { mode, concurrency, process_result, .. } => {
            assert_eq!(mode, CronMode::Singleton);
            assert_eq!(concurrency, CronConcurrency::Replace);
            assert!(!process_result);
        }
        _ => panic!("expected Cron"),
    }
}

/// Regression test for the runtime error
/// "Failed to decode CronMode, no variants matched". Guards against the case
/// where a SurrealDB-persisted Cron row, written before mode/concurrency/process_result
/// were added, fails to roundtrip through the SurrealValue derive.
///
/// Drives the actual SurrealDB layer: writes a modern cron, then strips the
/// new fields via UPDATE to simulate a legacy row, then verifies the load
/// path falls back to defaults instead of erroring.
#[tokio::test]
async fn legacy_cron_row_loads_via_surrealdb_with_defaults() {
    let db = test_db().await;
    let s = svc(db.clone());

    let next = next_cron_occurrence("0 9 * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "Legacy cron", "no mode field",
            "0 9 * * *", "UTC".into(), next,
            None, None, None, None,
            CronMode::Singleton, CronConcurrency::Replace, false, None, None)
        .await
        .unwrap();

    // Strip the new fields to simulate a row written before this PR.
    db.query(
        r#"UPDATE type::record("task", $id) SET
            kind.mode = NONE,
            kind.concurrency = NONE,
            kind.process_result = NONE"#,
    )
    .bind(("id", template.id.clone()))
    .await
    .expect("legacy strip failed");

    // The bug was: this find_by_id would error with "Failed to decode CronMode".
    let loaded = s
        .find_by_id(&template.id)
        .await
        .expect("DB query failed — legacy row should decode with defaults")
        .expect("legacy task not found");

    match loaded.kind {
        TaskKind::Cron {
            mode,
            concurrency,
            process_result,
            cron_expression,
            ..
        } => {
            assert_eq!(cron_expression, "0 9 * * *");
            assert_eq!(mode, CronMode::Singleton, "missing mode defaults to Singleton");
            assert_eq!(
                concurrency,
                CronConcurrency::Replace,
                "missing concurrency defaults to Replace"
            );
            assert!(!process_result, "missing process_result defaults to false");
        }
        other => panic!("expected Cron kind, got {:?}", other),
    }
}

#[tokio::test]
async fn service_cancel_cascades_template_and_active_runs() {
    // Single TaskService::cancel call on a Cron template must mark both the
    // template and its in-flight CronRun children Cancelled.
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::PerInstance, CronConcurrency::Allow, false, None, None)
        .await
        .unwrap();
    let r1 = s.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();
    let r2 = s.spawn_cron_run(&template, Utc::now(), 2).await.unwrap();
    // r1 still Pending, r2 marked InProgress to cover both active states.
    s.mark_in_progress(&r2.id, Some("chat-x")).await.unwrap();

    let cancelled = s.cancel("user-1", &template.id).await.unwrap();
    assert_eq!(cancelled.status, TaskStatus::Cancelled);

    let r1_after = s.find_by_id(&r1.id).await.unwrap().unwrap();
    let r2_after = s.find_by_id(&r2.id).await.unwrap().unwrap();
    assert_eq!(r1_after.status, TaskStatus::Cancelled, "pending child cascaded");
    assert_eq!(r2_after.status, TaskStatus::Cancelled, "in-progress child cascaded");
}

#[tokio::test]
async fn service_cancel_is_idempotent_on_terminal_states() {
    // Calling cancel on a task that's already Cancelled/Completed/Failed
    // must succeed and return the task as-is (no 400, no state change).
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::Singleton, CronConcurrency::Replace, false, None, None)
        .await
        .unwrap();

    // 1. Cancelled → idempotent
    s.mark_cancelled(&template.id).await.unwrap();
    let again = s.cancel("user-1", &template.id).await.unwrap();
    assert_eq!(again.status, TaskStatus::Cancelled);

    // 2. Completed → idempotent
    let other = s.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();
    s.mark_completed(&other.id, Some("done".into())).await.unwrap();
    let returned = s.cancel("user-1", &other.id).await.unwrap();
    assert_eq!(returned.status, TaskStatus::Completed, "no status mutation");

    // 3. Failed → idempotent
    let other = s.spawn_cron_run(&template, Utc::now(), 2).await.unwrap();
    s.mark_failed(&other.id, "boom".into()).await.unwrap();
    let returned = s.cancel("user-1", &other.id).await.unwrap();
    assert_eq!(returned.status, TaskStatus::Failed);
}

#[tokio::test]
async fn service_delete_cascades_cron_template_to_runs() {
    // Deleting a Cron template must also delete every CronRun child row.
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::PerInstance, CronConcurrency::Forbid, false, None, None)
        .await
        .unwrap();
    let r1 = s.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();
    let r2 = s.spawn_cron_run(&template, Utc::now(), 2).await.unwrap();
    s.mark_completed(&r1.id, Some("done".into())).await.unwrap();
    s.mark_in_progress(&r2.id, Some("c")).await.unwrap();

    s.delete("user-1", &template.id).await.unwrap();

    assert!(s.find_by_id(&template.id).await.unwrap().is_none(), "template gone");
    assert!(s.find_by_id(&r1.id).await.unwrap().is_none(), "completed child gone");
    assert!(s.find_by_id(&r2.id).await.unwrap().is_none(), "in-progress child gone");
    // No orphan references via the runs query either.
    let leftover = s.find_runs_by_cron(&template.id).await.unwrap();
    assert!(leftover.is_empty());
}

#[tokio::test]
async fn service_delete_non_cron_does_not_touch_cron_runs() {
    // Sanity: deleting a non-cron task must NOT touch unrelated CronRun rows
    // (e.g. accidentally interpreting source_chat_id as a cron template id).
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::Singleton, CronConcurrency::Replace, false, None, None)
        .await
        .unwrap();
    let run = s.spawn_cron_run(&template, Utc::now(), 1).await.unwrap();

    use frona::agent::task::models::CreateTaskRequest;
    let direct = s
        .create(
            "user-1",
            CreateTaskRequest {
                agent_id: "agent-1".into(),
                space_id: None,
                chat_id: None,
                title: "unrelated".into(),
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

    s.delete("user-1", &direct.id).await.unwrap();

    // Cron template + run untouched.
    assert!(s.find_by_id(&template.id).await.unwrap().is_some());
    assert!(s.find_by_id(&run.id).await.unwrap().is_some());
}

#[tokio::test]
async fn service_delete_rejects_wrong_user() {
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::Singleton, CronConcurrency::Replace, false, None, None)
        .await
        .unwrap();
    let err = s.delete("user-2", &template.id).await.unwrap_err();
    assert!(matches!(err, frona::core::error::AppError::Forbidden(_)));
}

#[tokio::test]
async fn service_cancel_rejects_wrong_user() {
    // Authorization check survives the layering changes.
    let s = svc(test_db().await);
    let next = next_cron_occurrence("* * * * *", "UTC").unwrap();
    let template = s
        .create_cron_template(
            "user-1", "agent-1", "t", "d", "* * * * *", "UTC".into(),
            next, None, None, None, None,
            CronMode::Singleton, CronConcurrency::Replace, false, None, None)
        .await
        .unwrap();

    let err = s.cancel("user-2", &template.id).await.unwrap_err();
    assert!(
        matches!(err, frona::core::error::AppError::Forbidden(_)),
        "got {err:?}"
    );
}
