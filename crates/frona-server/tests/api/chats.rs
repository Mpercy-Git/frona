use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn create_chat_returns_json() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "chatuser", "chatuser@example.com", "password123").await;
    let agent = create_agent(&state, &token, "ChatAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let chat = create_chat(&state, &token, agent_id, Some("Hello")).await;
    assert!(chat["id"].is_string());
    assert_eq!(chat["agent_id"], agent_id);
    assert_eq!(chat["title"], "Hello");
}

#[tokio::test]
async fn chat_metadata_round_trip_and_partial_patch() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "cmd", "cmd@example.com", "password123").await;
    let agent = create_agent(&state, &token, "MetaAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/chats",
            &token,
            serde_json::json!({
                "agent_id": agent_id,
                "metadata": {"channel:external_id": "dm:42", "tag": "hi"},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let chat = body_json(resp).await;
    assert_eq!(chat["metadata"]["channel:external_id"], "dm:42");
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/chats/{id}"),
            &token,
            serde_json::json!({
                "metadata": {"channel:external_id": "dm:99", "tag": null},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["metadata"]["channel:external_id"], "dm:99");
    assert!(json["metadata"].get("tag").is_none());
}

#[tokio::test]
async fn create_chat_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chats")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"agent_id": "fake"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_chats_returns_only_own() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) = register_user(&state, "chat-a", "chata@example.com", "password123").await;
    let (token_b, _) = register_user(&state, "chat-b", "chatb@example.com", "password123").await;

    let agent_a = create_agent(&state, &token_a, "AgA").await;
    let agent_b = create_agent(&state, &token_b, "AgB").await;

    create_chat(&state, &token_a, agent_a["id"].as_str().unwrap(), None).await;
    create_chat(&state, &token_b, agent_b["id"].as_str().unwrap(), None).await;

    let app = build_app(state);
    let resp = app.oneshot(auth_get("/api/chats", &token_a)).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn get_chat_by_id() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "getchat", "getchat@example.com", "password123").await;
    let agent = create_agent(&state, &token, "GC").await;
    let chat = create_chat(
        &state,
        &token,
        agent["id"].as_str().unwrap(),
        Some("MyChat"),
    )
    .await;
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(&format!("/api/chats/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["title"], "MyChat");
}

#[tokio::test]
async fn get_chat_other_user_returns_error() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) = register_user(&state, "chatown", "chatown@example.com", "password123").await;
    let (token_b, _) = register_user(&state, "chatoth", "chatoth@example.com", "password123").await;

    let agent = create_agent(&state, &token_a, "CO").await;
    let chat = create_chat(&state, &token_a, agent["id"].as_str().unwrap(), None).await;
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(&format!("/api/chats/{id}"), &token_b))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "Expected 404 or 403, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn update_chat_title() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "upchat", "upchat@example.com", "password123").await;
    let agent = create_agent(&state, &token, "UC").await;
    let chat = create_chat(&state, &token, agent["id"].as_str().unwrap(), Some("Old")).await;
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/chats/{id}"),
            &token,
            serde_json::json!({"title": "New Title"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["title"], "New Title");
}

#[tokio::test]
async fn delete_chat() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "delchat", "delchat@example.com", "password123").await;
    let agent = create_agent(&state, &token, "DC").await;
    let chat = create_chat(&state, &token, agent["id"].as_str().unwrap(), None).await;
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_delete(&format!("/api/chats/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(&format!("/api/chats/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn archive_and_unarchive_chat() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "archuser", "archuser@example.com", "password123").await;
    let agent = create_agent(&state, &token, "AR").await;
    let chat = create_chat(&state, &token, agent["id"].as_str().unwrap(), Some("Arch")).await;
    let id = chat["id"].as_str().unwrap();

    // Archive
    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/chats/{id}/archive"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["archived_at"].is_string());

    // Should not appear in main list
    let app = build_app(state.clone());
    let resp = app.oneshot(auth_get("/api/chats", &token)).await.unwrap();
    let list = body_json(resp).await;
    assert!(
        list.as_array().unwrap().iter().all(|c| c["id"] != id),
        "Archived chat should not appear in main list"
    );

    // Should appear in archived list
    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/chats/archived", &token))
        .await
        .unwrap();
    let archived = body_json(resp).await;
    assert!(archived.as_array().unwrap().iter().any(|c| c["id"] == id));

    // Unarchive
    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/chats/{id}/unarchive"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["archived_at"].is_null());

    // Should be back in main list
    let app = build_app(state);
    let resp = app.oneshot(auth_get("/api/chats", &token)).await.unwrap();
    let list = body_json(resp).await;
    assert!(list.as_array().unwrap().iter().any(|c| c["id"] == id));
}

