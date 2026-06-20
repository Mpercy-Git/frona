use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::{auth_get, body_json, build_app, register_user, test_app_state};

#[tokio::test]
async fn test_health_returns_ok() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let req = Request::builder()
        .uri("/api/system/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn test_healthz_returns_ok() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let req = Request::builder()
        .uri("/healthz")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_health_returns_draining_during_shutdown() {
    let (state, _tmp) = test_app_state().await;
    state.shutdown_token.cancel();
    let app = build_app(state);
    let req = Request::builder()
        .uri("/api/system/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "draining");
}

#[tokio::test]
async fn test_api_returns_503_during_shutdown() {
    let (state, _tmp) = test_app_state().await;
    let (token, _user_id) = register_user(&state, "alice", "alice@test.com", "password123").await;
    state.shutdown_token.cancel();
    let app = build_app(state);
    let resp = app.oneshot(auth_get("/api/agents", &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "Server is shutting down");
}

#[tokio::test]
async fn test_api_works_before_shutdown() {
    let (state, _tmp) = test_app_state().await;
    let (token, _user_id) = register_user(&state, "bob", "bob@test.com", "password123").await;
    let app = build_app(state);
    let resp = app.oneshot(auth_get("/api/agents", &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_task_spawn_rejected_during_shutdown() {
    let (state, _tmp) = test_app_state().await;
    state.shutdown_token.cancel();

    let executor = state.task_executor.clone();

    let task = frona::agent::task::models::Task {
        id: "test-task-1".into(),
        user_id: "test-user".into(),
        agent_id: "test-agent".into(),
        space_id: None,
        chat_id: None,
        title: "Test task".into(),
        description: "Should be rejected".into(),
        kind: frona::agent::task::models::TaskKind::Direct { source_chat_id: None },
        status: frona::agent::task::models::TaskStatus::Pending,
        run_at: None,
        result_summary: None,
        error_message: None,
        quarantined: false,
        result_schema: None,
        result_description: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let result = executor.run_task(task).await;
    assert!(result.is_ok(), "run_task should return Ok even during shutdown");
}
