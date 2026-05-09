use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn manifests_endpoint_lists_telegram() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mfest", "mfest@example.com", "password123").await;
    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/channels/manifests", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let manifests = body_json(resp).await;
    let arr = manifests.as_array().expect("manifests array");
    let telegram = arr
        .iter()
        .find(|m| m["id"] == "telegram")
        .expect("Telegram manifest registered at startup");
    assert_eq!(telegram["display_name"], "Telegram Bot");
    let fields = telegram["config_fields"]
        .as_array()
        .expect("config_fields array");
    assert!(
        fields.iter().any(|f| f["name"] == "bot_token"),
        "manifest must declare a bot_token field"
    );
}

#[tokio::test]
async fn telegram_webhook_creates_entities_with_metadata() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "tgwh", "tgwh@example.com", "password123").await;
    let agent = create_agent(&state, &token, "TgAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json("/api/spaces", &token, serde_json::json!({"name": "Telegram"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let space = body_json(resp).await;
    let space_id = space["id"].as_str().unwrap();

    let now = chrono::Utc::now();
    let channel = frona::chat::channel::Channel {
        id: format!("channel:{}", uuid::Uuid::new_v4()),
        user_id: user_id.clone(),
        space_id: space_id.to_string(),
        provider: "telegram".into(),
        agent_id: agent_id.to_string(),
        config: {
            let mut m = std::collections::BTreeMap::new();
            m.insert("bot_token".into(), "fake-bot-token-for-test".into());
            m
        },
        dispatch_mode: frona::chat::channel::DispatchMode::Message,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: Some(frona::chat::channel::models::UserAddress {
            address: Some("@alice".into()),
            pairing_code: None,
            pairing_initiated_at: None,
            paired_at: Some(now),
        }),
        created_at: now,
        updated_at: now,
    };
    use frona::core::repository::Repository;
    let channel = frona::db::repo::generic::SurrealRepo::<frona::chat::channel::Channel>::new(
        state.db.clone(),
    )
    .create(&channel)
    .await
    .unwrap();
    let channel_id = channel.id.as_str();
    state
        .channel_manager
        .start_channel(&state, &channel)
        .await
        .unwrap();

    let payload = serde_json::json!({
        "update_id": 1001,
        "message": {
            "message_id": 42,
            "chat": {"id": 12345, "type": "private"},
            "from": {
                "id": 12345,
                "first_name": "Alice",
                "username": "alice"
            },
            "text": "hello"
        }
    });
    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/webhooks/channels/telegram/{}",
                    channel_id.strip_prefix("channel:").unwrap_or(channel_id),
                ))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get(
            &format!("/api/spaces/{space_id}/chats"),
            &token,
        ))
        .await
        .unwrap();
    let chats_json = if resp.status() == StatusCode::OK {
        body_json(resp).await
    } else {
        let app = build_app(state.clone());
        let resp = app.oneshot(auth_get("/api/chats", &token)).await.unwrap();
        body_json(resp).await
    };
    let chats = chats_json.as_array().expect("chats array");
    let chat = chats
        .iter()
        .find(|c| c["channel_external_id"] == "dm:12345")
        .expect("chat with channel_external_id present");
    assert_eq!(chat["agent_id"], agent_id);
    assert!(chat["channel_id"].is_string(), "channel_id should be set on channel-bound chat");

    let chat_id = chat["id"].as_str().unwrap();
    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/chats/{chat_id}/messages"),
            &token,
        ))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let messages = json["messages"].as_array().expect("messages array");
    let user_msg = messages
        .iter()
        .find(|m| m["role"] == "user")
        .expect("user message persisted");
    assert_eq!(user_msg["content"], "hello");
    assert_eq!(user_msg["external_msg_id"], "42");
}

