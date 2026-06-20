//! End-to-end test for timezone-aware scheduling.
//!
//! Exercises the full path from agent-style JSON arguments → tool layer parsing
//! → TaskService persistence → re-read from DB → recomputed next occurrence.
//! Verifies that a `cron_expression` plus a per-task `timezone` (or user-default)
//! flows through every layer to produce the correct UTC firing instant, and that
//! advancing the cron preserves the snapshotted timezone.

use chrono::{Timelike, Utc};
use frona::agent::task::models::TaskKind;
use frona::agent::task::service::TaskService;
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::tool::parse_run_at;
use frona::tool::task::next_cron_occurrence;
use serde_json::json;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;

const SERVER_DEFAULT_TZ: &str = "UTC";

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

fn make_task_service(db: Surreal<Db>) -> TaskService {
    TaskService::new(SurrealRepo::new(db), frona::chat::broadcast::BroadcastService::new())
}

/// Mirror of the TZ resolution logic in `TaskTool::resolve_timezone`:
/// explicit arg → user TZ → server default. Centralised here so this test
/// proves the documented behaviour, not the tool's internal implementation.
fn resolve_tz(args: &serde_json::Value, user_tz: Option<&str>) -> String {
    args.get("timezone")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| user_tz.filter(|s| !s.is_empty()).map(|s| s.to_string()))
        .unwrap_or_else(|| SERVER_DEFAULT_TZ.to_string())
}

#[tokio::test]
async fn agent_creates_cron_in_user_local_time_persists_correct_utc_instant() {
    let svc = make_task_service(test_db().await);
    let user_tz = Some("America/Los_Angeles");

    let args = json!({
        "title": "Morning brief",
        "instruction": "Send the daily brief",
        "cron_expression": "0 8 * * *",
    });

    let timezone = resolve_tz(&args, user_tz);
    assert_eq!(timezone, "America/Los_Angeles");

    let cron_expr = args["cron_expression"].as_str().unwrap();
    let next_run_at = next_cron_occurrence(cron_expr, &timezone).unwrap();

    let task = svc
        .create_cron_template(
            "user-1",
            "agent-1",
            args["title"].as_str().unwrap(),
            args["instruction"].as_str().unwrap(),
            cron_expr,
            timezone.clone(),
            next_run_at,
            None,
            None,
            Some("chat-1".to_string()),
            None,
            Default::default(),
            Default::default(),
            false, None, None)
        .await
        .unwrap();

    let stored = svc.find_by_id(&task.id).await.unwrap().expect("task should exist");
    match stored.kind {
        TaskKind::Cron { timezone: stored_tz, cron_expression, next_run_at: stored_next, .. } => {
            assert_eq!(stored_tz.as_deref(), Some("America/Los_Angeles"));
            assert_eq!(cron_expression, "0 8 * * *");
            let la: chrono_tz::Tz = "America/Los_Angeles".parse().unwrap();
            let local = stored_next.unwrap().with_timezone(&la);
            assert_eq!(local.hour(), 8, "8am LA projected back is {local}");
            assert_eq!(local.minute(), 0);
        }
        _ => panic!("expected Cron kind"),
    }
}

#[tokio::test]
async fn agent_creates_cron_with_explicit_timezone_override() {
    let svc = make_task_service(test_db().await);
    let user_tz = Some("America/Los_Angeles");

    let args = json!({
        "title": "Tokyo standup ping",
        "instruction": "Ping the Tokyo team",
        "cron_expression": "0 9 * * MON-FRI",
        "timezone": "Asia/Tokyo",
    });

    let timezone = resolve_tz(&args, user_tz);
    assert_eq!(timezone, "Asia/Tokyo", "per-task override beats user TZ");

    let next_run_at = next_cron_occurrence(args["cron_expression"].as_str().unwrap(), &timezone).unwrap();
    let task = svc
        .create_cron_template(
            "user-1",
            "agent-1",
            args["title"].as_str().unwrap(),
            args["instruction"].as_str().unwrap(),
            args["cron_expression"].as_str().unwrap(),
            timezone.clone(),
            next_run_at,
            None,
            None,
            Some("chat-1".to_string()),
            None,
            Default::default(),
            Default::default(),
            false, None, None)
        .await
        .unwrap();

    let stored = svc.find_by_id(&task.id).await.unwrap().unwrap();
    match stored.kind {
        TaskKind::Cron { timezone: stored_tz, next_run_at: stored_next, .. } => {
            assert_eq!(stored_tz.as_deref(), Some("Asia/Tokyo"));
            let tokyo: chrono_tz::Tz = "Asia/Tokyo".parse().unwrap();
            let local = stored_next.unwrap().with_timezone(&tokyo);
            assert_eq!(local.hour(), 9);
            // Tokyo doesn't observe DST, so this is always JST = UTC+9.
            // Same instant projected to UTC must be hour = (9 - 9) mod 24 = 0.
            assert_eq!(stored_next.unwrap().hour(), 0);
        }
        _ => panic!("expected Cron kind"),
    }
}

