use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serenity::Error as SerenityError;
use serenity::all::{
    ButtonStyle, ChannelId, Client, Context, CreateActionRow, CreateButton,
    CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage, EventHandler,
    GatewayIntents, Http, Interaction, Message as DiscordMessage, UserId,
};
use serenity::http::HttpError;

use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;

use super::super::models::{
    ChannelAdapter, ChannelCtx, ExternalMessage, external_chat_id,
};
use super::super::typing::TypingIndicator;
#[cfg(test)]
use super::super::models::ChannelFactory;

// Discord API cap. https://discord.com/developers/docs/resources/message
const DISCORD_MAX_MESSAGE_LEN: usize = 2000;
const DISCORD_CHUNK_TARGET: usize = 1900;

/// Discord's typing indicator auto-fades in ~10s. Refresh a bit early so a
/// long inference keeps showing "typing…" continuously.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Deserialize)]
pub struct DiscordConfig {
    pub bot_token: String,
}

#[derive(crate::ChannelFactory)]
#[channel(id = "discord", from = DiscordConfig)]
pub struct DiscordAdapter {
    bot_token: String,
    http: Arc<Http>,
    self_id: Arc<OnceLock<UserId>>,
    typing: TypingIndicator,
}

impl From<DiscordConfig> for DiscordAdapter {
    fn from(cfg: DiscordConfig) -> Self {
        let http = Arc::new(Http::new(&cfg.bot_token));
        Self {
            bot_token: cfg.bot_token,
            http,
            self_id: Arc::new(OnceLock::new()),
            typing: TypingIndicator::new(),
        }
    }
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    async fn on_connect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        let me = self.http.get_current_user().await.map_err(|e| {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                error = %e,
                "Discord get_current_user failed — bot_token rejected",
            );
            AppError::Validation(format!("Discord rejected the bot_token: {e}"))
        })?;
        let _ = self.self_id.set(me.id);

        tracing::info!(
            channel_id = %ctx.channel.id,
            discord_user_id = %me.id,
            username = %me.name,
            "Discord bot authenticated",
        );

        let handler = DiscordEventHandler {
            emit: ctx.emit.clone(),
            channel_id_log: ctx.channel.id.clone(),
            self_id: self.self_id.clone(),
            channel_manager: ctx.channel_manager.clone(),
            chat_service: ctx.chat_service.clone(),
        };
        let intents = GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;
        let mut client = Client::builder(&self.bot_token, intents)
            .event_handler(handler)
            .await
            .map_err(|e| AppError::Internal(format!("Discord client build failed: {e}")))?;
        let shard_manager = client.shard_manager.clone();

        let cancel = ctx.cancel.clone();
        let channel_id = ctx.channel.id.clone();
        let channel_manager = ctx.channel_manager.clone();
        tokio::spawn(async move {
            let outcome = tokio::select! {
                res = client.start() => GatewayOutcome::Stopped(res),
                _ = cancel.cancelled() => GatewayOutcome::Cancelled,
            };
            match outcome {
                GatewayOutcome::Stopped(Err(e)) => {
                    let reason = format!("Discord gateway failed: {e}");
                    tracing::warn!(channel_id = %channel_id, error = %e, "Discord gateway terminated");
                    channel_manager.report_failure(&channel_id, reason).await;
                }
                GatewayOutcome::Stopped(Ok(())) => {
                    tracing::info!(channel_id = %channel_id, "Discord gateway stopped cleanly");
                }
                GatewayOutcome::Cancelled => {
                    shard_manager.shutdown_all().await;
                    tracing::info!(
                        channel_id = %channel_id,
                        "Discord gateway shut down (channel cancelled)",
                    );
                }
            }
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

    async fn on_inference_start(
        &self,
        chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        let Ok(external_id) = external_chat_id(chat) else { return Ok(()) };
        let Ok(discord_channel_id) = parse_external_id(external_id) else { return Ok(()) };

        let http = self.http.clone();
        self.typing.start(chat.id.clone(), TYPING_REFRESH_INTERVAL, move || {
            let http = http.clone();
            async move {
                if let Err(e) = discord_channel_id.broadcast_typing(&*http).await {
                    tracing::debug!(
                        channel_id = %discord_channel_id,
                        error = %e,
                        "Discord broadcast_typing failed (best-effort)",
                    );
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

    async fn on_pending_hitl(
        &self,
        batch: &[crate::inference::tool_call::ToolCall],
        _msg: &Message,
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<Vec<crate::inference::hitl::HitlDelivery>, AppError> {
        let channel_id = parse_external_id(external_chat_id(chat)?)?;
        let mut out = Vec::with_capacity(batch.len());
        for tc in batch {
            let Some(h) = tc.hitl.as_ref() else { continue };
            let kind = crate::chat::channel::hitl::kind_for(&h.request);
            let body = h.prompt.clone();
            let components = build_discord_components(&tc.id, &kind, &h.url);
            let req = CreateMessage::new().content(body).components(components);
            match channel_id.send_message(&*self.http, req).await {
                Ok(sent) => out.push(crate::inference::hitl::HitlDelivery {
                    channel_id: ctx.channel.id.clone(),
                    external_message_id: sent.id.get().to_string(),
                    delivered_at: chrono::Utc::now(),
                }),
                Err(e) => {
                    let retryable = is_discord_retryable_error(&e);
                    tracing::warn!(
                        channel_id = %ctx.channel.id,
                        tool_call_id = %tc.id,
                        retryable = retryable,
                        error = %e,
                        "Discord on_pending_hitl: send failed",
                    );
                    if retryable {
                        return Err(AppError::Internal(format!(
                            "Discord send failed: {e}"
                        )));
                    }
                    break;
                }
            }
        }
        Ok(out)
    }
}

/// 5xx, 429, and non-HTTP errors (network, gateway) are transient — propagate
/// as `Err` so `record_segment_failure` schedules backoff retry. 4xx (except
/// 429) is permanent (validation, missing perms, unknown channel) — return
/// `Ok(partial)` so the batch parks instead of burning the retry budget.
fn is_discord_retryable_error(err: &SerenityError) -> bool {
    match err {
        SerenityError::Http(HttpError::UnsuccessfulRequest(resp)) => {
            let code = resp.status_code.as_u16();
            code == 429 || (500..=599).contains(&code)
        }
        _ => true,
    }
}

impl DiscordAdapter {
    async fn post_message(&self, chat: &Chat, text: &str) -> Result<(), AppError> {
        let channel_id = parse_external_id(external_chat_id(chat)?)?;
        for chunk in chunk_for_discord(text) {
            let req = CreateMessage::new().content(chunk);
            if let Err(e) = channel_id.send_message(&*self.http, req).await {
                return Err(map_send_error(e, channel_id));
            }
        }
        Ok(())
    }
}

enum GatewayOutcome {
    Stopped(Result<(), SerenityError>),
    Cancelled,
}

struct DiscordEventHandler {
    emit: tokio::sync::mpsc::Sender<ExternalMessage>,
    channel_id_log: String,
    self_id: Arc<OnceLock<UserId>>,
    channel_manager: Arc<super::super::ChannelManager>,
    chat_service: crate::chat::service::ChatService,
}

#[async_trait]
impl EventHandler for DiscordEventHandler {
    async fn message(&self, _ctx: Context, msg: DiscordMessage) {
        let Some(&self_id) = self.self_id.get() else {
            return;
        };
        if let Some(em) = convert_message(&msg, self_id)
            && let Err(e) = self.emit.send(em).await
        {
            tracing::warn!(
                channel_id = %self.channel_id_log,
                error = %e,
                "Discord inbound emit channel closed",
            );
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Component(component) = interaction else {
            return;
        };
        let custom_id = component.data.custom_id.clone();

        let parsed = crate::chat::channel::hitl::parse_resolve_callback_data(
            &custom_id,
            &self.chat_service,
        )
        .await;

        let (tool_call_id, response) = match parsed {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    channel_id = %self.channel_id_log,
                    data = %custom_id,
                    error = %e,
                    "Discord callback_data parse failed",
                );
                let resp = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .ephemeral(true)
                        .content("Could not interpret that action."),
                );
                let _ = component.create_response(&ctx.http, resp).await;
                return;
            }
        };

        let answer_label = crate::chat::channel::hitl::response_display(&response);
        let outcome = self
            .channel_manager
            .resolve_hitl(&tool_call_id, response)
            .await;

        let summary = match &outcome {
            Ok(crate::inference::hitl::ResolveOutcome::Resolved { .. }) => answer_label,
            Ok(crate::inference::hitl::ResolveOutcome::AlreadyResolved) => "Already resolved".to_string(),
            Err(e) => format!("Failed: {e}"),
        };

        let original = component.message.content.clone();
        let new_content = format!("{original}\n\n→ {summary}");
        let resp = CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new()
                .content(new_content)
                .components(Vec::new()),
        );
        if let Err(e) = component.create_response(&ctx.http, resp).await {
            tracing::warn!(
                channel_id = %self.channel_id_log,
                error = %e,
                "Discord interaction response failed",
            );
        }
    }
}

const DISCORD_LABEL_MAX: usize = 80;
const DISCORD_CHOICE_BUTTON_CAP: usize = 5;

fn truncate_label(s: &str) -> String {
    if s.chars().count() <= DISCORD_LABEL_MAX {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(DISCORD_LABEL_MAX - 1).collect();
        out.push('…');
        out
    }
}

/// Mirror of `telegram::build_inline_keyboard`. Approval → Yes/No row plus a
/// URL fallback (the only Approval path today is `manage_app` deploy, where
/// the user needs the link to inspect the manifest). Choice → option buttons
/// only — they ARE the resolve action. External → URL only. Discord rejects
/// link buttons with non-http(s) URLs, so URL rows silently drop if invalid.
fn build_discord_components(
    tcid: &str,
    kind: &crate::chat::channel::hitl::HitlKind,
    url: &str,
) -> Vec<CreateActionRow> {
    use crate::chat::channel::hitl::HitlKind;
    let url_row = || -> Option<CreateActionRow> {
        if url.starts_with("http://") || url.starts_with("https://") {
            Some(CreateActionRow::Buttons(vec![
                CreateButton::new_link(url).label("Open on web →"),
            ]))
        } else {
            None
        }
    };
    match kind {
        HitlKind::Approval => {
            let mut rows = vec![CreateActionRow::Buttons(vec![
                CreateButton::new(format!("r:{tcid}:y"))
                    .label("Yes")
                    .style(ButtonStyle::Success),
                CreateButton::new(format!("r:{tcid}:n"))
                    .label("No")
                    .style(ButtonStyle::Danger),
            ])];
            if let Some(row) = url_row() {
                rows.push(row);
            }
            rows
        }
        HitlKind::Choice { options } => options
            .iter()
            .take(DISCORD_CHOICE_BUTTON_CAP)
            .enumerate()
            .map(|(i, opt)| {
                CreateActionRow::Buttons(vec![
                    CreateButton::new(format!("r:{tcid}:c:{i}"))
                        .label(truncate_label(opt))
                        .style(ButtonStyle::Primary),
                ])
            })
            .collect(),
        HitlKind::External => url_row().map(|r| vec![r]).unwrap_or_default(),
    }
}

fn convert_message(msg: &DiscordMessage, self_id: UserId) -> Option<ExternalMessage> {
    if should_skip(msg.author.id, msg.author.bot, &msg.content, self_id) {
        return None;
    }
    let display = msg
        .member
        .as_ref()
        .and_then(|m| m.nick.clone())
        .or_else(|| msg.author.global_name.clone())
        .or_else(|| Some(msg.author.name.clone()));
    Some(ExternalMessage {
        external_chat_id: build_external_chat_id(msg.channel_id, msg.guild_id.is_none()),
        sender_address: msg.author.id.to_string(),
        sender_external_id: Some(msg.author.id.to_string()),
        sender_display_name: display,
        content: msg.content.clone(),
        attachments: Vec::new(),
    })
}

fn should_skip(author_id: UserId, author_bot: bool, content: &str, self_id: UserId) -> bool {
    author_id == self_id || author_bot || content.trim().is_empty()
}

fn build_external_chat_id(channel_id: ChannelId, is_dm: bool) -> String {
    if is_dm {
        format!("dm:{channel_id}")
    } else {
        format!("group:{channel_id}")
    }
}

fn parse_external_id(s: &str) -> Result<ChannelId, AppError> {
    let (kind, id_str) = s.split_once(':').ok_or_else(|| {
        AppError::Validation(format!("unrecognised Discord external_id format: {s:?}"))
    })?;
    if !matches!(kind, "dm" | "group") || id_str.is_empty() {
        return Err(AppError::Validation(format!(
            "unrecognised Discord external_id format: {s:?}"
        )));
    }
    let id: u64 = id_str.parse().map_err(|_| {
        AppError::Validation(format!("invalid Discord channel id: {id_str}"))
    })?;
    Ok(ChannelId::new(id))
}

fn map_send_error(err: SerenityError, channel_id: ChannelId) -> AppError {
    if let SerenityError::Http(HttpError::UnsuccessfulRequest(resp)) = &err
        && resp.status_code.as_u16() == 403
    {
        return AppError::Validation(format!(
            "Discord rejected send_message on {channel_id}: bot lacks `View Channel` or `Send Messages` permission"
        ));
    }
    AppError::Internal(format!("Discord send_message failed: {err}"))
}

fn chunk_for_discord(text: &str) -> Vec<String> {
    if text.len() <= DISCORD_MAX_MESSAGE_LEN {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= DISCORD_MAX_MESSAGE_LEN {
            chunks.push(remaining.to_string());
            break;
        }
        let upper = remaining
            .char_indices()
            .take_while(|(i, _)| *i < DISCORD_CHUNK_TARGET)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(remaining.len());
        let slice = &remaining[..upper];
        let split_at = slice
            .rfind('\n')
            .map(|i| i + 1)
            .or_else(|| slice.rfind(' ').map(|i| i + 1))
            .unwrap_or(upper);
        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start();
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Tests that construct `DiscordAdapter::from(...)` must call this: rustls
    /// panics without an installed `CryptoProvider`, and tests don't go
    /// through `AppState::new` where prod installs it.
    fn install_crypto_provider() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }

    #[test]
    fn manifest_declares_required_secret_bot_token() {
        let m = DiscordAdapterFactory.manifest();
        assert_eq!(m.id, "discord");
        assert_eq!(m.display_name, "Discord");
        let f = m
            .config_fields
            .iter()
            .find(|f| f.name == "bot_token")
            .expect("bot_token field missing");
        assert!(f.is_required);
        assert!(f.is_secret);
    }

    #[test]
    fn factory_create_with_valid_config_succeeds() {
        install_crypto_provider();
        let cfg = json!({"bot_token": "abc.def.ghi"});
        DiscordAdapterFactory
            .create(cfg)
            .expect("valid config should produce a DiscordAdapter");
    }

    #[test]
    fn factory_create_rejects_missing_bot_token() {
        let cfg = json!({});
        assert!(matches!(
            DiscordAdapterFactory.create(cfg),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn parse_external_id_dm() {
        let c = parse_external_id("dm:123456789012345678").unwrap();
        assert_eq!(c.get(), 123456789012345678);
    }

    #[test]
    fn parse_external_id_group() {
        let c = parse_external_id("group:987654321098765432").unwrap();
        assert_eq!(c.get(), 987654321098765432);
    }

    #[test]
    fn parse_external_id_rejects_garbage() {
        assert!(parse_external_id("nonsense").is_err());
        assert!(parse_external_id("dm:").is_err());
        assert!(parse_external_id("group:").is_err());
        assert!(parse_external_id("group:not-a-number").is_err());
        assert!(parse_external_id("thread:123").is_err());
    }

    #[test]
    fn build_external_chat_id_dm() {
        assert_eq!(
            build_external_chat_id(ChannelId::new(42), true),
            "dm:42",
        );
    }

    #[test]
    fn build_external_chat_id_group() {
        assert_eq!(
            build_external_chat_id(ChannelId::new(999), false),
            "group:999",
        );
    }

    #[test]
    fn should_skip_self_message() {
        let me = UserId::new(1);
        assert!(should_skip(me, false, "hi", me));
    }

    #[test]
    fn should_skip_bot_author() {
        assert!(should_skip(UserId::new(2), true, "hi", UserId::new(1)));
    }

    #[test]
    fn should_skip_empty_content() {
        assert!(should_skip(UserId::new(2), false, "   ", UserId::new(1)));
    }

    #[test]
    fn should_not_skip_human_message() {
        assert!(!should_skip(
            UserId::new(2),
            false,
            "hello",
            UserId::new(1)
        ));
    }

    #[test]
    fn chunk_for_discord_under_limit_returns_one_chunk() {
        let chunks = chunk_for_discord("hello world");
        assert_eq!(chunks, vec!["hello world".to_string()]);
    }

    #[test]
    fn chunk_for_discord_splits_on_newline_boundary() {
        let line = "a".repeat(500);
        let blob = format!("{line}\n{line}\n{line}\n{line}\n{line}");
        let chunks = chunk_for_discord(&blob);
        assert!(chunks.len() >= 2, "expected at least 2 chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(c.len() <= DISCORD_MAX_MESSAGE_LEN, "chunk exceeds limit: {}", c.len());
        }
        let rejoined: String = chunks.join("\n");
        assert_eq!(rejoined.replace('\n', ""), blob.replace('\n', ""));
    }

    #[test]
    fn chunk_for_discord_falls_back_to_hard_split_when_no_boundary() {
        let blob = "x".repeat(2500);
        let chunks = chunk_for_discord(&blob);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.len() <= DISCORD_MAX_MESSAGE_LEN);
        }
        let rejoined: String = chunks.concat();
        assert_eq!(rejoined, blob);
    }
}