#[tokio::test]
async fn telegram_webhook_persists_when_channel_is_signal_mode() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "tgto", "tgto@example.com", "password123").await;
    let agent = create_agent(&state, &token, "TgSignalAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json("/api/spaces", &token, serde_json::json!({"name": "Telegram"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let space = body_json(resp).await;
    let space_id = space["id"].as_str().unwrap();

    let now = chrono::Utc::now();
    let channel = frona::chat::channel::Channel {
        id: format!("channel:{}", uuid::Uuid::new_v4()),
        user_id: user_id.clone(),
        space_id: space_id.to_string(),
        provider: "telegram".into(),
        agent_id: agent_id.to_string(),
        config: {
            let mut m = std::collections::BTreeMap::new();
            m.insert("bot_token".into(), "fake-bot-token-for-test".into());
            m
        },
        dispatch_mode: frona::chat::channel::DispatchMode::Signal,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: None,
        created_at: now,
        updated_at: now,
    };
    use frona::core::repository::Repository;
    let channel = frona::db::repo::generic::SurrealRepo::<frona::chat::channel::Channel>::new(
        state.db.clone(),
    )
    .create(&channel)
    .await
    .unwrap();
    let channel_id = channel.id.as_str();
    state
        .channel_manager
        .start_channel(&state, &channel)
        .await
        .unwrap();

    let payload = serde_json::json!({
        "update_id": 7001,
        "message": {
            "message_id": 77,
            "chat": {"id": 77777, "type": "private"},
            "from": {
                "id": 77777,
                "first_name": "Bank",
                "username": "bank2fa"
            },
            "text": "Your code is 482193"
        }
    });
    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/webhooks/channels/telegram/{}",
                    channel_id.strip_prefix("channel:").unwrap_or(channel_id),
                ))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state.clone());
    let resp = app.oneshot(auth_get("/api/chats", &token)).await.unwrap();
    let chats = body_json(resp).await;
    let chat = chats
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["channel_external_id"] == "dm:77777")
        .expect("chat should exist for signal-mode inbound");
    let chat_id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(&format!("/api/chats/{chat_id}/messages"), &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let messages = json["messages"].as_array().expect("messages array");
    let user_msg = messages
        .iter()
        .find(|m| m["role"] == "user")
        .expect("inbound message should persist (receive_signal allowed by default)");
    assert_eq!(user_msg["content"], "Your code is 482193");
}

#[tokio::test]
async fn telegram_webhook_drops_inbound_when_receive_message_forbidden() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "tgblk", "tgblk@example.com", "password123").await;
    let agent = create_agent(&state, &token, "TgBlockedAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    state
        .policy_service
        .create_policy(
            &user_id,
            "@id(\"block-tg-spam-msg\")\nforbid(\n  principal,\n  action == Policy::Action::\"receive_message\",\n  resource in Policy::Channel::\"telegram\"\n)\nwhen { resource.sender.address == \"@spammer\" };",
        )
        .await
        .unwrap();
    // Both gates must deny for a true discard. receive_signal default-permits,
    // so we explicitly forbid it for the same source — otherwise the message
    // would fall through to signal mode and persist.
    state
        .policy_service
        .create_policy(
            &user_id,
            "@id(\"block-tg-spam-signal\")\nforbid(\n  principal,\n  action == Policy::Action::\"receive_signal\",\n  resource in Policy::Channel::\"telegram\"\n)\nwhen { resource.sender.address == \"@spammer\" };",
        )
        .await
        .unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json("/api/spaces", &token, serde_json::json!({"name": "Telegram"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let space = body_json(resp).await;
    let space_id = space["id"].as_str().unwrap();

    let now = chrono::Utc::now();
    let channel = frona::chat::channel::Channel {
        id: format!("channel:{}", uuid::Uuid::new_v4()),
        user_id: user_id.clone(),
        space_id: space_id.to_string(),
        provider: "telegram".into(),
        agent_id: agent_id.to_string(),
        config: {
            let mut m = std::collections::BTreeMap::new();
            m.insert("bot_token".into(), "fake-bot-token-for-test".into());
            m
        },
        dispatch_mode: frona::chat::channel::DispatchMode::Message,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: None,
        created_at: now,
        updated_at: now,
    };
    use frona::core::repository::Repository;
    let channel = frona::db::repo::generic::SurrealRepo::<frona::chat::channel::Channel>::new(
        state.db.clone(),
    )
    .create(&channel)
    .await
    .unwrap();
    let channel_id = channel.id.as_str();
    state
        .channel_manager
        .start_channel(&state, &channel)
        .await
        .unwrap();

    let payload = serde_json::json!({
        "update_id": 9001,
        "message": {
            "message_id": 99,
            "chat": {"id": 99999, "type": "private"},
            "from": {
                "id": 99999,
                "first_name": "Spam",
                "username": "spammer"
            },
            "text": "buy crypto"
        }
    });
    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/webhooks/channels/telegram/{}",
                    channel_id.strip_prefix("channel:").unwrap_or(channel_id),
                ))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state.clone());
    let resp = app.oneshot(auth_get("/api/chats", &token)).await.unwrap();
    let chats = body_json(resp).await;
    let chat = chats
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["metadata"]["channel:external_id"] == "dm:99999");
    if let Some(chat) = chat {
        let chat_id = chat["id"].as_str().unwrap();
        let app = build_app(state);
        let resp = app
            .oneshot(auth_get(
                &format!("/api/chats/{chat_id}/messages"),
                &token,
            ))
            .await
            .unwrap();
        let json = body_json(resp).await;
        let messages = json["messages"].as_array().expect("messages array");
        let dropped_msg_present = messages.iter().any(|m| m["content"] == "buy crypto");
        assert!(
            !dropped_msg_present,
            "Forbidden inbound message must NOT be persisted",
        );
    }
}

