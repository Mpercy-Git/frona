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

#[derive(Default)]
#[allow(dead_code)] // msg_id is recorded for diagnostic purposes; only some tests assert on it.
struct CapturedToolCall {
    msg_id: String,
    tool_call_id: String,
    turn_text: Option<String>,
}

#[derive(Default)]
struct StubConfig {
    render_tool_segments: bool,
    fail_on_tool: Option<(String, String)>,
    fail_on_send: Option<String>,
}

struct StubAdapter {
    captured: std::sync::Arc<StdMutex<Vec<CapturedSend>>>,
    tool_calls: std::sync::Arc<StdMutex<Vec<CapturedToolCall>>>,
    config: std::sync::Arc<StdMutex<StubConfig>>,
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
    async fn on_tool(
        &self,
        tc: &frona::inference::tool_call::ToolCall,
        msg: &frona::chat::message::models::Message,
        _chat: &frona::chat::models::Chat,
        _ctx: &frona::chat::channel::ChannelCtx,
    ) -> Result<(), frona::core::error::AppError> {
        let injected = {
            let mut cfg = self.config.lock().unwrap();
            if let Some((id, _)) = &cfg.fail_on_tool {
                if id == &tc.id {
                    cfg.fail_on_tool.take()
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some((_, err)) = injected {
            return Err(frona::core::error::AppError::Internal(err));
        }
        let render = self.config.lock().unwrap().render_tool_segments;
        if !render {
            return Ok(());
        }
        self.tool_calls.lock().unwrap().push(CapturedToolCall {
            msg_id: msg.id.clone(),
            tool_call_id: tc.id.clone(),
            turn_text: tc.turn_text.clone(),
        });
        Ok(())
    }
    async fn on_send(
        &self,
        msg: &frona::chat::message::models::Message,
        _tool_calls: &[frona::inference::tool_call::ToolCall],
        chat: &frona::chat::models::Chat,
        _ctx: &frona::chat::channel::ChannelCtx,
    ) -> Result<(), frona::core::error::AppError> {
        let injected = self.config.lock().unwrap().fail_on_send.take();
        if let Some(err) = injected {
            return Err(frona::core::error::AppError::Internal(err));
        }
        if msg.content.trim().is_empty() {
            return Ok(());
        }
        self.captured.lock().unwrap().push(CapturedSend {
            msg_id: msg.id.clone(),
            chat_id: chat.id.clone(),
            content: msg.content.clone(),
        });
        Ok(())
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
    tool_calls: std::sync::Arc<StdMutex<Vec<CapturedToolCall>>>,
    config: std::sync::Arc<StdMutex<StubConfig>>,
}

impl StubFactory {
    fn new(captured: std::sync::Arc<StdMutex<Vec<CapturedSend>>>) -> Self {
        Self {
            captured,
            tool_calls: std::sync::Arc::new(StdMutex::new(Vec::new())),
            config: std::sync::Arc::new(StdMutex::new(StubConfig::default())),
        }
    }
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
            tool_calls: self.tool_calls.clone(),
            config: self.config.clone(),
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
        .register_factory(std::sync::Arc::new(StubFactory::new(
            captured.clone()
)));

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
        .register_factory(std::sync::Arc::new(StubFactory::new(
            captured.clone()
)));

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
        .register_factory(std::sync::Arc::new(StubFactory::new(
            captured.clone()
)));

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

struct SegmentTestSetup {
    state: frona::core::state::AppState,
    chat: frona::chat::models::Chat,
    agent_id: String,
    captured: std::sync::Arc<StdMutex<Vec<CapturedSend>>>,
    tool_calls_recorder: std::sync::Arc<StdMutex<Vec<CapturedToolCall>>>,
    config: std::sync::Arc<StdMutex<StubConfig>>,
    _tmp: tempfile::TempDir,
}

async fn setup_segment_test(prefix: &str) -> SegmentTestSetup {
    use frona::core::repository::Repository;

    let (state, _tmp) = test_app_state().await;
    let (token, user_id) = register_user(
        &state,
        prefix,
        &format!("{prefix}@example.com"),
        "password123",
    )
    .await;
    let agent = create_agent(&state, &token, &format!("{prefix}_agent")).await;
    let agent_id = agent["id"].as_str().unwrap().to_string();

    let captured = std::sync::Arc::new(StdMutex::new(Vec::<CapturedSend>::new()));
    let factory = std::sync::Arc::new(StubFactory::new(captured.clone()));
    let tool_calls_recorder = factory.tool_calls.clone();
    let config = factory.config.clone();
    state.channel_registry.register_factory(factory);

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/spaces",
            &token,
            serde_json::json!({"name": format!("{prefix} Space")}),
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
            &format!("test:{prefix}"),
            None,
        )
        .await
        .unwrap();
    let chat = state.chat_service.find_chat(&chat.id).await.unwrap().unwrap();

    SegmentTestSetup {
        state,
        chat,
        agent_id,
        captured,
        tool_calls_recorder,
        config,
        _tmp,
    }
}

async fn create_executing_msg(
    state: &frona::core::state::AppState,
    chat_id: &str,
    agent_id: &str,
) -> frona::chat::message::models::Message {
    let resp = state
        .chat_service
        .create_executing_agent_message(
            chat_id,
            agent_id,
            Some(frona::chat::message::models::MessageDelivery::pending(
                chrono::Utc::now(),
            )),
        )
        .await
        .unwrap();
    state
        .chat_service
        .find_message(&resp.id)
        .await
        .unwrap()
        .expect("message just created")
}

async fn insert_tool_call(
    state: &frona::core::state::AppState,
    chat_id: &str,
    msg_id: &str,
    turn: u32,
    turn_text: Option<&str>,
) -> frona::inference::tool_call::ToolCall {
    let id = uuid::Uuid::new_v4().to_string();
    state
        .chat_service
        .begin_tool_call(
            &id,
            chat_id,
            msg_id,
            turn,
            &format!("provider-{id}"),
            "stub_tool",
            &serde_json::json!({}),
            None,
            turn_text.map(String::from),
        )
        .await
        .unwrap()
}

async fn complete_msg(
    state: &frona::core::state::AppState,
    msg_id: &str,
    content: &str,
) {
    state
        .chat_service
        .complete_agent_message(msg_id, content.to_string(), vec![], None)
        .await
        .unwrap();
}

async fn reload_msg(
    state: &frona::core::state::AppState,
    msg_id: &str,
) -> frona::chat::message::models::Message {
    state
        .chat_service
        .find_message(msg_id)
        .await
        .unwrap()
        .expect("message must exist")
}

async fn poll_delivery_state(
    state: &frona::core::state::AppState,
    msg_id: &str,
    target: frona::chat::message::models::DeliveryState,
) {
    let svc = state.chat_service.clone();
    let id = msg_id.to_string();
    poll_until(&format!("delivery state == {target:?} for {msg_id}"), || {
        let svc = svc.clone();
        let id = id.clone();
        async move {
            svc.find_message(&id)
                .await
                .ok()
                .flatten()
                .and_then(|m| m.delivery.map(|d| d.state))
                == Some(target)
        }
    })
    .await;
}

#[tokio::test]
async fn segments_happy_path_tools_then_trailing() {
    let setup = setup_segment_test("seg_happy").await;
    setup.config.lock().unwrap().render_tool_segments = true;

    let msg = create_executing_msg(&setup.state, &setup.chat.id, &setup.agent_id).await;
    let tc0 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 0, Some("first")).await;
    let tc1 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 1, Some("second")).await;

    complete_msg(&setup.state, &msg.id, "tail").await;
    poll_delivery_state(&setup.state, &msg.id, frona::chat::message::models::DeliveryState::Sent).await;

    {
        let tool_calls_seen = setup.tool_calls_recorder.lock().unwrap();
        assert_eq!(tool_calls_seen.len(), 2, "on_tool fires once per tool segment");
        assert_eq!(tool_calls_seen[0].tool_call_id, tc0.id);
        assert_eq!(tool_calls_seen[0].turn_text.as_deref(), Some("first"));
        assert_eq!(tool_calls_seen[1].tool_call_id, tc1.id);
    }
    {
        let sends = setup.captured.lock().unwrap();
        assert_eq!(sends.len(), 1, "on_send fires once for trailing");
        assert_eq!(sends[0].content, "tail");
    }

    let final_msg = reload_msg(&setup.state, &msg.id).await;
    assert_eq!(final_msg.delivery.unwrap().tool_index, 2, "cursor at final_index after trailing sent");
}

#[tokio::test]
async fn segments_skip_empty_turn_text_at_manager() {
    let setup = setup_segment_test("seg_skip").await;
    setup.config.lock().unwrap().render_tool_segments = true;

    let msg = create_executing_msg(&setup.state, &setup.chat.id, &setup.agent_id).await;
    insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 0, Some("")).await;
    insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 1, None).await;
    let tc2 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 2, Some("real")).await;

    complete_msg(&setup.state, &msg.id, "tail").await;
    poll_delivery_state(&setup.state, &msg.id, frona::chat::message::models::DeliveryState::Sent).await;

    {
        let seen = setup.tool_calls_recorder.lock().unwrap();
        assert_eq!(seen.len(), 1, "on_tool only invoked for non-empty turn_text");
        assert_eq!(seen[0].tool_call_id, tc2.id);
    }

    let final_msg = reload_msg(&setup.state, &msg.id).await;
    assert_eq!(final_msg.delivery.unwrap().tool_index, 3, "cursor at final_index after walking 3 tools");
}