/// The nav's "what is the agent working on" snapshot. `ActiveSessions` is a
/// server-wide map with no notion of ownership, so the ownership filter here is
/// the thing standing between one user's chat ids and another user's browser.
#[tokio::test]
async fn chat_activity_reports_own_running_chats_only() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) = register_user(&state, "act-a", "acta@example.com", "password123").await;
    let (token_b, _) = register_user(&state, "act-b", "actb@example.com", "password123").await;
    let agent_a = create_agent(&state, &token_a, "ActA").await;
    let agent_b = create_agent(&state, &token_b, "ActB").await;

    let mine = create_chat(&state, &token_a, agent_a["id"].as_str().unwrap(), None).await;
    let mine_id = mine["id"].as_str().unwrap().to_string();
    let idle = create_chat(&state, &token_a, agent_a["id"].as_str().unwrap(), None).await;
    let idle_id = idle["id"].as_str().unwrap().to_string();
    let theirs = create_chat(&state, &token_b, agent_b["id"].as_str().unwrap(), None).await;
    let theirs_id = theirs["id"].as_str().unwrap().to_string();

    // Both users have a live turn.
    let _ = state.active_sessions.register(&mine_id).await;
    let _ = state.active_sessions.register(&theirs_id).await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/chats/activity", &token_a))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    let working: Vec<&str> = json["working"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(working, vec![mine_id.as_str()]);
    assert!(!working.contains(&theirs_id.as_str()));
    assert!(!working.contains(&idle_id.as_str()));
    assert!(json["waiting"].as_array().unwrap().is_empty());
}

/// A turn parked on a human is a different state from a turn being worked on,
/// and it is the one most likely to sit unnoticed — it has to survive a reload,
/// which means coming from the database rather than the live session map.
#[tokio::test]
async fn chat_activity_reports_chats_paused_on_a_human() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "act-p", "actp@example.com", "password123").await;
    let (other_token, _) = register_user(&state, "act-q", "actq@example.com", "password123").await;
    let agent = create_agent(&state, &token, "ActP").await;
    let agent_id = agent["id"].as_str().unwrap();

    let chat = create_chat(&state, &token, agent_id, None).await;
    let chat_id = chat["id"].as_str().unwrap().to_string();

    // Production flips Executing → Paused through `pause_agent_message` when
    // the loop hits a pending HITL; replicate that end state here.
    let msg = state
        .chat_service
        .create_executing_agent_message(&chat_id, agent_id)
        .await
        .unwrap();
    use frona::core::repository::Repository;
    let msg_repo: SurrealRepo<frona::chat::message::models::Message> =
        SurrealRepo::new(state.db.clone());
    let mut stored = msg_repo.find_by_id(&msg.id).await.unwrap().unwrap();
    stored.status = Some(frona::chat::message::models::MessageStatus::Paused);
    msg_repo.update(&stored).await.unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/chats/activity", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(
        json["waiting"].as_array().unwrap(),
        &vec![serde_json::json!(chat_id)]
    );
    assert!(json["working"].as_array().unwrap().is_empty());

    // The other user must not learn that this chat exists, let alone its state.
    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/chats/activity", &other_token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert!(json["waiting"].as_array().unwrap().is_empty());

    // A chat that is both paused and running counts as working — the more
    // urgent of the two, and never reported twice.
    let _ = state.active_sessions.register(&chat_id).await;
    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/chats/activity", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(
        json["working"].as_array().unwrap(),
        &vec![serde_json::json!(chat_id)]
    );
    assert!(json["waiting"].as_array().unwrap().is_empty());
}
