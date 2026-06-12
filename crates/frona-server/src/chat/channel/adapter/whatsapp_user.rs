use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;

use super::super::models::{
    ChannelAdapter, ChannelCtx, ExternalMessage, SetupConfig, external_chat_id,
};
use super::super::typing::TypingIndicator;
#[cfg(test)]
use super::super::models::ChannelFactory;

use wa_rs::bot::Bot;
use wa_rs::client::Client;
use wa_rs::types::events::Event;
use wa_rs_sqlite_storage::SqliteStore;
use wa_rs_tokio_transport::TokioWebSocketTransportFactory;
use wa_rs_ureq_http::UreqHttpClient;

/// WhatsApp's composing indicator auto-fades around 10s. Refresh just under
/// that so a long inference keeps showing "typing…" without spamming.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WhatsAppUserConfig {}

#[derive(crate::ChannelFactory)]
#[channel(id = "whatsapp_user", from = WhatsAppUserConfig)]
pub struct WhatsAppUserAdapter {
    client: Mutex<Option<Arc<Client>>>,
    typing: TypingIndicator,
}

impl From<WhatsAppUserConfig> for WhatsAppUserAdapter {
    fn from(_: WhatsAppUserConfig) -> Self {
        Self {
            client: Mutex::new(None),
            typing: TypingIndicator::new(),
        }
    }
}

#[async_trait]
impl ChannelAdapter for WhatsAppUserAdapter {
    async fn on_setup_begin(
        &self,
        ctx: &ChannelCtx,
    ) -> Result<Option<SetupConfig>, AppError> {
        // Building a bot when paired makes WhatsApp kick one of the sessions
        // with `conflict=replaced`, breaking delivery.
        if is_already_paired(&ctx.data_dir).await {
            tracing::info!(
                channel_id = %ctx.channel.id,
                "WhatsApp Personal already paired - skipping setup, falling through to on_connect",
            );
            return Ok(None);
        }
        let (client, qr) = build_and_run_bot(ctx, /* expect_setup */ true).await?;
        *self.client.lock().await = Some(client);
        tracing::info!(
            channel_id = %ctx.channel.id,
            has_qr = %qr.is_some(),
            "WhatsApp Personal setup started - awaiting QR scan",
        );
        Ok(Some(SetupConfig {
            qr,
            code: None,
            instructions: Some(
                "Open WhatsApp on your phone, go to Settings → Linked Devices → \
                 Link a Device, and scan the QR code shown above."
                    .into(),
            ),
            expires_at: Some(Utc::now() + chrono::Duration::seconds(60)),
            initiated_at: None,
        }))
    }

    async fn on_setup_complete(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        // wa-rs has already persisted device keys; nothing else to do.
        tracing::info!(
            channel_id = %ctx.channel.id,
            "WhatsApp Personal device linked - wa-rs keys persisted",
        );
        Ok(())
    }

