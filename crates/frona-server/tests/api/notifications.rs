use axum::body::Body;
use axum::http::{Request, StatusCode};
use frona::notification::models::{NotificationData, NotificationLevel};
use tower::ServiceExt;

use super::*;

async fn seed_notification(state: &AppState, user_id: &str, title: &str) {
    state
        .notification_service
        .create(
            user_id,
            NotificationData::System {},
            NotificationLevel::Info,
            title.to_string(),
            "test body".to_string(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn list_notifications_empty() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "notif-empty", "notifempty@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["notifications"].as_array().unwrap().len(), 0);
    assert_eq!(json["unread_count"], 0);
}

#[tokio::test]
async fn list_notifications_returns_created() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "notif-list", "notiflist@example.com", "password123").await;

    seed_notification(&state, &user_id, "First").await;
    seed_notification(&state, &user_id, "Second").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["notifications"].as_array().unwrap().len(), 2);
    assert_eq!(json["unread_count"], 2);
}

#[tokio::test]
async fn list_notifications_isolates_users() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, user_id_a) =
        register_user(&state, "notif-a", "notifa@example.com", "password123").await;
    let (token_b, user_id_b) =
        register_user(&state, "notif-b", "notifb@example.com", "password123").await;

    seed_notification(&state, &user_id_a, "For A").await;
    seed_notification(&state, &user_id_b, "For B").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/notifications", &token_a))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(json["notifications"][0]["title"], "For A");

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token_b))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(json["notifications"][0]["title"], "For B");
}

#[tokio::test]
async fn list_notifications_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/notifications")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mark_read_succeeds() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "notif-read", "notifread@example.com", "password123").await;

    seed_notification(&state, &user_id, "To read").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let notif_id = json["notifications"][0]["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/notifications/{notif_id}/read"),
            &token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["unread_count"], 0);
}

#[tokio::test]
async fn mark_read_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/notifications/fake-id/read")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mark_all_read_succeeds() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "notif-all", "notifall@example.com", "password123").await;

    seed_notification(&state, &user_id, "One").await;
    seed_notification(&state, &user_id, "Two").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/notifications/read-all",
            &token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["unread_count"], 0);
}

#[tokio::test]
async fn mark_all_read_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/notifications/read-all")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
