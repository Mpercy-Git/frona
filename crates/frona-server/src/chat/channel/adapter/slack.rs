use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use slack_morphism::errors::SlackClientError;
use slack_morphism::prelude::*;
use tokio::sync::OnceCell;

use crate::chat::channel::adapter::markdown;
use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;

use super::super::models::{
    ChannelAdapter, ChannelCtx, ExternalMessage, external_chat_id,
};
#[cfg(test)]
use super::super::models::ChannelFactory;

#[derive(Debug, Clone, Deserialize)]
pub struct SlackConfig {
    pub bot_token: String,
    pub app_token: String,
}

#[derive(Debug, Clone)]
struct SlackSelfIdentity {
    user_id: String,
    bot_id: Option<String>,
}

#[derive(Clone)]
struct SlackChannelState {
    emit: tokio::sync::mpsc::Sender<ExternalMessage>,
    identity: SlackSelfIdentity,
}

#[derive(crate::ChannelFactory)]
#[channel(id = "slack", from = SlackConfig)]
pub struct SlackAdapter {
    bot_token: SlackApiToken,
    app_token: SlackApiToken,
    client: Arc<SlackHyperClient>,
    identity: OnceCell<SlackSelfIdentity>,
}

impl From<SlackConfig> for SlackAdapter {
    fn from(cfg: SlackConfig) -> Self {
        let connector = SlackClientHyperConnector::new()
            .expect("Slack TLS connector init failed — rustls provider should be installed by AppState::new");
        Self {
            bot_token: SlackApiToken::new(SlackApiTokenValue::from(cfg.bot_token)),
            app_token: SlackApiToken::new(SlackApiTokenValue::from(cfg.app_token)),
            client: Arc::new(SlackClient::new(connector)),
            identity: OnceCell::new(),
        }
    }
}

#[async_trait]
impl ChannelAdapter for SlackAdapter {
    async fn on_connect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        let session = self.client.open_session(&self.bot_token);
        let auth = session.auth_test().await.map_err(|e| {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                error = %e,
                "Slack auth.test failed — bot_token rejected (check token is the Bot User OAuth Token, not the App-Level Token)",
            );
            AppError::Validation(format!("Slack auth.test rejected the bot_token: {e}"))
        })?;
        let identity = SlackSelfIdentity {
            user_id: auth.user_id.0.clone(),
            bot_id: auth.bot_id.as_ref().map(|b| b.0.clone()),
        };
        let _ = self.identity.set(identity.clone());

        tracing::info!(
            channel_id = %ctx.channel.id,
            slack_user_id = %identity.user_id,
            slack_team = %auth.team,
            "Slack bot authenticated",
        );

        let env = Arc::new(
            SlackClientEventsListenerEnvironment::new(self.client.clone())
                .with_user_state(SlackChannelState {
                    emit: ctx.emit.clone(),
                    identity,
                })
                .with_error_handler(socket_mode_error_handler),
        );
        let callbacks =
            SlackSocketModeListenerCallbacks::new().with_push_events(handle_push_event);
        let listener = SlackClientSocketModeListener::new(
            &SlackClientSocketModeConfig::new(),
            env,
            callbacks,
        );

        let app_token = self.app_token.clone();
        let cancel = ctx.cancel.clone();
        let channel_id = ctx.channel.id.clone();
        let channel_manager = ctx.channel_manager.clone();
        tokio::spawn(async move {
            if let Err(e) = listener.listen_for(&app_token).await {
                let reason = format!("Slack Socket Mode listen_for failed: {e}");
                tracing::warn!(
                    channel_id = %channel_id,
                    error = %e,
                    "Slack Socket Mode registration failed",
                );
                channel_manager.report_failure(&channel_id, reason).await;
                return;
            }
            // Not `serve()`: dropping its future skips the inner `shutdown()`,
            // leaving slack-morphism's WSS tasks alive after cancellation.
            listener.start().await;
            tracing::info!(
                channel_id = %channel_id,
                "Slack channel connected via Socket Mode",
            );
            cancel.cancelled().await;
            listener.shutdown().await;
            tracing::info!(
                channel_id = %channel_id,
                "Slack Socket Mode listener stopped (channel cancelled)",
            );
        });

        Ok(())
    }

    async fn on_disconnect(&self, _ctx: &ChannelCtx) -> Result<(), AppError> {
        Ok(())
    }

    async fn on_tool(
        &self,
        tool_call: &crate::inference::tool_call::ToolCall,
        _msg: &Message,
        chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        let Some(text) = tool_call.turn_text.as_deref() else {
            return Ok(());
        };
        if text.trim().is_empty() {
            return Ok(());
        }
        self.post_message(chat, text).await
    }

    async fn on_send(
        &self,
        msg: &Message,
        _tool_calls: &[crate::inference::tool_call::ToolCall],
        chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        if msg.content.trim().is_empty() {
            return Ok(());
        }
        self.post_message(chat, &msg.content).await
    }
}

