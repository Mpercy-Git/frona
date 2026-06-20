use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use teloxide::Bot;
use teloxide::payloads::{
    AnswerCallbackQuerySetters, DeleteWebhookSetters, EditMessageTextSetters,
    SendAudioSetters, SendMediaGroupSetters, SendMessageSetters,
    SendPhotoSetters, SendVideoSetters,
};
use teloxide::prelude::Requester;
use teloxide::types::{
    ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, InputMedia,
    InputMediaPhoto, ParseMode, Recipient, ThreadId,
};
use url::Url;

use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;
use teloxide::ApiError;
use teloxide::RequestError;

use super::super::attachment;
use super::super::error::{ChannelError, ChannelErrorKind};
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
    splitter: super::split::TelegramMarkdownV2Splitter,
}

/// Telegram MarkdownV2 caps a single `send_message` at 4096 chars. Longer
/// agent replies split into sequential top-level messages.
const TELEGRAM_MAX_MESSAGE_LEN: usize = 4096;

impl From<TelegramConfig> for TelegramAdapter {
    fn from(cfg: TelegramConfig) -> Self {
        Self {
            bot: Bot::new(cfg.bot_token),
            typing: TypingIndicator::new(),
            splitter: super::split::TelegramMarkdownV2Splitter::new(
                TELEGRAM_MAX_MESSAGE_LEN,
                None,
            ),
        }
    }
}

fn classify_telegram_error(e: &RequestError) -> ChannelError {
    let msg = e.to_string();
    match e {
        RequestError::RetryAfter(s) => {
            ChannelError::transient(msg).with_retry_hint(s.duration())
        }
        RequestError::Network(_) | RequestError::Io(_) => ChannelError::transient(msg),
        RequestError::MigrateToChatId(_) => {
            ChannelError::terminal(msg, ChannelErrorKind::NotFound)
        }
        RequestError::Api(api) => match api {
            ApiError::BotBlocked
            | ApiError::BotKicked
            | ApiError::BotKickedFromSupergroup
            | ApiError::BotKickedFromChannel
            | ApiError::UserDeactivated
            | ApiError::CantInitiateConversation
            | ApiError::CantTalkWithBots
            | ApiError::NotEnoughRightsToPostMessages
            | ApiError::GroupDeactivated
            | ApiError::MethodNotAvailableInPrivateChats => {
                ChannelError::terminal(msg, ChannelErrorKind::Forbidden)
            }
            ApiError::ChatNotFound | ApiError::UserNotFound => {
                ChannelError::terminal(msg, ChannelErrorKind::NotFound)
            }
            ApiError::InvalidToken => {
                ChannelError::terminal(msg, ChannelErrorKind::Unauthorized)
            }
            ApiError::CantParseEntities(_)
            | ApiError::CantParseUrl
            | ApiError::WrongHttpUrl
            | ApiError::WrongFileId
            | ApiError::WrongFileIdOrUrl
            | ApiError::FailedToGetUrlContent
            | ApiError::ImageProcessFailed
            | ApiError::PhotoAsInputFileRequired
            | ApiError::ButtonUrlInvalid
            | ApiError::ButtonDataInvalid => {
                ChannelError::terminal(msg, ChannelErrorKind::PayloadInvalid)
            }
            ApiError::MessageIsTooLong
            | ApiError::EditedMessageIsTooLong
            | ApiError::TooMuchMessages
            | ApiError::RequestEntityTooLarge => {
                ChannelError::terminal(msg, ChannelErrorKind::PayloadTooLarge)
            }
            _ => ChannelError::transient(msg),
        },
        _ => ChannelError::transient(msg),
    }
}

impl TelegramAdapter {
    async fn send_bubble(&self, chat: &Chat, text: &str) -> Result<String, ChannelError> {
        self.send_bubble_with_keyboard(chat, text, None).await
    }

