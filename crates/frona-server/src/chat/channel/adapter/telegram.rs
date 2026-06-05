use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use teloxide::Bot;
use teloxide::payloads::{
    AnswerCallbackQuerySetters, DeleteWebhookSetters, EditMessageTextSetters, SendMessageSetters,
};
use teloxide::prelude::Requester;
use teloxide::types::{
    ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode, Recipient, ThreadId,
};
use url::Url;

use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;

use super::super::models::{
    ChannelAdapter, ChannelCtx, ExternalMessage, external_chat_id,
};
use super::super::typing::TypingIndicator;
#[cfg(test)]
use super::super::models::ChannelFactory;

/// Telegram's typing indicator auto-fades in ~5s. Refresh just before that
/// so long inferences keep showing "typing…" without bombarding the API.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
}

#[derive(crate::ChannelFactory)]
#[channel(id = "telegram", from = TelegramConfig)]
pub struct TelegramAdapter {
    bot: Bot,
    typing: TypingIndicator,
}

impl From<TelegramConfig> for TelegramAdapter {
    fn from(cfg: TelegramConfig) -> Self {
        Self {
            bot: Bot::new(cfg.bot_token),
            typing: TypingIndicator::new(),
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

    async fn on_inference_start(
        &self,
        chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        let Ok(external_id) = external_chat_id(chat) else { return Ok(()) };
        let Ok((tg_chat_id, _thread)) = parse_external_id(external_id) else {
            return Ok(());
        };

        let bot = self.bot.clone();
        self.typing.start(chat.id.clone(), TYPING_REFRESH_INTERVAL, move || {
            let bot = bot.clone();
            async move {
                if let Err(e) = bot
                    .send_chat_action(Recipient::Id(tg_chat_id), ChatAction::Typing)
                    .await
                {
                    tracing::debug!(error = %e, "Telegram sendChatAction failed (best-effort)");
                }
            }
        }).await;
        Ok(())
    }

    async fn on_inference_done(
        &self,
        chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        self.typing.stop(&chat.id).await;
        Ok(())
    }

    async fn on_webhook(
        &self,
        ctx: &ChannelCtx,
        request: Request<Bytes>,
    ) -> Result<Response, AppError> {
        let body: serde_json::Value = serde_json::from_slice(request.body())
            .map_err(|e| AppError::Validation(format!("invalid Telegram webhook body: {e}")))?;

        // Route callback_query (button taps) to the HITL resolve dispatcher
        // BEFORE delegating to the standard inbound-message emitter.
        if let Some(cq) = body.get("callback_query") {
            if let Err(e) = self.handle_callback_query(ctx, cq.clone()).await {
                tracing::warn!(error = %e, "Telegram callback_query handling failed");
            }
            return Ok(StatusCode::OK.into_response());
        }

        emit_inbound_update(ctx, body).await?;
        Ok(StatusCode::OK.into_response())
    }

    async fn on_pending_hitl(
        &self,
        batch: &[crate::inference::tool_call::ToolCall],
        _msg: &Message,
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<Vec<crate::inference::hitl::HitlDelivery>, AppError> {
        let mut out = Vec::new();
        for tc in batch {
            let Some(h) = tc.hitl.as_ref() else { continue };
            let kind = crate::chat::channel::hitl::kind_for(&h.request);
            let keyboard = build_inline_keyboard(&tc.id, &kind, &h.url);

            let (chat_id, thread_id) = match parse_external_id(external_chat_id(chat)?) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, "Telegram on_pending_hitl: parse_external_id");
                    break;
                }
            };

            let (rendered, parse_mode) = match telegram_markdown_v2::convert(&h.prompt) {
                Ok(v2) => (v2, Some(ParseMode::MarkdownV2)),
                Err(_) => (super::markdown::to_plain(&h.prompt), None),
            };

            let mut send = self
                .bot
                .send_message(Recipient::Id(chat_id), rendered)
                .reply_markup(keyboard);
            if let Some(mode) = parse_mode {
                send = send.parse_mode(mode);
            }
            if let Some(t) = thread_id {
                send = send.message_thread_id(t);
            }

            match send.await {
                Ok(sent) => out.push(crate::inference::hitl::HitlDelivery {
                    channel_id: ctx.channel.id.clone(),
                    external_message_id: sent.id.0.to_string(),
                    delivered_at: chrono::Utc::now(),
                }),
                Err(e) => {
                    tracing::warn!(
                        tool_call_id = %tc.id,
                        error = %e,
                        "Telegram on_pending_hitl: send failed, stopping batch",
                    );
                    break;
                }
            }
        }
        Ok(out)
    }
}

