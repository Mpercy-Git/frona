use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use teloxide::Bot;
use teloxide::payloads::{DeleteWebhookSetters, SendMessageSetters};
use teloxide::prelude::Requester;
use teloxide::types::{ChatAction, ChatId, ParseMode, Recipient, ThreadId};
use url::Url;

use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;

use super::super::models::{
    ChannelAdapter, ChannelCtx, ExternalMessage, external_chat_id,
};
#[cfg(test)]
use super::super::models::ChannelFactory;

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
}

#[derive(crate::ChannelFactory)]
#[channel(id = "telegram", from = TelegramConfig)]
pub struct TelegramAdapter {
    bot: Bot,
}

impl From<TelegramConfig> for TelegramAdapter {
    fn from(cfg: TelegramConfig) -> Self {
        Self {
            bot: Bot::new(cfg.bot_token),
        }
    }
}

impl TelegramAdapter {
    async fn send_bubble(&self, chat: &Chat, text: &str) -> Result<String, AppError> {
        let (chat_id, thread_id) = parse_external_id(external_chat_id(chat)?)?;
        let (rendered, parse_mode) = match telegram_markdown_v2::convert(text) {
            Ok(v2) => (v2, Some(ParseMode::MarkdownV2)),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "telegram MarkdownV2 conversion failed; falling back to plain text",
                );
                (super::markdown::to_plain(text), None)
            }
        };
        let mut send = self.bot.send_message(Recipient::Id(chat_id), rendered);
        if let Some(mode) = parse_mode {
            send = send.parse_mode(mode);
        }
        if let Some(t) = thread_id {
            send = send.message_thread_id(t);
        }
        let sent = send
            .await
            .map_err(|e| AppError::Internal(format!("Telegram sendMessage failed: {e}")))?;
        Ok(sent.id.0.to_string())
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    async fn on_connect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        let url = Url::parse(&ctx.webhook_url).map_err(|e| {
            AppError::Validation(format!("invalid webhook URL {}: {e}", ctx.webhook_url))
        })?;

        // Probe Telegram's current view: skip the round-trip if it's already
        // ours, and explicitly clear any stale registration (with its queued
        // updates) before installing ours so we don't inherit the backlog.
        let info = self.bot.get_webhook_info().await.map_err(|e| {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                error = %e,
                "Telegram getWebhookInfo failed — channel will be marked Failed",
            );
            AppError::Internal(format!("Telegram getWebhookInfo failed: {e}"))
        })?;
        let current = info.url.as_ref().map(|u| u.as_str().to_string());

        if current.as_deref() == Some(url.as_str()) {
            tracing::info!(
                channel_id = %ctx.channel.id,
                url = %ctx.webhook_url,
                "Telegram webhook already registered correctly, skipping setWebhook",
            );
            return Ok(());
        }

        if let Some(existing) = current.as_deref() {
            tracing::info!(
                channel_id = %ctx.channel.id,
                existing = %existing,
                "Telegram bot held a different webhook URL; clearing it (with pending updates) before re-registering",
            );
            if let Err(e) = self
                .bot
                .delete_webhook()
                .drop_pending_updates(true)
                .await
            {
                tracing::warn!(
                    channel_id = %ctx.channel.id,
                    error = %e,
                    "Telegram deleteWebhook failed before re-registering (continuing to setWebhook)",
                );
            }
        }

        self.bot.set_webhook(url).await.map_err(|e| {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                url = %ctx.webhook_url,
                error = %e,
                "Telegram setWebhook failed — channel will be marked Failed (check bot_token, network/DNS, rate limits)",
            );
            AppError::Internal(format!("Telegram setWebhook failed: {e}"))
        })?;
        tracing::info!(
            channel_id = %ctx.channel.id,
            url = %ctx.webhook_url,
            "Telegram channel registered setWebhook",
        );
        Ok(())
    }

    async fn on_disconnect(&self, _ctx: &ChannelCtx) -> Result<(), AppError> {
        if let Err(e) = self.bot.delete_webhook().await {
            tracing::warn!(error = %e, "Telegram deleteWebhook failed (continuing)");
        }
        Ok(())
    }

    async fn on_tool(
        &self,
        tool_call: &crate::inference::tool_call::ToolCall,
        _msg: &Message,
        chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        let Some(text) = tool_call.turn_text.as_deref() else { return Ok(()) };
        if text.trim().is_empty() {
            return Ok(());
        }
        self.send_bubble(chat, text).await?;
        Ok(())
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
        self.send_bubble(chat, &msg.content).await?;
        Ok(())
    }

    async fn on_inference_active(
        &self,
        chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        let Ok(external_id) = external_chat_id(chat) else { return Ok(()) };
        let Ok((chat_id, _thread)) = parse_external_id(external_id) else {
            return Ok(());
        };
        if let Err(e) = self
            .bot
            .send_chat_action(Recipient::Id(chat_id), ChatAction::Typing)
            .await
        {
            tracing::debug!(error = %e, "Telegram sendChatAction failed (best-effort)");
        }
        Ok(())
    }

    async fn on_webhook(
        &self,
        ctx: &ChannelCtx,
        request: Request<Bytes>,
    ) -> Result<Response, AppError> {
        let body: serde_json::Value = serde_json::from_slice(request.body())
            .map_err(|e| AppError::Validation(format!("invalid Telegram webhook body: {e}")))?;
        emit_inbound_update(ctx, body).await?;
        Ok(StatusCode::OK.into_response())
    }
}