#[tokio::test]
async fn pairing_round_trip_flips_channel_to_connected() {
    use frona::core::repository::Repository;

    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "pair", "pair@example.com", "password123").await;
    let agent = create_agent(&state, &token, "PairAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/spaces", &token, serde_json::json!({"name": "Pair Space"})))
        .await.unwrap();
    let space_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let channel_id = format!("channel:{}", uuid::Uuid::new_v4());
    let channel = frona::chat::channel::Channel {
        id: channel_id.clone(),
        user_id: user_id.clone(),
        space_id: space_id.clone(),
        provider: "telegram".into(),
        agent_id: agent_id.into(),
        config: {
            let mut m = std::collections::BTreeMap::new();
            m.insert("bot_token".into(), "fake-bot-token".into());
            m
        },
        dispatch_mode: frona::chat::channel::DispatchMode::Message,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: None,
        created_at: now,
        updated_at: now,
    };
    frona::db::repo::generic::SurrealRepo::<frona::chat::channel::Channel>::new(
        state.db.clone()).create(&channel).await.unwrap();
    state.channel_manager.start_channel(&state, &channel).await.unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/channels/{channel_id}/pair"), &token, serde_json::json!({})))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let code = body["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 6, "code should be 6 chars: {code}");

    let mid = state.channel_service.find_owned(&user_id, &channel_id).await.unwrap();
    assert_eq!(format!("{:?}", mid.status), "Pairing");
    assert_eq!(
        mid.user_address.as_ref().and_then(|ua| ua.pairing_code.as_deref()),
        Some(code.as_str()),
    );
    assert!(mid.user_address.as_ref().and_then(|ua| ua.address.as_deref()).is_none());

    let payload = serde_json::json!({
        "update_id": 42,
        "message": {
            "message_id": 1,
            "chat": {"id": 555, "type": "private"},
            "from": {"id": 555, "first_name": "Op", "username": "operator"},
            "text": code,
        }
    });
    let app = build_app(state.clone());
    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri(format!(
                "/api/webhooks/channels/telegram/{}",
                channel_id.strip_prefix("channel:").unwrap_or(&channel_id),
            ))
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string())).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Pipeline runs async (mpsc → process_inbound). Poll until the
    // redemption shows up in DB (max 2s).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut after = state.channel_service.find_owned(&user_id, &channel_id).await.unwrap();
    while tokio::time::Instant::now() < deadline
        && format!("{:?}", after.status) != "Connected"
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        after = state.channel_service.find_owned(&user_id, &channel_id).await.unwrap();
    }
    assert_eq!(format!("{:?}", after.status), "Connected");
    let ua = after.user_address.as_ref().expect("user_address set");
    assert_eq!(ua.address.as_deref(), Some("@operator"));
    assert!(ua.pairing_code.is_none());
    assert!(ua.pairing_initiated_at.is_none());
    assert!(ua.paired_at.is_some());
}