impl SlackAdapter {
    async fn post_message(&self, chat: &Chat, text: &str) -> Result<(), AppError> {
        let (channel_id, thread_ts) = parse_external_id(external_chat_id(chat)?)?;
        if text.trim().is_empty() {
            return Ok(());
        }

        // Block Kit `markdown` renders CommonMark server-side, not Slack mrkdwn.
        // https://api.slack.com/reference/block-kit/blocks#markdown
        let content = SlackMessageContent::new()
            .with_text(markdown::to_plain(text))
            .with_blocks(vec![SlackMarkdownBlock::new(text.to_string()).into()]);
        let mut req = SlackApiChatPostMessageRequest::new(channel_id.clone(), content);
        if let Some(ts) = thread_ts {
            req = req.with_thread_ts(ts);
        }

        let session = self.client.open_session(&self.bot_token);
        if let Err(e) = session.chat_post_message(&req).await {
            return Err(map_post_error(e, &channel_id.0));
        }
        Ok(())
    }
}

fn map_post_error(err: SlackClientError, channel_id: &str) -> AppError {
    if let SlackClientError::ApiError(api) = &err
        && api.code == "not_in_channel"
    {
        return AppError::Validation(format!(
            "Slack rejected chat.postMessage with `not_in_channel` for {channel_id}: invite the bot to the channel (`/invite @YourBot`)"
        ));
    }
    AppError::Internal(format!("Slack chat.postMessage failed: {err}"))
}

