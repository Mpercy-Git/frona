//! Integration tests for timezone-aware scheduling.
//!
//! Exercises `TaskService::create_cron_template` and `Scheduler::advance_cron_template`
//! across multiple timezones and DST boundaries, verifying that the stored
//! `next_run_at` UTC instant matches the user-local wall-clock intent.

use chrono::{Timelike, Utc};
use frona::agent::task::models::{TaskKind, TaskStatus};
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

fn make_task_service(db: Surreal<Db>) -> TaskService {
    TaskService::new(SurrealRepo::new(db), frona::chat::broadcast::BroadcastService::new())
}

#[tokio::test]
async fn create_cron_template_snapshots_timezone() {
    let svc = make_task_service(test_db().await);
    let next = next_cron_occurrence("0 8 * * *", "America/Los_Angeles").unwrap();
    let task = svc
        .create_cron_template(
            "user-1",
            "agent-1",
            "Morning brief",
            "Send the daily brief",
            "0 8 * * *",
            "America/Los_Angeles".to_string(),
            next,
            None,
            None,
            None,
            None,
            Default::default(),
            Default::default(),
            false, None)
        .await
        .unwrap();

    match task.kind {
        TaskKind::Cron { timezone, next_run_at, .. } => {
            assert_eq!(timezone.as_deref(), Some("America/Los_Angeles"));
            assert_eq!(next_run_at, Some(next));
        }
        _ => panic!("expected Cron kind"),
    }
}

#[tokio::test]
async fn cron_in_la_resolves_to_correct_utc_hour() {
    // 8am LA: 16:00 UTC in winter (PST), 15:00 UTC in summer (PDT). Verify by
    // projecting the resulting UTC instant back into LA clock — it must read 08:00.
    let next = next_cron_occurrence("0 8 * * *", "America/Los_Angeles").unwrap();
    let la: chrono_tz::Tz = "America/Los_Angeles".parse().unwrap();
    let local = next.with_timezone(&la);
    assert_eq!(local.hour(), 8, "next 8am LA projected back is {local}");
    assert_eq!(local.minute(), 0);
    assert!(next > Utc::now());
}

#[tokio::test]
async fn cron_in_tokyo_resolves_correctly() {
    let next = next_cron_occurrence("0 8 * * *", "Asia/Tokyo").unwrap();
    let tokyo: chrono_tz::Tz = "Asia/Tokyo".parse().unwrap();
    let local = next.with_timezone(&tokyo);
    assert_eq!(local.hour(), 8);
    assert_eq!(local.minute(), 0);
}

#[tokio::test]
async fn cron_invalid_timezone_rejected() {
    let err = next_cron_occurrence("0 8 * * *", "Mars/Olympus_Mons").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid timezone"), "got: {msg}");
    assert!(msg.contains("IANA"), "expected IANA hint, got: {msg}");
}

#[tokio::test]
async fn cron_utc_legacy_behavior_preserved() {
    // Tasks stored before this change have no timezone field; the scheduler
    // falls back to the server default. With server default = "UTC", the
    // computed next instant matches the pre-fix UTC-only semantics.
    let next = next_cron_occurrence("0 8 * * *", "UTC").unwrap();
    let utc_now = Utc::now();
    assert!(next > utc_now);
    assert_eq!(next.hour(), 8, "8am UTC interpreted as UTC stays 8 UTC");
}

#[tokio::test]
async fn advance_cron_template_preserves_kind_and_timezone() {
    let svc = make_task_service(test_db().await);
    let first = next_cron_occurrence("0 9 * * *", "America/Los_Angeles").unwrap();
    let task = svc
        .create_cron_template(
            "user-1",
            "agent-1",
            "Morning poll",
            "Poll the morning queue",
            "0 9 * * *",
            "America/Los_Angeles".to_string(),
            first,
            None,
            None,
            None,
            None,
            Default::default(),
            Default::default(),
            false, None)
        .await
        .unwrap();

    let second = next_cron_occurrence("0 9 * * *", "America/Los_Angeles")
        .map(|d| d + chrono::Duration::days(1))
        .unwrap();
    let advanced = svc
        .advance_cron_template(&task.id, second)
        .await
        .unwrap();

    match advanced.kind {
        TaskKind::Cron { timezone, next_run_at, .. } => {
            assert_eq!(
                timezone.as_deref(),
                Some("America/Los_Angeles"),
                "timezone snapshot must survive advance"
            );
            assert_eq!(next_run_at, Some(second));
        }
        _ => panic!("expected Cron kind"),
    }
    assert_eq!(advanced.status, TaskStatus::Pending);
}

#[tokio::test]
async fn cron_legacy_row_without_timezone_field_deserializes() {
    // Simulate a pre-migration DB row that lacks the `timezone` field on
    // TaskKind::Cron. The #[serde(default)] attribute should yield None,
    // and the scheduler later resolves that to the server default.
    let json = serde_json::json!({
        "type": "Cron",
        "cron_expression": "0 9 * * *",
        "next_run_at": "2030-01-01T09:00:00Z",
        "source_agent_id": null,
        "source_chat_id": null,
    });
    let kind: TaskKind = serde_json::from_value(json).unwrap();
    match kind {
        TaskKind::Cron { timezone, cron_expression, .. } => {
            assert!(timezone.is_none(), "legacy rows deserialize with timezone=None");
            assert_eq!(cron_expression, "0 9 * * *");
        }
        _ => panic!("expected Cron kind"),
    }
}
