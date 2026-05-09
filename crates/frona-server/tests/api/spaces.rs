use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn create_space_returns_json() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "spaceuser", "spaceuser@example.com", "password123").await;
    let json = create_space(&state, &token, "MySpace").await;
    assert!(json["id"].is_string());
    assert_eq!(json["name"], "MySpace");
}

#[tokio::test]
async fn create_space_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/spaces")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "X"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_spaces_returns_only_own() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "space-a", "spacea@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "space-b", "spaceb@example.com", "password123").await;

    create_space(&state, &token_a, "SpaceA").await;
    create_space(&state, &token_b, "SpaceB").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/spaces", &token_a))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let spaces = json.as_array().unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0]["name"], "SpaceA");
}

#[tokio::test]
async fn update_space() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "upspace", "upspace@example.com", "password123").await;
    let space = create_space(&state, &token, "Before").await;
    let id = space["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/spaces/{id}"),
            &token,
            serde_json::json!({"name": "After"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["name"], "After");
}

#[tokio::test]
async fn delete_space() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "delspace", "delspace@example.com", "password123").await;
    let space = create_space(&state, &token, "GoAway").await;
    let id = space["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_delete(&format!("/api/spaces/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/spaces", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn create_space_with_metadata_round_trips() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "metauser", "metauser@example.com", "password123").await;
    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/spaces",
            &token,
            serde_json::json!({
                "name": "MD",
                "metadata": {
                    "channel:provider": "telegram",
                    "channel:status": "disconnected",
                },
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["metadata"]["channel:provider"], "telegram");
    assert_eq!(json["metadata"]["channel:status"], "disconnected");
}

#[tokio::test]
async fn update_space_metadata_partial_set_and_unset() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "patchmd", "patchmd@example.com", "password123").await;
    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/spaces",
            &token,
            serde_json::json!({
                "name": "MD",
                "metadata": {"a": 1, "b": 2},
            }),
        ))
        .await
        .unwrap();
    let space = body_json(resp).await;
    let id = space["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/spaces/{id}"),
            &token,
            serde_json::json!({
                "metadata": {"a": 99, "b": null, "c": "new"},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["metadata"]["a"], 99);
    assert!(json["metadata"].get("b").is_none());
    assert_eq!(json["metadata"]["c"], "new");
}

#[tokio::test]
async fn space_stream_filters_by_space_id() {
    use http_body_util::BodyExt;

    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "spsse", "spsse@example.com", "password123").await;
    let space_a = create_space(&state, &token, "A").await;
    let space_b = create_space(&state, &token, "B").await;
    let id_a = space_a["id"].as_str().unwrap();
    let id_b = space_b["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get(&format!("/api/spaces/{id_a}/stream"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().contains("event-stream"))
        .unwrap_or(false));

    let app = build_app(state.clone());
    let _ = app
        .oneshot(auth_put_json(
            &format!("/api/spaces/{id_a}"),
            &token,
            serde_json::json!({"metadata": {"channel:status": "connected"}}),
        ))
        .await
        .unwrap();
    let app = build_app(state);
    let _ = app
        .oneshot(auth_put_json(
            &format!("/api/spaces/{id_b}"),
            &token,
            serde_json::json!({"metadata": {"channel:status": "connected"}}),
        ))
        .await
        .unwrap();

    let mut body = resp.into_body();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(800);
    let mut text = String::new();
    while let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, body.frame()).await {
        if let Some(data) = frame.data_ref() {
            text.push_str(&String::from_utf8_lossy(data));
        }
    }

    assert!(text.contains("entity_updated"), "expected entity_updated frame, got:\n{text}");
    assert!(text.contains(id_a), "expected space A's id in stream, got:\n{text}");
    assert!(
        !text.contains(id_b),
        "Space B's id should not appear in Space A's stream:\n{text}"
    );
}

#[tokio::test]
async fn delete_space_other_user_returns_error() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "sp-owner", "spowner@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "sp-other", "spother@example.com", "password123").await;

    let space = create_space(&state, &token_a, "Mine").await;
    let id = space["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_delete(&format!("/api/spaces/{id}"), &token_b))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "Expected 404 or 403, got {}",
        resp.status()
    );
}
