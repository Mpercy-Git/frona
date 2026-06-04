use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde::Deserialize;
use serenity::Error as SerenityError;
use serenity::all::{
    ChannelId, Client, Context, CreateMessage, EventHandler, GatewayIntents, Http,
    Message as DiscordMessage, UserId,
};
use serenity::http::HttpError;

use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;

use super::super::models::{
    ChannelAdapter, ChannelCtx, ExternalMessage, external_chat_id,
};
#[cfg(test)]
use super::super::models::ChannelFactory;

// Discord API cap. https://discord.com/developers/docs/resources/message
const DISCORD_MAX_MESSAGE_LEN: usize = 2000;
const DISCORD_CHUNK_TARGET: usize = 1900;

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
}

impl From<DiscordConfig> for DiscordAdapter {
    fn from(cfg: DiscordConfig) -> Self {
        let http = Arc::new(Http::new(&cfg.bot_token));
        Self {
            bot_token: cfg.bot_token,
            http,
            self_id: Arc::new(OnceLock::new()),
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
        let channel_id = parse_external_id(external_chat_id(chat)?)?;
        if let Err(e) = channel_id.broadcast_typing(&*self.http).await {
            tracing::debug!(
                channel_id = %channel_id,
                error = %e,
                "Discord broadcast_typing failed (non-fatal)",
            );
        }
        Ok(())
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