/// Build the inline keyboard for a HITL prompt. Approval → Yes/No; Choice →
/// one button per option + a "Open on web →" URL fallback; External → URL only.
fn build_inline_keyboard(
    tcid: &str,
    kind: &crate::chat::channel::hitl::HitlKind,
    url: &str,
) -> InlineKeyboardMarkup {
    use crate::chat::channel::hitl::HitlKind;
    let url_button = || {
        Url::parse(url).ok().map(|u| {
            vec![InlineKeyboardButton::url(
                "Open on web →".to_string(),
                u,
            )]
        })
    };

    match kind {
        HitlKind::Approval => InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("Yes".to_string(), format!("r:{tcid}:y")),
            InlineKeyboardButton::callback("No".to_string(), format!("r:{tcid}:n")),
        ]]),
        HitlKind::Choice { options } => {
            let mut rows: Vec<Vec<InlineKeyboardButton>> = options
                .iter()
                .enumerate()
                .map(|(i, opt)| {
                    vec![InlineKeyboardButton::callback(
                        opt.clone(),
                        format!("r:{tcid}:c:{i}"),
                    )]
                })
                .collect();
            if let Some(row) = url_button() {
                rows.push(row);
            }
            InlineKeyboardMarkup::new(rows)
        }
        HitlKind::External => InlineKeyboardMarkup::new(
            url_button()
                .map(|row| vec![row])
                .unwrap_or_default(),
        ),
    }
}

impl TelegramAdapter {
    async fn handle_callback_query(
        &self,
        ctx: &ChannelCtx,
        cq: serde_json::Value,
    ) -> Result<(), AppError> {
        let cq_id = cq
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("callback_query missing id".into()))?
            .to_string();
        let data = cq.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let message_obj = cq.get("message");

        let (tool_call_id, response) = match crate::chat::channel::hitl::parse_resolve_callback_data(
            data,
            &ctx.chat_service,
        )
        .await
        {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::warn!(data = %data, error = %e, "telegram callback_data parse failed");
                let _ = self
                    .bot
                    .answer_callback_query(teloxide::types::CallbackQueryId(cq_id.clone()))
                    .text("Could not interpret that action.".to_string())
                    .await;
                return Ok(());
            }
        };

        // Capture the user-facing label BEFORE we move `response` into
        // resolve_hitl. The toast/message-edit reflects what the user
        // actually picked, not a generic "Resolved" placeholder.
        let answer_label = crate::chat::channel::hitl::response_display(&response);

        let outcome = ctx
            .channel_manager
            .resolve_hitl(&tool_call_id, response)
            .await;

        let toast = match &outcome {
            Ok(crate::inference::hitl::ResolveOutcome::Resolved { .. }) => answer_label.clone(),
            Ok(crate::inference::hitl::ResolveOutcome::AlreadyResolved) => {
                "Already resolved".to_string()
            }
            Err(e) => format!("Failed: {e}"),
        };
        let _ = self
            .bot
            .answer_callback_query(teloxide::types::CallbackQueryId(cq_id.clone()))
            .text(toast.clone())
            .await;

        // Edit the original message to remove buttons + reflect outcome.
        if let Some(msg_obj) = message_obj
            && let (Some(chat_id), Some(msg_id)) = (
                msg_obj
                    .get("chat")
                    .and_then(|c| c.get("id"))
                    .and_then(|v| v.as_i64()),
                msg_obj.get("message_id").and_then(|v| v.as_i64()),
            )
        {
            let chat_id = ChatId(chat_id);
            let msg_id = teloxide::types::MessageId(msg_id as i32);
            let original_text = msg_obj
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let new_text = format!("{original_text}\n\n→ {toast}");
            let _ = self
                .bot
                .edit_message_text(Recipient::Id(chat_id), msg_id, new_text)
                .reply_markup(InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new()))
                .await;
        }
        Ok(())
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

    #[test]
    fn build_inline_keyboard_approval_has_two_buttons() {
        use crate::chat::channel::hitl::HitlKind;
        let kb = build_inline_keyboard("tc-1", &HitlKind::Approval, "https://x/chats/abc");
        assert_eq!(kb.inline_keyboard.len(), 1);
        assert_eq!(kb.inline_keyboard[0].len(), 2);
        // Buttons should encode the tool_call_id in callback_data.
        let labels: Vec<&str> = kb.inline_keyboard[0]
            .iter()
            .map(|b| b.text.as_str())
            .collect();
        assert!(labels.contains(&"Yes"));
        assert!(labels.contains(&"No"));
    }

    #[test]
    fn build_inline_keyboard_choice_has_option_row_and_url_row() {
        use crate::chat::channel::hitl::HitlKind;
        let kb = build_inline_keyboard(
            "tc-1",
            &HitlKind::Choice {
                options: vec!["us".to_string(), "eu".to_string()],
            },
            "https://x/chats/abc",
        );
        // 2 option rows + 1 URL row
        assert_eq!(kb.inline_keyboard.len(), 3);
        assert_eq!(kb.inline_keyboard[0][0].text, "us");
        assert_eq!(kb.inline_keyboard[1][0].text, "eu");
        assert_eq!(kb.inline_keyboard[2][0].text, "Open on web →");
    }

    #[test]
    fn build_inline_keyboard_external_has_only_url_button() {
        use crate::chat::channel::hitl::HitlKind;
        let kb = build_inline_keyboard("tc-1", &HitlKind::External, "https://x/chats/abc");
        assert_eq!(kb.inline_keyboard.len(), 1);
        assert_eq!(kb.inline_keyboard[0][0].text, "Open on web →");
    }
}
