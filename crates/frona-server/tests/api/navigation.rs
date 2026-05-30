use axum::http::StatusCode;
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn get_navigation_empty() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "nav-empty", "navempty@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/navigation", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["spaces"].as_array().unwrap().len(), 0);
    assert_eq!(json["standalone_chats"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn get_navigation_with_standalone_chat() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "nav-chat", "navchat@example.com", "password123").await;
    let agent = create_agent(&state, &token, "NavAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    create_chat(&state, &token, agent_id, Some("Standalone")).await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/navigation", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["spaces"].as_array().unwrap().len(), 0);
    let standalone = json["standalone_chats"].as_array().unwrap();
    assert_eq!(standalone.len(), 1);
    assert_eq!(standalone[0]["title"], "Standalone");
}

#[tokio::test]
async fn get_navigation_with_space() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "nav-space", "navspace@example.com", "password123").await;

    create_space(&state, &token, "MySpace").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/navigation", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let spaces = json["spaces"].as_array().unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0]["name"], "MySpace");
    assert!(spaces[0]["chats"].is_array());
}

#[tokio::test]
async fn get_navigation_isolates_users() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "nav-a", "nava@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "nav-b", "navb@example.com", "password123").await;

    create_space(&state, &token_a, "SpaceA").await;
    create_space(&state, &token_b, "SpaceB").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/navigation", &token_a))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let spaces = json["spaces"].as_array().unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0]["name"], "SpaceA");
}