fn parse_external_id(s: &str) -> Result<(ChatId, Option<ThreadId>), AppError> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        ["dm", id] | ["group", id] => {
            let n: i64 = id
                .parse()
                .map_err(|e| AppError::Validation(format!("bad chat id in {s:?}: {e}")))?;
            Ok((ChatId(n), None))
        }
        ["group", chat, "topic", thread] => {
            let chat_n: i64 = chat
                .parse()
                .map_err(|e| AppError::Validation(format!("bad chat id in {s:?}: {e}")))?;
            let thread_n: i32 = thread
                .parse()
                .map_err(|e| AppError::Validation(format!("bad thread id in {s:?}: {e}")))?;
            Ok((ChatId(chat_n), Some(ThreadId(teloxide::types::MessageId(thread_n)))))
        }
        _ => Err(AppError::Validation(format!(
            "unrecognised Telegram external_id format: {s:?}"
        ))),
    }
}

/// Telegram inbound update payload — only the fields we care about. We accept the
/// raw JSON and pluck what we need rather than depending on teloxide's full `Update`
/// type, which can change shape between API versions.
#[derive(Debug, Deserialize)]
struct InboundUpdate {
    message: Option<InboundMessage>,
}

#[derive(Debug, Deserialize)]
struct InboundMessage {
    #[serde(default)]
    message_thread_id: Option<i64>,
    chat: InboundChat,
    from: Option<InboundUser>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InboundChat {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct InboundUser {
    id: i64,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

async fn emit_inbound_update(
    ctx: &ChannelCtx,
    raw: serde_json::Value,
) -> Result<(), AppError> {
    let update: InboundUpdate = serde_json::from_value(raw)
        .map_err(|e| AppError::Validation(format!("invalid Telegram update: {e}")))?;
    let Some(message) = update.message else {
        return Ok(());
    };

    let external_chat_id = match (message.chat.kind.as_str(), message.message_thread_id) {
        ("private", _) => format!("dm:{}", message.chat.id),
        (_, Some(thread_id)) => {
            format!("group:{}:topic:{thread_id}", message.chat.id)
        }
        _ => format!("group:{}", message.chat.id),
    };

    let from = message.from.ok_or_else(|| {
        AppError::Validation("Telegram update missing `from` user".into())
    })?;
    let display_name = from
        .username
        .as_deref()
        .map(|u| format!("@{u}"))
        .or_else(|| {
            match (from.first_name.as_deref(), from.last_name.as_deref()) {
                (Some(f), Some(l)) => Some(format!("{f} {l}")),
                (Some(f), None) => Some(f.to_string()),
                (None, Some(l)) => Some(l.to_string()),
                (None, None) => None,
            }
        })
        .unwrap_or_else(|| from.id.to_string());

    let sender_address = from
        .username
        .as_ref()
        .map(|u| format!("@{u}"))
        .unwrap_or_else(|| from.id.to_string());

    let event = ExternalMessage {
        external_chat_id,
        sender_address,
        sender_external_id: Some(from.id.to_string()),
        sender_display_name: Some(display_name),
        content: message.text.unwrap_or_default(),
        attachments: vec![],
    };

    ctx.emit
        .send(event)
        .await
        .map_err(|e| AppError::Internal(format!("inbound emit channel closed: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn manifest_has_bot_token_param() {
        let m = TelegramAdapterFactory.manifest();
        assert_eq!(m.id, "telegram");
        assert_eq!(m.display_name, "Telegram Bot");
        let bot_token = m
            .config_fields
            .iter()
            .find(|f| f.name == "bot_token")
            .expect("bot_token field must be declared");
        assert!(bot_token.is_required);
        assert!(bot_token.is_secret);
    }

    #[test]
    fn factory_create_with_valid_config_succeeds() {
        let cfg = json!({"bot_token": "123:abc-def"});
        let _ch = TelegramAdapterFactory
            .create(cfg)
            .expect("valid config should produce a Channel");
    }

    #[test]
    fn factory_create_rejects_missing_bot_token() {
        let cfg = json!({"unrelated": "x"});
        match TelegramAdapterFactory.create(cfg) {
            Err(AppError::Validation(_)) => {}
            Ok(_) => panic!("expected Validation error, got Ok"),
            Err(e) => panic!("expected Validation, got: {e}"),
        }
    }

    #[test]
    fn parse_external_id_dm() {
        let (chat, thread) = parse_external_id("dm:12345").unwrap();
        assert_eq!(chat, ChatId(12345));
        assert!(thread.is_none());
    }

    #[test]
    fn parse_external_id_group_negative() {
        let (chat, thread) = parse_external_id("group:-1001234567890").unwrap();
        assert_eq!(chat, ChatId(-1001234567890));
        assert!(thread.is_none());
    }

    #[test]
    fn parse_external_id_forum_topic() {
        let (chat, thread) = parse_external_id("group:-100123:topic:42").unwrap();
        assert_eq!(chat, ChatId(-100123));
        assert!(thread.is_some());
    }

    #[test]
    fn parse_external_id_rejects_garbage() {
        assert!(parse_external_id("nonsense").is_err());
        assert!(parse_external_id("dm:notanumber").is_err());
        assert!(parse_external_id("group:1:topic:notanumber").is_err());
    }
}