#[tokio::test]
async fn segments_empty_trailing_drains_to_sent() {
    let setup = setup_segment_test("seg_drain").await;
    setup.config.lock().unwrap().render_tool_segments = true;

    let msg = create_executing_msg(&setup.state, &setup.chat.id, &setup.agent_id).await;
    insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 0, Some("only this")).await;

    complete_msg(&setup.state, &msg.id, "").await;
    poll_delivery_state(&setup.state, &msg.id, frona::chat::message::models::DeliveryState::Sent).await;

    assert_eq!(setup.tool_calls_recorder.lock().unwrap().len(), 1);
    assert!(
        setup.captured.lock().unwrap().is_empty(),
        "on_send returns None for empty trailing → no captured send",
    );
}

#[tokio::test]
async fn segments_transient_failure_backs_off_and_resumes() {
    let setup = setup_segment_test("seg_transient").await;
    setup.config.lock().unwrap().render_tool_segments = true;

    let msg = create_executing_msg(&setup.state, &setup.chat.id, &setup.agent_id).await;
    let tc0 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 0, Some("first")).await;
    let tc1 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 1, Some("second")).await;

    setup.config.lock().unwrap().fail_on_tool = Some((tc1.id.clone(), "transient blip".into()));

    complete_msg(&setup.state, &msg.id, "tail").await;
    poll_delivery_state(&setup.state, &msg.id, frona::chat::message::models::DeliveryState::Failed).await;

    {
        let seen = setup.tool_calls_recorder.lock().unwrap();
        assert_eq!(seen.len(), 1, "only segment 0 made it through");
        assert_eq!(seen[0].tool_call_id, tc0.id);
    }
    {
        let mid = reload_msg(&setup.state, &msg.id).await.delivery.unwrap();
        assert_eq!(mid.tool_index, 1, "cursor halted at the failed segment");
        assert!(mid.next_attempt_at.is_some(), "transient → backoff scheduled");
        assert_eq!(mid.attempts, 1);
    }

    {
        use frona::core::repository::Repository;
        let repo = SurrealRepo::<frona::chat::message::models::Message>::new(setup.state.db.clone());
        let mut m = setup.state.chat_service.find_message(&msg.id).await.unwrap().unwrap();
        m.delivery.as_mut().unwrap().next_attempt_at = Some(chrono::Utc::now());
        repo.update(&m).await.unwrap();
    }
    let _ = setup.state.channel_manager.retry_due_deliveries().await.unwrap();
    poll_delivery_state(&setup.state, &msg.id, frona::chat::message::models::DeliveryState::Sent).await;

    let seen_after = setup.tool_calls_recorder.lock().unwrap();
    assert_eq!(seen_after.len(), 2, "retry sent segment 1");
    assert_eq!(seen_after[1].tool_call_id, tc1.id);
    drop(seen_after);
    assert_eq!(setup.captured.lock().unwrap().len(), 1, "trailing sent once");
}