    async fn on_connect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        let (client, _qr) = build_and_run_bot(ctx, /* expect_setup */ false).await?;
        *self.client.lock().await = Some(client);
        tracing::info!(
            channel_id = %ctx.channel.id,
            "WhatsApp Personal connected (re-using persisted device keys)",
        );
        Ok(())
    }

    async fn on_disconnect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        // The cancel token + spawned bot.run task exiting close the
        // WebSocket cleanly - just drop our client handle.
        self.client.lock().await.take();
        tracing::info!(channel_id = %ctx.channel.id, "WhatsApp Personal disconnected");
        Ok(())
    }

    async fn on_inference_start(&self, chat: &Chat, _ctx: &ChannelCtx) -> Result<(), AppError> {
        let Some(client) = self.client.lock().await.clone() else { return Ok(()) };
        let Ok(external) = external_chat_id(chat) else { return Ok(()) };
        let Ok(to_raw) = parse_external_id(external) else { return Ok(()) };
        let Ok(to) = to_raw.parse::<wa_rs::Jid>() else { return Ok(()) };

        self.typing.start(chat.id.clone(), TYPING_REFRESH_INTERVAL, move || {
            let client = client.clone();
            let to = to.clone();
            async move {
                if let Err(e) = client.chatstate().send_composing(&to).await {
                    tracing::debug!(error = %e, "WhatsApp Personal send_composing failed (best-effort)");
                }
            }
        }).await;
        Ok(())
    }

    async fn on_inference_done(&self, chat: &Chat, ctx: &ChannelCtx) -> Result<(), AppError> {
        self.typing.stop(&chat.id).await;
        let Some(client) = self.client.lock().await.clone() else { return Ok(()) };
        let Ok(external) = external_chat_id(chat) else { return Ok(()) };
        let Ok(to_raw) = parse_external_id(external) else { return Ok(()) };
        let Ok(to) = to_raw.parse::<wa_rs::Jid>() else { return Ok(()) };
        if let Err(e) = client.chatstate().send_paused(&to).await {
            tracing::debug!(
                channel_id = %ctx.channel.id,
                error = %e,
                "WhatsApp Personal send_paused failed (best-effort)",
            );
        }
        Ok(())
    }

    async fn on_send(
        &self,
        msg: &Message,
        _tool_calls: &[crate::inference::tool_call::ToolCall],
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        if msg.content.trim().is_empty() {
            // Nothing to send; media attachments aren't wired yet.
            return Ok(());
        }
        let client = self.require_client(ctx, &msg.id).await?;
        let to = resolve_send_jid(&client, chat, ctx).await?;
        let body = super::markdown::to_whatsapp(&msg.content);
        let request_id = send_text(&client, &to, body).await.inspect_err(|e| {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                msg_id = %msg.id,
                to = %to,
                error = %e,
                "WhatsApp Personal send_message failed (check connectivity / device unlinked)",
            );
        })?;
        tracing::info!(
            channel_id = %ctx.channel.id,
            msg_id = %msg.id,
            to = %to,
            wa_request_id = %request_id,
            "WhatsApp Personal message sent",
        );
        Ok(())
    }

    async fn on_pending_hitl(
        &self,
        batch: &[crate::inference::tool_call::ToolCall],
        _msg: &Message,
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<Vec<crate::inference::hitl::HitlDelivery>, AppError> {
        // wa_rs is text-only. Sequential cadence: render only the first
        // pending HITL. The delivery cursor advances by 1; the next pending
        // HITL renders after this one resolves (via text reply or web URL).
        let Some(tc) = batch.first() else { return Ok(Vec::new()) };
        let Some(h) = tc.hitl.as_ref() else { return Ok(Vec::new()) };

        let client = self.require_client(ctx, &tc.id).await?;
        let to = resolve_send_jid(&client, chat, ctx).await?;
        // Body policy mirrors Discord/Slack: Choice/Approval → prompt only
        // (text reply IS the resolve action); External (creds) → prompt + URL
        // (URL is the only resolve path for vault picks).
        let kind = crate::chat::channel::hitl::kind_for(&h.request);
        let raw_body = match kind {
            crate::chat::channel::hitl::HitlKind::External => {
                crate::chat::channel::hitl::render_default_text(h)
            }
            _ => h.prompt.clone(),
        };
        let body = super::markdown::to_whatsapp(&raw_body);
        let request_id = send_text(&client, &to, body).await.inspect_err(|e| {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                tool_call_id = %tc.id,
                to = %to,
                error = %e,
                "whatsapp_user on_pending_hitl: send failed",
            );
        })?;
        tracing::info!(
            channel_id = %ctx.channel.id,
            tool_call_id = %tc.id,
            to = %to,
            wa_request_id = %request_id,
            "whatsapp_user HITL prompt sent",
        );
        Ok(vec![crate::inference::hitl::HitlDelivery {
            channel_id: ctx.channel.id.clone(),
            external_message_id: request_id,
            delivered_at: chrono::Utc::now(),
        }])
    }
}

impl WhatsAppUserAdapter {
    async fn require_client(
        &self,
        ctx: &ChannelCtx,
        log_id: &str,
    ) -> Result<Arc<Client>, AppError> {
        self.client.lock().await.clone().ok_or_else(|| {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                id = %log_id,
                "WhatsApp Personal send aborted - client not initialised (channel not Connected?)",
            );
            AppError::Internal("whatsapp_user client not initialised".into())
        })
    }
}

/// Parse the chat's stored JID and resolve LID → PN for 1:1 sends.
/// WhatsApp silently drops 1:1 stanzas addressed to a peer's LID; groups
/// (`@g.us`) are NOT LIDs and stay as-is.
async fn resolve_send_jid(
    client: &Client,
    chat: &Chat,
    ctx: &ChannelCtx,
) -> Result<wa_rs::Jid, AppError> {
    let to_raw = parse_external_id(external_chat_id(chat)?)?;
    let stored: wa_rs::Jid = to_raw
        .parse()
        .map_err(|e| AppError::Validation(format!("invalid WhatsApp JID {to_raw:?}: {e}")))?;
    if !stored.is_lid() {
        return Ok(stored);
    }
    match client.get_phone_number_from_lid(&stored.user).await {
        Some(pn_user) => {
            let pn_str = format!("{pn_user}@s.whatsapp.net");
            match pn_str.parse::<wa_rs::Jid>() {
                Ok(pn_jid) => {
                    tracing::debug!(
                        channel_id = %ctx.channel.id,
                        lid = %stored,
                        pn = %pn_jid,
                        "whatsapp_user resolved peer LID -> PN for 1:1 send",
                    );
                    Ok(pn_jid)
                }
                Err(_) => Ok(stored),
            }
        }
        None => {
            tracing::debug!(
                channel_id = %ctx.channel.id,
                lid = %stored,
                "whatsapp_user no LID->PN mapping; sending to LID",
            );
            Ok(stored)
        }
    }
}