async fn handle_push_event(
    event: SlackPushEventCallback,
    _client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<()> {
    let (emit, identity) = {
        let guard = states.read().await;
        match guard.get_user_state::<SlackChannelState>() {
            Some(state) => (state.emit.clone(), state.identity.clone()),
            None => return Ok(()),
        }
    };
    if let Some(msg) = convert_message_event(event, &identity)
        && let Err(e) = emit.send(msg).await
    {
        tracing::warn!(error = %e, "Slack inbound emit channel closed");
    }
    Ok(())
}

fn socket_mode_error_handler(
    err: Box<dyn std::error::Error + Send + Sync + 'static>,
    _client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> HttpStatusCode {
    tracing::warn!(error = %err, "Slack Socket Mode handler error");
    HttpStatusCode::OK
}

fn convert_message_event(
    event: SlackPushEventCallback,
    identity: &SlackSelfIdentity,
) -> Option<ExternalMessage> {
    let SlackEventCallbackBody::Message(m) = event.event else {
        return None;
    };

    match m.subtype.as_ref() {
        None => {}
        Some(SlackMessageEventType::ThreadBroadcast) => {} // user-authored, keep
        Some(_) => return None,
    }

    let user_id = m.sender.user.as_ref().map(|u| u.0.as_str()).unwrap_or("");
    if !user_id.is_empty() && user_id == identity.user_id {
        return None;
    }
    if let (Some(self_bot), Some(event_bot)) = (identity.bot_id.as_deref(), m.sender.bot_id.as_ref())
        && self_bot == event_bot.0
    {
        return None;
    }

    let channel = m.origin.channel.as_ref()?.0.clone();
    let text = m.content.as_ref().and_then(|c| c.text.clone()).unwrap_or_default();
    if text.trim().is_empty() {
        return None;
    }

    let external_chat_id = format_external_id(
        &channel,
        m.origin.channel_type.as_ref(),
        m.origin.thread_ts.as_ref(),
    );

    let display = m
        .sender
        .user_profile
        .as_ref()
        .and_then(|p| p.real_name.clone().or_else(|| p.display_name.clone()))
        .or_else(|| m.sender.username.clone());

    Some(ExternalMessage {
        external_chat_id,
        sender_address: if user_id.is_empty() {
            m.sender.bot_id.as_ref().map(|b| b.0.clone()).unwrap_or_default()
        } else {
            user_id.to_string()
        },
        sender_external_id: if user_id.is_empty() { None } else { Some(user_id.to_string()) },
        sender_display_name: display,
        content: text,
        attachments: vec![],
    })
}

fn format_external_id(
    channel: &str,
    channel_type: Option<&SlackChannelType>,
    thread_ts: Option<&SlackTs>,
) -> String {
    let is_dm = matches!(channel_type, Some(SlackChannelType(s)) if s == "im")
        || channel.starts_with('D');
    if is_dm {
        format!("dm:{channel}")
    } else if let Some(ts) = thread_ts {
        format!("group:{channel}:thread:{}", ts.0)
    } else {
        format!("group:{channel}")
    }
}

fn parse_external_id(s: &str) -> Result<(SlackChannelId, Option<SlackTs>), AppError> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        ["dm", id] | ["group", id] if !id.is_empty() => {
            Ok((SlackChannelId(id.to_string()), None))
        }
        ["group", id, "thread", ts] if !id.is_empty() && !ts.is_empty() => Ok((
            SlackChannelId(id.to_string()),
            Some(SlackTs(ts.to_string())),
        )),
        _ => Err(AppError::Validation(format!(
            "unrecognised Slack external_id format: {s:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Tests that construct `SlackAdapter::from(...)` must call this: rustls
    /// panics without an installed `CryptoProvider`, and tests don't go
    /// through `AppState::new` where prod installs it.
    fn install_crypto_provider() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }

    fn identity() -> SlackSelfIdentity {
        SlackSelfIdentity {
            user_id: "U07BOT".into(),
            bot_id: Some("B07BOT".into()),
        }
    }

    #[test]
    fn manifest_declares_required_secret_tokens() {
        let m = SlackAdapterFactory.manifest();
        assert_eq!(m.id, "slack");
        assert_eq!(m.display_name, "Slack");
        let by_name = |n: &str| {
            m.config_fields
                .iter()
                .find(|f| f.name == n)
                .unwrap_or_else(|| panic!("field {n} missing"))
        };
        for n in ["bot_token", "app_token"] {
            let f = by_name(n);
            assert!(f.is_required, "{n} should be required");
            assert!(f.is_secret, "{n} should be marked secret");
        }
    }

    #[test]
    fn factory_create_with_valid_config_succeeds() {
        install_crypto_provider();
        let cfg = json!({"bot_token": "xoxb-abc", "app_token": "xapp-xyz"});
        SlackAdapterFactory
            .create(cfg)
            .expect("valid config should produce a SlackAdapter");
    }

    #[test]
    fn factory_create_rejects_missing_app_token() {
        let cfg = json!({"bot_token": "xoxb-abc"});
        assert!(matches!(
            SlackAdapterFactory.create(cfg),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn factory_create_rejects_missing_bot_token() {
        let cfg = json!({"app_token": "xapp-xyz"});
        assert!(matches!(
            SlackAdapterFactory.create(cfg),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn parse_external_id_dm() {
        let (chan, ts) = parse_external_id("dm:D1ABCD").unwrap();
        assert_eq!(chan.0, "D1ABCD");
        assert!(ts.is_none());
    }

    #[test]
    fn parse_external_id_group() {
        let (chan, ts) = parse_external_id("group:C123456").unwrap();
        assert_eq!(chan.0, "C123456");
        assert!(ts.is_none());
    }

    #[test]
    fn parse_external_id_threaded() {
        let (chan, ts) = parse_external_id("group:C123456:thread:1700000000.000100").unwrap();
        assert_eq!(chan.0, "C123456");
        assert_eq!(ts.unwrap().0, "1700000000.000100");
    }

    #[test]
    fn parse_external_id_rejects_garbage() {
        assert!(parse_external_id("nonsense").is_err());
        assert!(parse_external_id("dm:").is_err());
        assert!(parse_external_id("group:").is_err());
        assert!(parse_external_id("group:C1:thread:").is_err());
    }

    #[test]
    fn format_external_id_dm_by_channel_type() {
        let id = format_external_id(
            "DXYZ",
            Some(&SlackChannelType("im".to_string())),
            None,
        );
        assert_eq!(id, "dm:DXYZ");
    }

    #[test]
    fn format_external_id_dm_by_prefix_fallback() {
        let id = format_external_id("D9999", None, None);
        assert_eq!(id, "dm:D9999");
    }

    #[test]
    fn format_external_id_group_with_thread() {
        let id = format_external_id(
            "C1",
            Some(&SlackChannelType("channel".to_string())),
            Some(&SlackTs("1700000000.000100".to_string())),
        );
        assert_eq!(id, "group:C1:thread:1700000000.000100");
    }

    fn message_event(payload: serde_json::Value) -> SlackPushEventCallback {
        let wrapper = json!({
            "team_id": "T1",
            "api_app_id": "A1",
            "event": payload,
            "type": "event_callback",
            "event_id": "Ev1",
            "event_time": 1700000000,
            "event_context": "ctx",
            "authed_users": [],
        });
        serde_json::from_value(wrapper).expect("test event JSON must parse")
    }

    #[test]
    fn convert_dm_text_returns_external_message() {
        let evt = message_event(json!({
            "type": "message",
            "channel": "D1ABCD",
            "channel_type": "im",
            "user": "U07HUMAN",
            "text": "hello",
            "ts": "1700000000.000100",
        }));
        let m = convert_message_event(evt, &identity()).expect("should convert");
        assert_eq!(m.external_chat_id, "dm:D1ABCD");
        assert_eq!(m.sender_address, "U07HUMAN");
        assert_eq!(m.sender_external_id.as_deref(), Some("U07HUMAN"));
        assert_eq!(m.content, "hello");
    }

    #[test]
    fn convert_channel_text_returns_group_external_id() {
        let evt = message_event(json!({
            "type": "message",
            "channel": "C99",
            "channel_type": "channel",
            "user": "U07HUMAN",
            "text": "hi team",
            "ts": "1700000001.000200",
        }));
        let m = convert_message_event(evt, &identity()).expect("should convert");
        assert_eq!(m.external_chat_id, "group:C99");
    }

    #[test]
    fn convert_threaded_reply_includes_thread_ts() {
        let evt = message_event(json!({
            "type": "message",
            "channel": "C99",
            "channel_type": "channel",
            "user": "U07HUMAN",
            "text": "reply",
            "ts": "1700000002.000300",
            "thread_ts": "1700000000.000100",
        }));
        let m = convert_message_event(evt, &identity()).expect("should convert");
        assert_eq!(m.external_chat_id, "group:C99:thread:1700000000.000100");
    }

    #[test]
    fn convert_skips_bot_message_subtype() {
        let evt = message_event(json!({
            "type": "message",
            "subtype": "bot_message",
            "channel": "C99",
            "channel_type": "channel",
            "bot_id": "B07OTHER",
            "text": "from a bot",
            "ts": "1700000003.000400",
        }));
        assert!(convert_message_event(evt, &identity()).is_none());
    }

    #[test]
    fn convert_skips_self_user_id() {
        let evt = message_event(json!({
            "type": "message",
            "channel": "C99",
            "channel_type": "channel",
            "user": "U07BOT",
            "text": "echo of self",
            "ts": "1700000004.000500",
        }));
        assert!(convert_message_event(evt, &identity()).is_none());
    }

    #[test]
    fn convert_skips_self_bot_id() {
        let evt = message_event(json!({
            "type": "message",
            "channel": "C99",
            "channel_type": "channel",
            "user": "U07HUMAN",
            "bot_id": "B07BOT",
            "text": "posted by our app",
            "ts": "1700000005.000600",
        }));
        assert!(convert_message_event(evt, &identity()).is_none());
    }

    #[test]
    fn convert_skips_message_changed_subtype() {
        let evt = message_event(json!({
            "type": "message",
            "subtype": "message_changed",
            "channel": "C99",
            "channel_type": "channel",
            "ts": "1700000006.000700",
        }));
        assert!(convert_message_event(evt, &identity()).is_none());
    }

    #[test]
    fn convert_skips_channel_join_subtype() {
        let evt = message_event(json!({
            "type": "message",
            "subtype": "channel_join",
            "channel": "C99",
            "channel_type": "channel",
            "user": "U07HUMAN",
            "text": "<@U07HUMAN> has joined the channel",
            "ts": "1700000007.000800",
        }));
        assert!(convert_message_event(evt, &identity()).is_none());
    }

    #[test]
    fn convert_skips_empty_text() {
        let evt = message_event(json!({
            "type": "message",
            "channel": "C99",
            "channel_type": "channel",
            "user": "U07HUMAN",
            "text": "   ",
            "ts": "1700000008.000900",
        }));
        assert!(convert_message_event(evt, &identity()).is_none());
    }

    #[test]
    fn convert_prefers_real_name_for_display() {
        let evt = message_event(json!({
            "type": "message",
            "channel": "D1ABCD",
            "channel_type": "im",
            "user": "U07HUMAN",
            "text": "hi",
            "ts": "1700000009.001000",
            "user_profile": {
                "real_name": "Ada Lovelace",
                "display_name": "ada",
            },
        }));
        let m = convert_message_event(evt, &identity()).expect("should convert");
        assert_eq!(m.sender_display_name.as_deref(), Some("Ada Lovelace"));
    }
}