#[tokio::test]
async fn segments_permanent_failure_halts() {
    let setup = setup_segment_test("seg_permanent").await;
    setup.config.lock().unwrap().render_tool_segments = true;

    let msg = create_executing_msg(&setup.state, &setup.chat.id, &setup.agent_id).await;
    insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 0, Some("first")).await;
    let tc1 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 1, Some("second")).await;

    setup.config.lock().unwrap().fail_on_tool =
        Some((tc1.id.clone(), "Forbidden: bot was blocked".into()));

    complete_msg(&setup.state, &msg.id, "tail").await;
    poll_delivery_state(&setup.state, &msg.id, frona::chat::message::models::DeliveryState::Failed).await;

    let delivery = reload_msg(&setup.state, &msg.id).await.delivery.unwrap();
    assert_eq!(delivery.tool_index, 1);
    assert!(
        delivery.next_attempt_at.is_none(),
        "permanent error must drain the retry queue (next_attempt_at=None)",
    );
}

#[tokio::test]
async fn segments_resume_after_partial_delivery() {
    let setup = setup_segment_test("seg_resume").await;
    setup.config.lock().unwrap().render_tool_segments = true;

    let msg = create_executing_msg(&setup.state, &setup.chat.id, &setup.agent_id).await;
    let _tc0 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 0, Some("a")).await;
    let _tc1 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 1, Some("b")).await;
    let tc2 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 2, Some("c")).await;

    setup.state.channel_manager.record_segment_progress(&msg.id).await.unwrap();
    setup.state.channel_manager.record_segment_progress(&msg.id).await.unwrap();
    assert_eq!(reload_msg(&setup.state, &msg.id).await.delivery.unwrap().tool_index, 2);

    complete_msg(&setup.state, &msg.id, "tail").await;
    poll_delivery_state(&setup.state, &msg.id, frona::chat::message::models::DeliveryState::Sent).await;

    let seen = setup.tool_calls_recorder.lock().unwrap();
    assert_eq!(seen.len(), 1, "only segment 2 was sent on resume");
    assert_eq!(seen[0].tool_call_id, tc2.id);
    drop(seen);
    assert_eq!(setup.captured.lock().unwrap().len(), 1, "trailing also sent");
}