async fn send_text(
    client: &Client,
    to: &wa_rs::Jid,
    body: String,
) -> Result<String, AppError> {
    let payload = wa_rs_proto::whatsapp::Message {
        conversation: Some(body),
        ..Default::default()
    };
    client
        .send_message(to.clone(), payload)
        .await
        .map_err(|e| AppError::Internal(format!("whatsapp_user send failed: {e}")))
}

/// When `expect_setup`, also returns the QR string emitted on first connect.
async fn build_and_run_bot(
    ctx: &ChannelCtx,
    expect_setup: bool,
) -> Result<(Arc<Client>, Option<String>), AppError> {
    let db_path = ctx.data_dir.join("session.db");
    let db_str = db_path
        .to_str()
        .ok_or_else(|| AppError::Internal("non-UTF8 wa-rs session path".into()))?
        .to_string();

    let backend = Arc::new(
        SqliteStore::new(&db_str)
            .await
            .map_err(|e| AppError::Internal(format!("wa-rs SqliteStore init: {e}")))?,
    );

    let (qr_tx, qr_rx) = if expect_setup {
        let (tx, rx) = oneshot::channel::<String>();
        (Some(Arc::new(Mutex::new(Some(tx)))), Some(rx))
    } else {
        (None, None)
    };

    let channel_id = ctx.channel.id.clone();
    let channel_manager = ctx.channel_manager.clone();
    let chat_service = ctx.chat_service.clone();
    let emit = ctx.emit.clone();

    // Device label shows in WhatsApp → Linked Devices. Platform type
    // stays `Desktop` - anything exotic risks tripping anti-abuse heuristics.
    let device_label = Some(super::resolve_device_label(ctx).await);

    let mut bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(TokioWebSocketTransportFactory::new())
        .with_http_client(UreqHttpClient::new())
        .with_device_props(
            device_label,
            None,
            Some(wa_rs_proto::whatsapp::device_props::PlatformType::Desktop),
        )
        .on_event(move |event, _client| {
            let qr_tx = qr_tx.clone();
            let channel_id = channel_id.clone();
            let channel_manager = channel_manager.clone();
            let chat_service = chat_service.clone();
            let emit = emit.clone();
            async move {
                match event {
                    Event::Connected(_) => {
                        tracing::info!(
                            channel_id = %channel_id,
                            "WhatsApp Personal WebSocket connected (handshake complete)",
                        );
                    }
                    Event::PairingQrCode { code, .. } => {
                        tracing::info!(
                            channel_id = %channel_id,
                            "WhatsApp Personal QR code emitted from wa-rs",
                        );
                        if let Some(slot) = qr_tx
                            && let Some(tx) = slot.lock().await.take()
                        {
                            let _ = tx.send(code);
                        }
                    }
                    Event::PairSuccess(_) => {
                        tracing::info!(
                            channel_id = %channel_id,
                            "WhatsApp Personal pair-success received from wa-rs",
                        );
                        channel_manager.report_setup_complete(&channel_id).await;
                    }
                    Event::Message(msg, info) => {
                        // WhatsApp echoes our own sends back on the same
                        // socket so other linked devices can sync.
                        if info.source.is_from_me {
                            return;
                        }
                        // Media isn't wired through yet - skip empty bodies
                        // rather than ship a no-content user message.
                        let text = msg
                            .conversation
                            .as_deref()
                            .or_else(|| {
                                msg.extended_text_message
                                    .as_ref()
                                    .and_then(|m| m.text.as_deref())
                            })
                            .unwrap_or("");
                        if text.is_empty() {
                            tracing::debug!(
                                channel_id = %channel_id,
                                "whatsapp_user inbound has no text body; skipping",
                            );
                            return;
                        }
                        let quoted_id = msg
                            .extended_text_message
                            .as_ref()
                            .and_then(|m| m.context_info.as_ref())
                            .and_then(|c| c.stanza_id.clone());
                        let sender = info.source.sender.to_string();
                        let chat_id = info.source.chat.to_string();
                        let external_chat_id = format!("wa:{chat_id}");

                        // HITL resolve pre-pass: quote-reply (or single
                        // pending) consumes the message instead of emitting a
                        // fresh user turn.
                        if let Ok(Some(chat)) = chat_service
                            .find_chat_by_channel_external_id(&channel_id, &external_chat_id)
                            .await
                        {
                            match crate::chat::channel::hitl::try_resolve_inbound(
                                &chat_service,
                                &channel_manager,
                                &chat.id,
                                quoted_id.as_deref(),
                                text,
                            )
                            .await
                            {
                                Ok(Some(crate::inference::hitl::ResolveOutcome::Resolved { .. })) => {
                                    tracing::info!(
                                        channel_id = %channel_id,
                                        wa_chat = %chat_id,
                                        quoted_id = ?quoted_id,
                                        "WhatsApp Personal inbound consumed as HITL resolution",
                                    );
                                    return;
                                }
                                Ok(Some(crate::inference::hitl::ResolveOutcome::AlreadyResolved)) => {
                                    tracing::info!(
                                        channel_id = %channel_id,
                                        wa_chat = %chat_id,
                                        "WhatsApp Personal inbound matched an already-resolved HITL; skipping",
                                    );
                                    return;
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    tracing::warn!(
                                        channel_id = %channel_id,
                                        error = %e,
                                        "WhatsApp Personal try_resolve_inbound failed; falling through to emit",
                                    );
                                }
                            }
                        }

                        tracing::info!(
                            channel_id = %channel_id,
                            from = %sender,
                            wa_chat = %chat_id,
                            "WhatsApp Personal inbound accepted - emitting to inbound pipeline",
                        );
                        let event = ExternalMessage {
                            external_chat_id,
                            sender_address: sender.clone(),
                            sender_external_id: Some(sender),
                            sender_display_name: None,
                            content: text.to_string(),
                            attachments: vec![],
                        };
                        if let Err(e) = emit.send(event).await {
                            tracing::warn!(
                                channel_id = %channel_id,
                                error = %e,
                                "WhatsApp Personal inbound emit failed (pipeline closed)",
                            );
                        }
                    }
                    _ => {}
                }
            }
        })
        .build()
        .await
        .map_err(|e| AppError::Internal(format!("wa-rs Bot build: {e}")))?;

    let client = bot.client();
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        let handle = match bot.run().await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "whatsapp_user bot.run failed");
                return;
            }
        };
        tokio::select! {
            r = handle => {
                if let Err(e) = r {
                    tracing::warn!(error = %e, "whatsapp_user bot task panicked");
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("whatsapp_user bot cancelled");
            }
        }
    });

    let qr = match qr_rx {
        Some(rx) => match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(code)) => Some(code),
            Ok(Err(_)) => return Err(AppError::Internal("wa-rs QR channel closed".into())),
            Err(_) => {
                return Err(AppError::Internal(
                    "timed out waiting for wa-rs QR code".into(),
                ));
            }
        },
        None => None,
    };

    Ok((client, qr))
}