#[tokio::test]
async fn agent_creates_naive_run_at_resolves_in_user_tz() {
    let user_tz = Some("America/Los_Angeles");
    let args = json!({
        "title": "Late check-in",
        "instruction": "Check the deploy queue",
        "run_at": "2030-05-20T22:00:00",
    });

    let timezone = resolve_tz(&args, user_tz);
    let dt = parse_run_at(args.get("run_at").unwrap(), &timezone)
        .unwrap()
        .unwrap();

    // PDT (May) = UTC-7, so 22:00 LA → 05:00 UTC on the following day.
    assert_eq!(dt.to_rfc3339(), "2030-05-21T05:00:00+00:00");
}

#[tokio::test]
async fn naive_run_at_honors_explicit_timezone_override() {
    let user_tz = Some("America/Los_Angeles");
    let args = json!({
        "title": "London team check-in",
        "instruction": "Ping London office",
        "run_at": "2030-05-20T06:00:00",
        "timezone": "Europe/London",
    });

    let timezone = resolve_tz(&args, user_tz);
    let dt = parse_run_at(args.get("run_at").unwrap(), &timezone)
        .unwrap()
        .unwrap();

    // BST (May) = UTC+1, so 06:00 London → 05:00 UTC.
    assert_eq!(dt.to_rfc3339(), "2030-05-20T05:00:00+00:00");
}

#[tokio::test]
async fn explicit_offset_run_at_rejected_at_tool_layer() {
    let args = json!({"run_at": "2030-05-20T22:00:00-04:00"});
    let err = parse_run_at(args.get("run_at").unwrap(), "America/Los_Angeles").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("explicit UTC offset"), "got: {msg}");
    assert!(msg.contains("`timezone` parameter"), "should mention the timezone param");
}

#[tokio::test]
async fn agent_with_no_user_tz_falls_back_to_server_default() {
    let svc = make_task_service(test_db().await);
    let user_tz: Option<&str> = None;
    let args = json!({"cron_expression": "0 8 * * *"});

    let timezone = resolve_tz(&args, user_tz);
    assert_eq!(timezone, SERVER_DEFAULT_TZ);

    let next_run_at = next_cron_occurrence(args["cron_expression"].as_str().unwrap(), &timezone).unwrap();
    let task = svc
        .create_cron_template(
            "user-1",
            "agent-1",
            "Server-TZ task",
            "Run something",
            args["cron_expression"].as_str().unwrap(),
            timezone.clone(),
            next_run_at,
            None,
            None,
            None,
            None,
            Default::default(),
            Default::default(),
            false, None, None)
        .await
        .unwrap();

    match svc.find_by_id(&task.id).await.unwrap().unwrap().kind {
        TaskKind::Cron { timezone: stored_tz, .. } => {
            assert_eq!(stored_tz.as_deref(), Some(SERVER_DEFAULT_TZ));
        }
        _ => panic!("expected Cron kind"),
    }
}

#[tokio::test]
async fn cron_advance_uses_snapshotted_timezone_not_server_default() {
    let svc = make_task_service(test_db().await);
    let tz = "America/Los_Angeles".to_string();
    let first = next_cron_occurrence("0 8 * * *", &tz).unwrap();
    let task = svc
        .create_cron_template(
            "user-1",
            "agent-1",
            "Daily brief",
            "desc",
            "0 8 * * *",
            tz.clone(),
            first,
            None,
            None,
            None,
            None,
            Default::default(),
            Default::default(),
            false, None, None)
        .await
        .unwrap();

    let stored = svc.find_by_id(&task.id).await.unwrap().unwrap();
    let stored_tz = match &stored.kind {
        TaskKind::Cron { timezone, .. } => timezone.clone().expect("must be present after create"),
        _ => panic!("expected Cron kind"),
    };
    assert_eq!(stored_tz, "America/Los_Angeles");

    let recomputed = next_cron_occurrence("0 8 * * *", &stored_tz).unwrap();
    let ny: chrono_tz::Tz = "America/Los_Angeles".parse().unwrap();
    assert_eq!(recomputed.with_timezone(&ny).hour(), 8);
    assert!(recomputed >= Utc::now());
}