#[tokio::test]
async fn pairing_cancel_reverts_to_disconnected() {
    use frona::core::repository::Repository;

    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "pcancel", "pcancel@example.com", "password123").await;
    let agent = create_agent(&state, &token, "CancelAgent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/spaces", &token, serde_json::json!({"name": "Cancel Space"})))
        .await.unwrap();
    let space_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let channel_id = format!("channel:{}", uuid::Uuid::new_v4());
    let channel = frona::chat::channel::Channel {
        id: channel_id.clone(),
        user_id: user_id.clone(),
        space_id,
        provider: "telegram".into(),
        agent_id: agent_id.into(),
        config: Default::default(),
        dispatch_mode: frona::chat::channel::DispatchMode::Message,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: None,
        created_at: now,
        updated_at: now,
    };
    frona::db::repo::generic::SurrealRepo::<frona::chat::channel::Channel>::new(
        state.db.clone()).create(&channel).await.unwrap();

    state.channel_service.initiate_pairing(&user_id, &channel_id).await.unwrap();
    state.channel_service.cancel_pairing(&user_id, &channel_id).await.unwrap();

    let after = state.channel_service.find_owned(&user_id, &channel_id).await.unwrap();
    assert_eq!(format!("{:?}", after.status), "Disconnected");
    assert!(after.user_address.is_none(), "no prior address → cleared");
}

#[tokio::test]
async fn restart_clears_orphaned_pairing() {
    use frona::core::repository::Repository;

    let (state, _tmp) = test_app_state().await;
    let (_token, user_id) =
        register_user(&state, "rstart", "rstart@example.com", "password123").await;

    let now = chrono::Utc::now();
    let channel_id = format!("channel:{}", uuid::Uuid::new_v4());
    let channel = frona::chat::channel::Channel {
        id: channel_id.clone(),
        user_id: user_id.clone(),
        space_id: "space-x".into(),
        provider: "telegram".into(),
        agent_id: "agent-x".into(),
        config: Default::default(),
        dispatch_mode: frona::chat::channel::DispatchMode::Message,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: None,
        created_at: now,
        updated_at: now,
    };
    frona::db::repo::generic::SurrealRepo::<frona::chat::channel::Channel>::new(
        state.db.clone()).create(&channel).await.unwrap();
    state.channel_service.initiate_pairing(&user_id, &channel_id).await.unwrap();

    let count = state.channel_service.revert_orphaned_pairings().await.unwrap();
    assert_eq!(count, 1);

    let after = state.channel_service.find_owned(&user_id, &channel_id).await.unwrap();
    assert_eq!(format!("{:?}", after.status), "Disconnected");
    assert!(after.user_address.is_none());
}

// This is the seam where the SMS-not-sent bug lived (broadcast missing on
// completion); regressions here would silently re-break outbound delivery.

use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct CapturedSend {
    msg_id: String,
    chat_id: String,
    content: String,
}

struct StubAdapter {
    captured: std::sync::Arc<StdMutex<Vec<CapturedSend>>>,
}

#[async_trait::async_trait]
impl frona::chat::channel::ChannelAdapter for StubAdapter {
    async fn on_connect(
        &self,
        _ctx: &frona::chat::channel::ChannelCtx,
    ) -> Result<(), frona::core::error::AppError> {
        Ok(())
    }
    async fn on_disconnect(
        &self,
        _ctx: &frona::chat::channel::ChannelCtx,
    ) -> Result<(), frona::core::error::AppError> {
        Ok(())
    }
    async fn on_send(
        &self,
        msg: &frona::chat::message::models::Message,
        chat: &frona::chat::models::Chat,
        _ctx: &frona::chat::channel::ChannelCtx,
    ) -> Result<String, frona::core::error::AppError> {
        let sid = format!("ext-{}", msg.id);
        self.captured.lock().unwrap().push(CapturedSend {
            msg_id: msg.id.clone(),
            chat_id: chat.id.clone(),
            content: msg.content.clone(),
        });
        Ok(sid)
    }
    async fn on_webhook(
        &self,
        ctx: &frona::chat::channel::ChannelCtx,
        request: axum::http::Request<axum::body::Bytes>,
    ) -> Result<axum::response::Response, frona::core::error::AppError> {
        let params: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(request.body())
                .into_owned()
                .collect();
        let from = params.get("from").cloned().unwrap_or_default();
        let text = params.get("text").cloned().unwrap_or_default();
        let event = frona::chat::channel::models::ExternalMessage {
            external_chat_id: format!("test:{from}"),
            external_msg_id: Some("ext-in-1".into()),
            sender_address: from.clone(),
            sender_external_id: Some(from.clone()),
            sender_display_name: Some(from),
            content: text,
        };
        ctx.emit
            .send(event)
            .await
            .map_err(|e| frona::core::error::AppError::Internal(format!("emit: {e}")))?;
        use axum::response::IntoResponse;
        Ok((axum::http::StatusCode::OK, "ok").into_response())
    }
}