#[tokio::test]
async fn segments_executing_excluded_from_retry_then_completed_walks_full_list() {
    let setup = setup_segment_test("seg_exec").await;
    setup.config.lock().unwrap().render_tool_segments = true;

    let msg = create_executing_msg(&setup.state, &setup.chat.id, &setup.agent_id).await;
    let tc0 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 0, Some("a")).await;
    let _tc1 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 1, Some("b")).await;

    let retried = setup.state.channel_manager.retry_due_deliveries().await.unwrap();
    assert_eq!(retried, 0, "Executing must not surface in retry queue");
    assert!(setup.tool_calls_recorder.lock().unwrap().is_empty());
    assert!(setup.captured.lock().unwrap().is_empty());

    let _tc2 = insert_tool_call(&setup.state, &setup.chat.id, &msg.id, 2, Some("c")).await;

    complete_msg(&setup.state, &msg.id, "tail").await;
    poll_delivery_state(&setup.state, &msg.id, frona::chat::message::models::DeliveryState::Sent).await;

    let seen = setup.tool_calls_recorder.lock().unwrap();
    assert_eq!(seen.len(), 3, "all 3 tool segments delivered, including the late-appended one");
    assert_eq!(seen[0].tool_call_id, tc0.id);
    drop(seen);
    assert_eq!(setup.captured.lock().unwrap().len(), 1);
}