/// Falls back to `false` on any I/O failure so callers re-enter the QR flow
/// rather than silently skipping a needed setup.
async fn is_already_paired(data_dir: &std::path::Path) -> bool {
    use wa_rs::store::traits::DeviceStore;
    let db_path = data_dir.join("session.db");
    if !db_path.exists() {
        return false;
    }
    let Some(db_str) = db_path.to_str() else {
        tracing::warn!(
            db_path = %db_path.display(),
            "whatsapp_user is_already_paired: non-UTF8 path, treating as not paired",
        );
        return false;
    };
    let store = match SqliteStore::new(db_str).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                db_path = %db_path.display(),
                error = %e,
                "whatsapp_user is_already_paired: store open failed, treating as not paired",
            );
            return false;
        }
    };
    match store.load().await {
        Ok(Some(device)) => device.pn.is_some(),
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(
                db_path = %db_path.display(),
                error = %e,
                "whatsapp_user is_already_paired: device load failed, treating as not paired",
            );
            false
        }
    }
}

fn parse_external_id(s: &str) -> Result<String, AppError> {
    s.strip_prefix("wa:")
        .filter(|rest| !rest.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Validation(format!("unrecognised WhatsApp external_id: {s:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn manifest_id_and_no_fields() {
        let m = WhatsAppUserAdapterFactory.manifest();
        assert_eq!(m.id, "whatsapp_user");
        assert!(m.config_fields.is_empty());
        assert!(m.setup_instructions.is_some());
        let urls: Vec<&str> = m.external_links.iter().map(|l| l.url.as_str()).collect();
        assert!(urls.iter().any(|u| u.contains("terms-of-service")));
        assert!(urls.iter().any(|u| u.contains("privacy-policy")));
    }

    #[test]
    fn factory_create_with_empty_config_succeeds() {
        WhatsAppUserAdapterFactory
            .create(json!({}))
            .expect("empty config should yield an adapter");
    }

    #[test]
    fn parse_external_id_strips_prefix() {
        assert_eq!(
            parse_external_id("wa:15551234567@s.whatsapp.net").unwrap(),
            "15551234567@s.whatsapp.net"
        );
        assert!(parse_external_id("foo:bar").is_err());
        assert!(parse_external_id("wa:").is_err());
    }
}
