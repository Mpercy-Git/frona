use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::super::*;

#[tokio::test]
async fn message_metadata_round_trip_via_send_and_patch() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mmd", "mmd@example.com", "password123").await;
    let agent = create_agent(&state, &token, "MdAgent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let chat = create_chat(&state, &token, agent_id, None).await;
    let chat_id = chat["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/chats/{chat_id}/messages"),
            &token,
            serde_json::json!({
                "content": "hello",
                "metadata": {"channel:external_msg_id": "tg:42"},
            }),
        ))
        .await
        .unwrap();
    let _ = resp.status();

    let app = build_app(state.clone());
    let list = app
        .oneshot(auth_get(
            &format!("/api/chats/{chat_id}/messages"),
            &token,
        ))
        .await
        .unwrap();
    let json = body_json(list).await;
    let messages = json["messages"].as_array().expect("messages array");
    let user_msg = messages
        .iter()
        .find(|m| m["role"] == "user")
        .expect("user message present");
    assert_eq!(user_msg["metadata"]["channel:external_msg_id"], "tg:42");
    let msg_id = user_msg["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_patch_json(
            &format!("/api/messages/{msg_id}"),
            &token,
            serde_json::json!({
                "metadata": {"channel:external_msg_id": "tg:99", "extra": "x"},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let patched = body_json(resp).await;
    assert_eq!(patched["metadata"]["channel:external_msg_id"], "tg:99");
    assert_eq!(patched["metadata"]["extra"], "x");
}


#[tokio::test]
async fn list_messages_empty_chat() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "msg-list", "msglist@example.com", "password123").await;
    let agent = create_agent(&state, &token, "ListAgent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let chat = create_chat(&state, &token, agent_id, Some("ListChat")).await;
    let chat_id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/chats/{chat_id}/messages"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["messages"].is_array());
    assert_eq!(json["has_more"], false);
}

#[tokio::test]
async fn list_messages_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/chats/fake-id/messages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_messages_other_user_returns_error() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "msg-own", "msgown@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "msg-oth", "msgoth@example.com", "password123").await;

    let agent = create_agent(&state, &token_a, "MsgOwn").await;
    let chat = create_chat(&state, &token_a, agent["id"].as_str().unwrap(), None).await;
    let chat_id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/chats/{chat_id}/messages"),
            &token_b,
        ))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND,
        "Expected 403 or 404, got {}",
        resp.status()
    );
}


#[tokio::test]
async fn cancel_generation_returns_json() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "msg-cancel", "msgcancel@example.com", "password123").await;
    let agent = create_agent(&state, &token, "CancelAgent").await;
    let chat = create_chat(&state, &token, agent["id"].as_str().unwrap(), None).await;
    let chat_id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/chats/{chat_id}/cancel"),
            &token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["cancelled"], false);
}

#[tokio::test]
async fn cancel_generation_other_user_returns_error() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "cancel-own", "cancelown@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "cancel-oth", "canceloth@example.com", "password123").await;

    let agent = create_agent(&state, &token_a, "CancelOwn").await;
    let chat = create_chat(&state, &token_a, agent["id"].as_str().unwrap(), None).await;
    let chat_id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/chats/{chat_id}/cancel"),
            &token_b,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND,
        "Expected 403 or 404, got {}",
        resp.status()
    );
}


#[tokio::test]
async fn resolve_tool_call_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chats/fake-id/tool-calls/resolve")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"resolutions": [{"tool_call_id": "te-1", "response": "yes"}]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn resolve_tool_call_other_user_returns_error() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "resolve-own", "resolveown@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "resolve-oth", "resolveoth@example.com", "password123").await;

    let agent = create_agent(&state, &token_a, "ResolveAgent").await;
    let chat = create_chat(&state, &token_a, agent["id"].as_str().unwrap(), None).await;
    let chat_id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/chats/{chat_id}/tool-calls/resolve"),
            &token_b,
            serde_json::json!({"resolutions": [{"tool_call_id": "fake-te", "response": "yes"}]}),
        ))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND,
        "Expected 403 or 404, got {}",
        resp.status()
    );
}


#[tokio::test]
async fn send_message_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chats/fake-id/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"content": "hello"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