    async fn send_bubble_with_keyboard(
        &self,
        chat: &Chat,
        text: &str,
        keyboard: Option<InlineKeyboardMarkup>,
    ) -> Result<String, ChannelError> {
        let (chat_id, thread_id) = parse_external_id(external_chat_id(chat)?)?;

        // The keyboard rides on the final chunk so the user reads the whole
        // reply before deciding.
        let (chunks, parse_mode) = match telegram_markdown_v2::convert_with_strategy(
            &super::markdown::fence_tables(text),
            telegram_markdown_v2::UnsupportedTagsStrategy::Escape,
        ) {
            Ok(v2) => (self.splitter.split(&v2), Some(ParseMode::MarkdownV2)),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "telegram MarkdownV2 conversion failed; falling back to plain text",
                );
                (
                    super::split::silent_split_plain(
                        &super::markdown::to_plain(text),
                        TELEGRAM_MAX_MESSAGE_LEN,
                    ),
                    None,
                )
            }
        };

        if chunks.is_empty() {
            // No body to send. Callers ignore the id.
            return Ok(String::new());
        }

        let last_idx = chunks.len() - 1;
        let mut last_sent_id = String::new();
        for (i, chunk) in chunks.into_iter().enumerate() {
            let is_last = i == last_idx;
            let mut send = self.bot.send_message(Recipient::Id(chat_id), chunk.clone());
            if let Some(mode) = parse_mode {
                send = send.parse_mode(mode);
            }
            if let Some(t) = thread_id {
                send = send.message_thread_id(t);
            }
            if is_last && let Some(kb) = keyboard.clone() {
                send = send.reply_markup(kb);
            }
            match send.await {
                Ok(sent) => last_sent_id = sent.id.0.to_string(),
                Err(RequestError::Api(ApiError::CantParseEntities(detail)))
                    if parse_mode.is_some() =>
                {
                    tracing::warn!(
                        detail = %detail,
                        chunk_idx = i,
                        "Telegram rejected MarkdownV2 chunk; retrying as plain text",
                    );
                    let mut retry = self.bot.send_message(Recipient::Id(chat_id), chunk);
                    if let Some(t) = thread_id {
                        retry = retry.message_thread_id(t);
                    }
                    if is_last && let Some(kb) = keyboard.clone() {
                        retry = retry.reply_markup(kb);
                    }
                    match retry.await {
                        Ok(sent) => last_sent_id = sent.id.0.to_string(),
                        Err(e) => return Err(classify_telegram_error(&e)),
                    }
                }
                Err(e) => return Err(classify_telegram_error(&e)),
            }
        }
        Ok(last_sent_id)
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
    ) -> Result<(), ChannelError> {
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
        ctx: &ChannelCtx,
    ) -> Result<(), ChannelError> {
        let body = crate::chat::channel::render::render_message_body(msg);
        let has_attachments = !msg.attachments.is_empty();

        if !has_attachments {
            if !body.trim().is_empty() {
                self.send_bubble(chat, &body).await?;
            }
            return Ok(());
        }

        let (chat_id, thread_id) = parse_external_id(external_chat_id(chat)?)?;

        let mut photo_group: Vec<InputMedia> = Vec::new();
        let mut audio_atts = Vec::new();
        let mut video_atts = Vec::new();
        let mut doc_atts = Vec::new();
        for att in &msg.attachments {
            match attachment::classify(att) {
                attachment::AttachmentKind::Image => {
                    match attachment::read_attachment_bytes(att, ctx).await {
                        Ok(bytes) => {
                            let f = InputFile::memory(bytes).file_name(att.filename.clone());
                            photo_group.push(InputMedia::Photo(InputMediaPhoto::new(f)));
                        }
                        Err(e) => {
                            tracing::warn!(
                                msg_id = %msg.id,
                                path = %att.path,
                                error = %e,
                                "telegram: failed to read image bytes; skipping",
                            );
                        }
                    }
                }
                attachment::AttachmentKind::Audio => audio_atts.push(att),
                attachment::AttachmentKind::Video => video_atts.push(att),
                attachment::AttachmentKind::Document => doc_atts.push(att),
            }
        }

        // Built before the body send so it can ride on the body bubble — or,
        // if the body is empty, on the paperclip-only fallback bubble below.
        let mut doc_keyboard: Option<InlineKeyboardMarkup> = None;
        if !doc_atts.is_empty() {
            let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::with_capacity(doc_atts.len());
            for att in &doc_atts {
                let url_str = match attachment::outbound_url(att, ctx, attachment::ChannelMode::Button).await {
                    Ok(u) => u,
                    Err(e) => {
                        tracing::warn!(msg_id = %msg.id, path = %att.path, error = %e, "telegram: canonical URL failed; skipping doc");
                        continue;
                    }
                };
                let Ok(parsed) = Url::parse(&url_str) else {
                    tracing::warn!(msg_id = %msg.id, url = %url_str, "telegram: unparseable canonical URL; skipping doc");
                    continue;
                };
                let label = attachment::button_label(att);
                rows.push(vec![InlineKeyboardButton::url(label, parsed)]);
            }
            if !rows.is_empty() {
                doc_keyboard = Some(InlineKeyboardMarkup::new(rows));
            }
        }

        let mut keyboard_consumed = false;
        if !body.trim().is_empty() {
            self.send_bubble_with_keyboard(chat, &body, doc_keyboard.clone()).await?;
            keyboard_consumed = doc_keyboard.is_some();
        }

        // sendMediaGroup is capped at 10 items per album.
        for chunk in photo_group.chunks(10) {
            if chunk.len() == 1 {
                if let InputMedia::Photo(p) = &chunk[0] {
                    let mut req = self.bot.send_photo(Recipient::Id(chat_id), p.media.clone());
                    if let Some(t) = thread_id {
                        req = req.message_thread_id(t);
                    }
                    req.await.map_err(|e| classify_telegram_error(&e))?;
                }
            } else {
                let mut req = self
                    .bot
                    .send_media_group(Recipient::Id(chat_id), chunk.to_vec());
                if let Some(t) = thread_id {
                    req = req.message_thread_id(t);
                }
                req.await.map_err(|e| classify_telegram_error(&e))?;
            }
        }

        for att in audio_atts {
            let bytes = attachment::read_attachment_bytes(att, ctx).await?;
            let f = InputFile::memory(bytes).file_name(att.filename.clone());
            let mut req = self.bot.send_audio(Recipient::Id(chat_id), f);
            if let Some(t) = thread_id {
                req = req.message_thread_id(t);
            }
            req.await.map_err(|e| classify_telegram_error(&e))?;
        }

        for att in video_atts {
            let bytes = attachment::read_attachment_bytes(att, ctx).await?;
            let f = InputFile::memory(bytes).file_name(att.filename.clone());
            let mut req = self.bot.send_video(Recipient::Id(chat_id), f);
            if let Some(t) = thread_id {
                req = req.message_thread_id(t);
            }
            req.await.map_err(|e| classify_telegram_error(&e))?;
        }

        if !keyboard_consumed && let Some(kb) = doc_keyboard {
            // sendMessage requires non-empty text; this paperclip is the
            // smallest valid placeholder.
            let mut req = self
                .bot
                .send_message(Recipient::Id(chat_id), "📎".to_string())
                .reply_markup(kb);
            if let Some(t) = thread_id {
                req = req.message_thread_id(t);
            }
            if let Err(e) = req.await {
                tracing::warn!(msg_id = %msg.id, error = %e, "telegram doc-button send_message failed");
            }
        }

        Ok(())
    }

    async fn on_inference_start(
        &self,
        chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), ChannelError> {
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
    ) -> Result<(), ChannelError> {
        self.typing.stop(&chat.id).await;
        Ok(())
    }

    async fn on_webhook(
        &self,
        ctx: &ChannelCtx,
        request: Request<Bytes>,
    ) -> Result<Response, ChannelError> {
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
    ) -> Result<Vec<crate::inference::hitl::HitlDelivery>, ChannelError> {
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

            let (chunks, parse_mode) = match telegram_markdown_v2::convert_with_strategy(
                &super::markdown::fence_tables(&h.prompt),
                telegram_markdown_v2::UnsupportedTagsStrategy::Escape,
            ) {
                Ok(v2) => (self.splitter.split(&v2), Some(ParseMode::MarkdownV2)),
                Err(_) => (
                    super::split::silent_split_plain(
                        &super::markdown::to_plain(&h.prompt),
                        TELEGRAM_MAX_MESSAGE_LEN,
                    ),
                    None,
                ),
            };

            if chunks.is_empty() {
                continue;
            }

            let last_idx = chunks.len() - 1;
            let mut send_err: Option<RequestError> = None;
            let mut last_sent_id = String::new();
            for (i, chunk) in chunks.into_iter().enumerate() {
                let is_last = i == last_idx;
                let mut send = self.bot.send_message(Recipient::Id(chat_id), chunk);
                if let Some(mode) = parse_mode {
                    send = send.parse_mode(mode);
                }
                if let Some(t) = thread_id {
                    send = send.message_thread_id(t);
                }
                if is_last {
                    send = send.reply_markup(keyboard.clone());
                }
                match send.await {
                    Ok(sent) => last_sent_id = sent.id.0.to_string(),
                    Err(e) => {
                        send_err = Some(e);
                        break;
                    }
                }
            }

            match send_err {
                None => out.push(crate::inference::hitl::HitlDelivery {
                    channel_id: ctx.channel.id.clone(),
                    external_message_id: last_sent_id,
                    delivered_at: chrono::Utc::now(),
                }),
                Some(e) => {
                    tracing::warn!(
                        tool_call_id = %tc.id,
                        error = %e,
                        "Telegram on_pending_hitl: send failed",
                    );
                    if out.is_empty() {
                        return Err(classify_telegram_error(&e));
                    }
                    break;
                }
            }
        }
        Ok(out)
    }
}