struct StubFactory {
    captured: std::sync::Arc<StdMutex<Vec<CapturedSend>>>,
}

impl frona::chat::channel::ChannelFactory for StubFactory {
    fn manifest(&self) -> frona::chat::channel::ChannelManifest {
        frona::chat::channel::ChannelManifest {
            id: "test".into(),
            display_name: "Test".into(),
            description: "stub for e2e tests".into(),
            config_fields: vec![],
        }
    }
    fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn frona::chat::channel::ChannelAdapter>, frona::core::error::AppError> {
        Ok(Box::new(StubAdapter {
            captured: self.captured.clone(),
        }))
    }
}

async fn poll_until<F, Fut>(label: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("timeout waiting for: {label}");
}

#[tokio::test]
async fn inbound_webhook_persists_message_via_stub_adapter() {
    use frona::core::repository::Repository;

    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "e2ein", "e2ein@example.com", "password123").await;
    let agent = create_agent(&state, &token, "E2eAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let captured = std::sync::Arc::new(StdMutex::new(Vec::<CapturedSend>::new()));
    state
        .channel_registry
        .register_factory(std::sync::Arc::new(StubFactory {
            captured: captured.clone(),
        }));

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/spaces",
            &token,
            serde_json::json!({"name": "E2E"}),
        ))
        .await
        .unwrap();
    let space_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let channel = frona::chat::channel::Channel {
        id: format!("channel:{}", uuid::Uuid::new_v4()),
        user_id: user_id.clone(),
        space_id: space_id.clone(),
        provider: "test".into(),
        agent_id: agent_id.into(),
        config: Default::default(),
        dispatch_mode: frona::chat::channel::DispatchMode::Message,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: None,
        created_at: now,
        updated_at: now,
    };
    SurrealRepo::<frona::chat::channel::Channel>::new(state.db.clone())
        .create(&channel)
        .await
        .unwrap();
    state
        .channel_manager
        .start_channel(&state, &channel)
        .await
        .unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/webhooks/channels/test/{}",
                    channel.id.strip_prefix("channel:").unwrap_or(&channel.id),
                ))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("from=%2B15551234567&text=hello"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // process_inbound runs async (mpsc → pipeline). Poll the user's chats.
    let svc = state.chat_service.clone();
    let user_for_poll = user_id.clone();
    poll_until("chat upserted", || {
        let svc = svc.clone();
        let uid = user_for_poll.clone();
        async move {
            svc.list_chats(&uid)
                .await
                .ok()
                .map(|chats| {
                    chats
                        .iter()
                        .any(|c| c.channel_external_id.as_deref() == Some("test:+15551234567"))
                })
                .unwrap_or(false)
        }
    })
    .await;
}

