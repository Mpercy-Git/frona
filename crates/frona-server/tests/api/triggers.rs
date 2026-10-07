use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use tower::ServiceExt;

use super::*;

async fn mint_trigger_token(
    state: &AppState,
    token: &str,
    agent_id: &str,
) -> axum::http::Response<Body> {
    build_app(state.clone())
        .oneshot(auth_post_json(
            &format!("/api/agents/{agent_id}/trigger-tokens"),
            token,
            serde_json::json!({"name": "Doorstep"}),
        ))
        .await
        .unwrap()
}

fn tiny_jpeg_base64() -> String {
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb([90, 120, 200]));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
    base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
}

#[tokio::test]
async fn a_trigger_token_wakes_its_agent_with_the_snapshot_attached() {
    let (state, _tmp) = test_app_state().await;
    let (session, user_id) =
        register_user(&state, "doorowner", "doorowner@example.com", "password123").await;
    let agent = create_agent(&state, &session, "Front door").await;
    let agent_id = agent["id"].as_str().unwrap().to_string();

    let resp = mint_trigger_token(&state, &session, &agent_id).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let minted = body_json(resp).await;
    assert_eq!(minted["scopes"][0], "agent:trigger");
    let trigger_token = minted["token"].as_str().unwrap().to_string();

    let resp = build_app(state.clone())
        .oneshot(auth_post_json(
            &format!("/api/agents/{agent_id}/trigger"),
            &trigger_token,
            serde_json::json!({
                "message": "Doorbell rang at 14:02.",
                "title": "Doorbell 14:02",
                "images": [{"media_type": "image/jpeg", "data": tiny_jpeg_base64()}],
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = body_json(resp).await;
    let chat_id = body["chat_id"].as_str().unwrap();

    let chat = state
        .chat_service
        .find_chat(chat_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chat.user_id, user_id);
    assert_eq!(chat.agent_id, agent_id);
    assert_eq!(chat.title.as_deref(), Some("Doorbell 14:02"));
    let image = &chat.metadata[frona::chat::models::PUSH_IMAGE_METADATA_KEY];
    assert_eq!(image["owner"], format!("user:{user_id}"));
    let path = image["path"].as_str().unwrap();
    assert!(path.starts_with("triggers/"), "{path}");

    let messages = state
        .chat_service
        .get_stored_messages(chat_id)
        .await
        .unwrap();
    let opening = messages
        .iter()
        .find(|m| m.content == "Doorbell rang at 14:02.")
        .expect("the trigger message opens the chat");
    assert_eq!(opening.attachments.len(), 1);
    assert_eq!(opening.attachments[0].content_type, "image/jpeg");
    assert_eq!(opening.attachments[0].path, path);
}

#[tokio::test]
async fn a_trigger_token_opens_nothing_else() {
    let (state, _tmp) = test_app_state().await;
    let (session, _) = register_user(
        &state,
        "doorlocked",
        "doorlocked@example.com",
        "password123",
    )
    .await;
    let agent = create_agent(&state, &session, "Front door").await;
    let other = create_agent(&state, &session, "Finance").await;
    let agent_id = agent["id"].as_str().unwrap();
    let trigger_token =
        body_json(mint_trigger_token(&state, &session, agent_id).await).await["token"]
            .as_str()
            .unwrap()
            .to_string();

    // Not the account's other routes...
    let resp = build_app(state.clone())
        .oneshot(auth_get("/api/tasks", &trigger_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = build_app(state.clone())
        .oneshot(auth_get("/api/agents", &trigger_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // ...not another agent...
    let resp = build_app(state.clone())
        .oneshot(auth_post_json(
            &format!("/api/agents/{}/trigger", other["id"].as_str().unwrap()),
            &trigger_token,
            serde_json::json!({"message": "hello"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // ...and it can't mint more tokens.
    let resp = mint_trigger_token(&state, &trigger_token, agent_id).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn only_the_owner_mints_or_triggers() {
    let (state, _tmp) = test_app_state().await;
    let (owner, _) = register_user(&state, "doorown", "doorown@example.com", "password123").await;
    let (stranger, _) = register_user(
        &state,
        "doorstranger",
        "doorstranger@example.com",
        "password123",
    )
    .await;
    let agent = create_agent(&state, &owner, "Front door").await;
    let agent_id = agent["id"].as_str().unwrap();

    let resp = mint_trigger_token(&state, &stranger, agent_id).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let resp = build_app(state.clone())
        .oneshot(auth_post_json(
            &format!("/api/agents/{agent_id}/trigger"),
            &stranger,
            serde_json::json!({"message": "hello"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_bad_trigger_leaves_nothing_behind() {
    let (state, _tmp) = test_app_state().await;
    let (session, user_id) =
        register_user(&state, "doorbad", "doorbad@example.com", "password123").await;
    let agent = create_agent(&state, &session, "Front door").await;
    let agent_id = agent["id"].as_str().unwrap();

    for body in [
        serde_json::json!({"message": "   "}),
        serde_json::json!({"message": "rang", "images": [{"media_type": "image/tiff", "data": tiny_jpeg_base64()}]}),
        serde_json::json!({"message": "rang", "images": [{"media_type": "image/png", "data": "%%%"}]}),
    ] {
        let resp = build_app(state.clone())
            .oneshot(auth_post_json(
                &format!("/api/agents/{agent_id}/trigger"),
                &session,
                body,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
    let chats = state
        .chat_service
        .list_chats(&user_id)
        .await
        .unwrap_or_default();
    assert!(chats.is_empty(), "no chat is opened for a rejected trigger");
}

#[tokio::test]
async fn a_notification_answer_needs_a_valid_token_for_an_open_question() {
    let (state, _tmp) = test_app_state().await;
    let (session, user_id) =
        register_user(&state, "doorpush", "doorpush@example.com", "password123").await;
    let agent = create_agent(&state, &session, "Front door").await;
    let chat = create_chat(&state, &session, agent["id"].as_str().unwrap(), None).await;
    let chat_id = chat["id"].as_str().unwrap();

    let post = |token: String| {
        Request::builder()
            .method("POST")
            .uri("/api/push/actions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"token": token}).to_string()))
            .unwrap()
    };

    // A session token is not an answer token.
    let resp = build_app(state.clone())
        .oneshot(post(session.clone()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // A well-signed answer to a question that doesn't exist.
    let token = state
        .presign_service
        .sign_push_action(&user_id, chat_id, "no-such-call", "I'm coming", 60)
        .await
        .unwrap();
    let resp = build_app(state.clone()).oneshot(post(token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Signed for someone else's chat.
    let (_, stranger_id) = register_user(
        &state,
        "doorpushstranger",
        "doorpushstranger@example.com",
        "password123",
    )
    .await;
    let token = state
        .presign_service
        .sign_push_action(&stranger_id, chat_id, "no-such-call", "I'm coming", 60)
        .await
        .unwrap();
    let resp = build_app(state.clone()).oneshot(post(token)).await.unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "{}",
        resp.status()
    );
}

#[tokio::test]
async fn an_agent_lists_only_its_own_trigger_tokens_and_revoking_one_kills_it() {
    let (state, _tmp) = test_app_state().await;
    let (session, _) =
        register_user(&state, "doorlist", "doorlist@example.com", "password123").await;
    let (stranger, _) = register_user(
        &state,
        "doorliststranger",
        "doorliststranger@example.com",
        "password123",
    )
    .await;
    let agent = create_agent(&state, &session, "Front door").await;
    let other = create_agent(&state, &session, "Back door").await;
    let agent_id = agent["id"].as_str().unwrap();

    let first = body_json(mint_trigger_token(&state, &session, agent_id).await).await;
    mint_trigger_token(&state, &session, agent_id).await;
    mint_trigger_token(&state, &session, other["id"].as_str().unwrap()).await;

    let list = |token: String| {
        let state = state.clone();
        let uri = format!("/api/agents/{agent_id}/trigger-tokens");
        async move {
            build_app(state)
                .oneshot(auth_get(&uri, &token))
                .await
                .unwrap()
        }
    };

    let resp = list(session.clone()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let tokens = body_json(resp).await;
    let tokens = tokens.as_array().unwrap();
    assert_eq!(tokens.len(), 2, "the other agent's token is not listed");
    assert!(
        tokens.iter().all(|t| t.get("token").is_none()),
        "the secret is never listed"
    );

    let resp = list(stranger).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Revoke through the ordinary token route; the token stops working at once.
    let id = first["id"].as_str().unwrap();
    let resp = build_app(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/auth/tokens/{id}"))
                .header("authorization", format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        body_json(list(session.clone()).await)
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let resp = build_app(state.clone())
        .oneshot(auth_post_json(
            &format!("/api/agents/{agent_id}/trigger"),
            first["token"].as_str().unwrap(),
            serde_json::json!({"message": "hello"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