/// Build the inline keyboard for a HITL prompt. Approval → Yes/No + URL
/// fallback (the only Approval today is App-deploy, where the user needs the
/// link to inspect the manifest). Choice → one button per option (no URL —
/// per-option buttons ARE the resolve action). External → URL only.
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
        HitlKind::Approval => {
            let mut rows = vec![vec![
                InlineKeyboardButton::callback("Yes".to_string(), format!("r:{tcid}:y")),
                InlineKeyboardButton::callback("No".to_string(), format!("r:{tcid}:n")),
            ]];
            if let Some(row) = url_button() {
                rows.push(row);
            }
            InlineKeyboardMarkup::new(rows)
        }
        HitlKind::Choice { options } => InlineKeyboardMarkup::new(
            options
                .iter()
                .enumerate()
                .map(|(i, opt)| {
                    vec![InlineKeyboardButton::callback(
                        opt.clone(),
                        format!("r:{tcid}:c:{i}"),
                    )]
                })
                .collect::<Vec<_>>(),
        ),
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
    fn table_converts_to_monospace_block_without_escaped_pipes() {
        let input = "Here you go:\n\n| a | b |\n| - | - |\n| 1 | 2 |";
        let out = telegram_markdown_v2::convert_with_strategy(
            &super::super::markdown::fence_tables(input),
            telegram_markdown_v2::UnsupportedTagsStrategy::Escape,
        )
        .unwrap();
        assert!(out.contains("```"), "table must render as code block: {out}");
        assert!(!out.contains("\\|"), "no escaped pipes outside code: {out}");
    }

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
    fn build_inline_keyboard_approval_has_yes_no_and_url_row() {
        use crate::chat::channel::hitl::HitlKind;
        let kb = build_inline_keyboard("tc-1", &HitlKind::Approval, "https://x/chats/abc");
        assert_eq!(kb.inline_keyboard.len(), 2);
        assert_eq!(kb.inline_keyboard[0].len(), 2);
        let labels: Vec<&str> = kb.inline_keyboard[0]
            .iter()
            .map(|b| b.text.as_str())
            .collect();
        assert!(labels.contains(&"Yes"));
        assert!(labels.contains(&"No"));
        assert_eq!(kb.inline_keyboard[1][0].text, "Open on web →");
    }

    #[test]
    fn build_inline_keyboard_choice_has_only_option_rows() {
        use crate::chat::channel::hitl::HitlKind;
        let kb = build_inline_keyboard(
            "tc-1",
            &HitlKind::Choice {
                options: vec!["us".to_string(), "eu".to_string()],
            },
            "https://x/chats/abc",
        );
        assert_eq!(kb.inline_keyboard.len(), 2);
        assert_eq!(kb.inline_keyboard[0][0].text, "us");
        assert_eq!(kb.inline_keyboard[1][0].text, "eu");
    }

    #[test]
    fn build_inline_keyboard_external_has_only_url_button() {
        use crate::chat::channel::hitl::HitlKind;
        let kb = build_inline_keyboard("tc-1", &HitlKind::External, "https://x/chats/abc");
        assert_eq!(kb.inline_keyboard.len(), 1);
        assert_eq!(kb.inline_keyboard[0][0].text, "Open on web →");
    }
}