#[tokio::test]
async fn agent_message_completion_dispatches_to_outbound_adapter() {
    use frona::core::repository::Repository;

    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "e2eout", "e2eout@example.com", "password123").await;
    let agent = create_agent(&state, &token, "E2eOutAgent").await;
    let agent_id = agent["id"].as_str().unwrap().to_string();

    let captured = std::sync::Arc::new(StdMutex::new(Vec::<CapturedSend>::new()));
    state
        .channel_registry
        .register_factory(std::sync::Arc::new(StubFactory {
            captured: captured.clone(),
        }));

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/spaces",
            &token,
            serde_json::json!({"name": "E2E Out"}),
        ))
        .await
        .unwrap();
    let space_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let channel = frona::chat::channel::Channel {
        id: format!("channel:{}", uuid::Uuid::new_v4()),
        user_id: user_id.clone(),
        space_id: space_id.clone(),
        provider: "test".into(),
        agent_id: agent_id.clone(),
        config: Default::default(),
        dispatch_mode: frona::chat::channel::DispatchMode::Message,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: None,
        created_at: now,
        updated_at: now,
    };
    SurrealRepo::<frona::chat::channel::Channel>::new(state.db.clone())
        .create(&channel)
        .await
        .unwrap();
    state
        .channel_manager
        .start_channel(&state, &channel)
        .await
        .unwrap();

    let chat = state
        .chat_service
        .upsert_channel_chat(
            &user_id,
            &space_id,
            &agent_id,
            &channel.id,
            "test:+15551234567",
            None,
        )
        .await
        .unwrap();

    let executing = state
        .chat_service
        .create_executing_agent_message(
            &chat.id,
            &agent_id,
            Some(frona::chat::message::models::MessageDelivery::pending(
                chrono::Utc::now(),
            )),
        )
        .await
        .unwrap();
    state
        .chat_service
        .complete_agent_message(&executing.id, "hello back".into(), vec![], None)
        .await
        .unwrap();

    let captured_for_poll = captured.clone();
    poll_until("on_send invoked", || {
        let c = captured_for_poll.clone();
        async move { !c.lock().unwrap().is_empty() }
    })
    .await;

    {
        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 1, "exactly one outbound dispatch");
        assert_eq!(calls[0].chat_id, chat.id);
        assert_eq!(calls[0].content, "hello back");
        assert_eq!(calls[0].msg_id, executing.id);
    }

    let msg = state
        .chat_service
        .get_message(&user_id, &executing.id)
        .await
        .unwrap();
    assert_eq!(
        msg.delivery.as_ref().map(|d| d.state),
        Some(frona::chat::message::models::DeliveryState::Sent),
    );
    assert_eq!(msg.external_msg_id.as_deref(), Some(format!("ext-{}", executing.id).as_str()));
}

#[tokio::test]
async fn empty_agent_message_skips_adapter_and_marks_sent() {
    use frona::core::repository::Repository;

    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "e2eempty", "e2eempty@example.com", "password123").await;
    let agent = create_agent(&state, &token, "E2eEmptyAgent").await;
    let agent_id = agent["id"].as_str().unwrap().to_string();

    let captured = std::sync::Arc::new(StdMutex::new(Vec::<CapturedSend>::new()));
    state
        .channel_registry
        .register_factory(std::sync::Arc::new(StubFactory {
            captured: captured.clone(),
        }));

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/spaces",
            &token,
            serde_json::json!({"name": "E2E Empty"}),
        ))
        .await
        .unwrap();
    let space_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let channel = frona::chat::channel::Channel {
        id: format!("channel:{}", uuid::Uuid::new_v4()),
        user_id: user_id.clone(),
        space_id: space_id.clone(),
        provider: "test".into(),
        agent_id: agent_id.clone(),
        config: Default::default(),
        dispatch_mode: frona::chat::channel::DispatchMode::Message,
        status: frona::chat::channel::ChannelStatus::Disconnected,
        error_message: None,
        last_started_at: None,
        user_address: None,
        created_at: now,
        updated_at: now,
    };
    SurrealRepo::<frona::chat::channel::Channel>::new(state.db.clone())
        .create(&channel)
        .await
        .unwrap();
    state
        .channel_manager
        .start_channel(&state, &channel)
        .await
        .unwrap();

    let chat = state
        .chat_service
        .upsert_channel_chat(
            &user_id,
            &space_id,
            &agent_id,
            &channel.id,
            "test:+15559999999",
            None,
        )
        .await
        .unwrap();

    let executing = state
        .chat_service
        .create_executing_agent_message(
            &chat.id,
            &agent_id,
            Some(frona::chat::message::models::MessageDelivery::pending(
                chrono::Utc::now(),
            )),
        )
        .await
        .unwrap();
    state
        .chat_service
        .complete_agent_message(&executing.id, String::new(), vec![], None)
        .await
        .unwrap();

    let svc = state.chat_service.clone();
    let user_for_poll = user_id.clone();
    let msg_id = executing.id.clone();
    poll_until("delivery state settled to Sent", || {
        let svc = svc.clone();
        let uid = user_for_poll.clone();
        let id = msg_id.clone();
        async move {
            svc.get_message(&uid, &id)
                .await
                .ok()
                .and_then(|m| m.delivery.map(|d| d.state))
                == Some(frona::chat::message::models::DeliveryState::Sent)
        }
    })
    .await;

    assert!(
        captured.lock().unwrap().is_empty(),
        "adapter.on_send must NOT be called for an empty agent message",
    );
}
