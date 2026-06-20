//! Signal channel adapter.
//!
//! Pure-Rust integration via `presage` + `presage-store-sqlite`. v1 supports:
//! QR-based device linking, DMs and groups (in/out), typing indicators, and
//! outbound read receipts. Phone-number registration and attachment bytes are
//! deferred (see the plan doc).
//!
//! The closest in-tree reference is `adapter/whatsapp_user.rs`; the structural
//! divergence is the threading model — see `worker.rs` for the constraint.
//!
//! # Known issue: first-contact senders silently dropped
//!
//! Upstream presage (rev `6793c3e`, current HEAD as of 2026-05-19) has a bug
//! in its receive-side cipher routing: envelopes destined for our **PNI**
//! (Phone Number Identifier) are misrouted to the ACI cipher and rejected
//! with `mismatching destination service id`. The envelope is silently
//! dropped — no error reaches our adapter, no message reaches the agent.
//! Fix lives in PR #395 (https://github.com/whisperfish/presage/pull/395),
//! open since 2026-04-05, no maintainer engagement.
//!
//! ## When the bug fires
//!
//! A sender's Signal app encrypts to whichever identity it has cached for
//! our number. It learns our **ACI** (the bug-free path) when:
//!   - Our number is saved as a contact on their phone → Signal's Contact
//!     Discovery Service (CDSI) resolves phone → ACI and caches it
//!   - They have received a message from us (ACI is in the envelope source)
//!   - They opened a 1:1 chat and Signal fetched our profile
//!
//! If none of those happened, their app falls back to encrypting to our
//! **PNI**, which triggers the bug. In practice this means:
//!   - Messages from established contacts (saved your number, exchanged
//!     messages before): **work fine**
//!   - Messages from group conversations where both sides are mutually
//!     saved: **work fine**
//!   - First message from a brand-new sender who typed our phone number
//!     directly without saving it as a contact: **silently dropped**
//!
//! ## User-facing workaround (document in the setup guide)
//!
//! Tell users that anyone who wants to message the linked number must
//! **save it as a contact on their phone first**, then send. Signal's CDSI
//! lookup runs automatically when a contact is added; once the sender's
//! cache has our ACI, all subsequent messages are ACI-destined and decrypt
//! correctly.
//!
//! ## When to drop the workaround
//!
//! Re-pin `presage` to a rev that includes PR #395 (or our own fork
//! carrying the cherry-pick). At that point this comment block, the
//! "Known issue" callout in the user guide, and the test-from-an-unsaved-
//! contact failure mode all become obsolete.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use tokio::sync::{Mutex, oneshot};

use crate::chat::channel::attachment;
use crate::chat::channel::error::ChannelError;
use crate::chat::channel::models::{
    ChannelAdapter, ChannelCtx, SetupConfig, external_chat_id,
};
#[cfg(test)]
use crate::chat::channel::models::ChannelFactory;
use crate::chat::channel::typing::TypingIndicator;
use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;
use crate::inference::tool_call::ToolCall;

pub mod command;
pub mod convert;
pub mod external_id;
pub mod worker;

use command::{SignalCommand, TypingAction};
use external_id::SignalTarget;
use worker::SignalHandle;

/// Signal's typing indicator auto-fades around 15s. Refresh just under so a
/// long inference keeps showing "typing…" continuously.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SignalConfig {}

/// Practical body cap for a single Signal `DataMessage`. The libsignal
/// proto accepts more, but clients chunk visually above ~2k characters.
const SIGNAL_MAX_MESSAGE_LEN: usize = 2000;

#[derive(crate::ChannelFactory)]
#[channel(id = "signal", from = SignalConfig)]
pub struct SignalAdapter {
    handle: Mutex<Option<SignalHandle>>,
    typing: TypingIndicator,
    splitter: super::split::SignalSplitter,
}

impl From<SignalConfig> for SignalAdapter {
    fn from(_: SignalConfig) -> Self {
        Self {
            handle: Mutex::new(None),
            typing: TypingIndicator::new(),
            splitter: super::split::SignalSplitter::new(
                super::split::MarkdownSplitter::new(SIGNAL_MAX_MESSAGE_LEN, None),
            ),
        }
    }
}

#[async_trait]
impl ChannelAdapter for SignalAdapter {
    async fn on_setup_begin(
        &self,
        ctx: &ChannelCtx,
    ) -> Result<Option<SetupConfig>, ChannelError> {
        // `link_secondary_device` would wipe the existing registration, so
        // skip setup entirely when the store already has one.
        let db_path = ctx.data_dir.join("store.db");
        if worker::is_already_registered(&db_path).await {
            tracing::info!(
                channel_id = %ctx.channel.id,
                db_path = %db_path.display(),
                "Signal already linked - skipping setup, falling through to on_connect",
            );
            return Ok(None);
        }

        let device_name = super::resolve_device_label(ctx).await;
        let (handle, qr) = worker::spawn(ctx, device_name, /* expect_setup */ true).await?;
        *self.handle.lock().await = Some(handle);

        let qr = qr.ok_or_else(|| {
            AppError::Internal("Signal worker did not emit a link URL".into())
        })?;
        tracing::info!(
            channel_id = %ctx.channel.id,
            has_qr = true,
            "Signal setup started - awaiting QR scan",
        );
        Ok(Some(SetupConfig {
            qr: Some(qr),
            code: None,
            instructions: Some(
                "Open Signal on your phone → Settings → Linked devices → Link new device, \
                 then scan this QR. Linking completes automatically once Signal acknowledges \
                 the scan."
                    .into(),
            ),
            expires_at: Some(Utc::now() + chrono::Duration::seconds(120)),
            initiated_at: None,
        }))
    }

    async fn on_setup_complete(&self, _ctx: &ChannelCtx) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn on_connect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        let device_name = super::resolve_device_label(ctx).await;
        let (handle, _) = worker::spawn(ctx, device_name, /* expect_setup */ false).await?;
        *self.handle.lock().await = Some(handle);
        tracing::info!(
            channel_id = %ctx.channel.id,
            "Signal connected (re-using persisted device keys)",
        );
        Ok(())
    }

    async fn on_disconnect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        if let Some(h) = self.handle.lock().await.take()
            && let Some(t) = h.thread
        {
            // spawn_blocking so the join doesn't park a tokio worker.
            let channel_id = ctx.channel.id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = t.join() {
                    tracing::warn!(channel_id = %channel_id, error = ?e, "Signal worker thread panicked on join");
                }
            })
            .await;
        }
        tracing::info!(channel_id = %ctx.channel.id, "Signal disconnected");
        Ok(())
    }

    async fn on_tool(
        &self,
        tool_call: &ToolCall,
        msg: &Message,
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<(), ChannelError> {
        let Some(text) = tool_call.turn_text.as_deref() else {
            return Ok(());
        };
        if text.trim().is_empty() {
            return Ok(());
        }
        self.dispatch_text(chat, text, &msg.id, ctx).await.map(|_| ())
    }

    async fn on_send(
        &self,
        msg: &Message,
        _tool_calls: &[ToolCall],
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<(), ChannelError> {
        let body = crate::chat::channel::render::render_message_body(msg);
        let has_attachments = !msg.attachments.is_empty();
        if body.trim().is_empty() && !has_attachments {
            return Ok(());
        }
        let mut combined = body.clone();
        for att in &msg.attachments {
            match attachment::outbound_url(att, ctx, attachment::ChannelMode::Inline).await {
                Ok(url) => {
                    let line = attachment::inline_list_line(att, &url);
                    if !combined.is_empty() {
                        combined.push_str("\n\n");
                    }
                    combined.push_str(&line);
                }
                Err(e) => {
                    tracing::warn!(
                        channel_id = %ctx.channel.id,
                        msg_id = %msg.id,
                        path = %att.path,
                        error = %e,
                        "signal: share_url issue failed; skipping attachment",
                    );
                }
            }
        }
        if combined.trim().is_empty() {
            return Ok(());
        }
        self.dispatch_text(chat, &combined, &msg.id, ctx).await.map(|_| ())
    }

    async fn on_inference_start(&self, chat: &Chat, ctx: &ChannelCtx) -> Result<(), ChannelError> {
        let Ok(target) = SignalTarget::parse(match external_chat_id(chat) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        }) else {
            return Ok(());
        };
        let Ok(cmd_tx) = self.cmd_tx(&ctx.channel.id).await else {
            return Ok(());
        };
        self.typing.start(chat.id.clone(), TYPING_REFRESH_INTERVAL, move || {
            let cmd_tx = cmd_tx.clone();
            let target = target.clone();
            async move {
                let _ = cmd_tx
                    .send(SignalCommand::SendTyping {
                        target,
                        action: TypingAction::Started,
                    })
                    .await;
            }
        }).await;
        Ok(())
    }

    async fn on_inference_done(&self, chat: &Chat, ctx: &ChannelCtx) -> Result<(), ChannelError> {
        self.typing.stop(&chat.id).await;
        self.dispatch_typing(chat, ctx, TypingAction::Stopped).await
    }

    async fn on_pending_hitl(
        &self,
        batch: &[ToolCall],
        _msg: &Message,
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<Vec<crate::inference::hitl::HitlDelivery>, ChannelError> {
        // Sequential cadence: render only the first pending HITL. The cursor
        // advances by 1; the next pending HITL renders after this one resolves
        // (via text reply or web URL).
        let Some(tc) = batch.first() else { return Ok(Vec::new()) };
        let Some(h) = tc.hitl.as_ref() else { return Ok(Vec::new()) };

        let body = crate::chat::channel::hitl::render_text(h);
        let ts = self.dispatch_text(chat, &body, &tc.id, ctx).await?;
        tracing::info!(
            channel_id = %ctx.channel.id,
            tool_call_id = %tc.id,
            signal_ts = ts,
            "Signal HITL prompt sent",
        );
        Ok(vec![crate::inference::hitl::HitlDelivery {
            channel_id: ctx.channel.id.clone(),
            external_message_id: ts.to_string(),
            delivered_at: Utc::now(),
        }])
    }
}

impl SignalAdapter {
    async fn cmd_tx(
        &self,
        channel_id: &str,
    ) -> Result<tokio::sync::mpsc::Sender<SignalCommand>, AppError> {
        self.handle
            .lock()
            .await
            .as_ref()
            .map(|h| h.cmd_tx.clone())
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "Signal adapter for {channel_id} has no live worker (channel not connected?)"
                ))
            })
    }

    async fn dispatch_text(
        &self,
        chat: &Chat,
        text: &str,
        msg_id: &str,
        ctx: &ChannelCtx,
    ) -> Result<u64, ChannelError> {
        let target = SignalTarget::parse(external_chat_id(chat)?)?;
        let cmd_tx = self.cmd_tx(&ctx.channel.id).await?;
        let chunks = self.splitter.split(text);
        if chunks.is_empty() {
            return Ok(0);
        }
        let (reply, rx) = oneshot::channel();
        cmd_tx
            .send(SignalCommand::SendText {
                target,
                chunks,
                msg_id: msg_id.to_string(),
                reply,
            })
            .await
            .map_err(|_| {
                ChannelError::transient("Signal worker command channel closed")
            })?;
        rx.await
            .map_err(|_| ChannelError::transient("Signal worker dropped reply oneshot"))?
    }

    async fn dispatch_typing(
        &self,
        chat: &Chat,
        ctx: &ChannelCtx,
        action: TypingAction,
    ) -> Result<(), ChannelError> {
        // Best-effort: never propagate errors, they'd block delivery.
        let Ok(target) = SignalTarget::parse(match external_chat_id(chat) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        }) else {
            return Ok(());
        };
        let Ok(cmd_tx) = self.cmd_tx(&ctx.channel.id).await else {
            return Ok(());
        };
        if let Err(e) = cmd_tx.send(SignalCommand::SendTyping { target, action }).await {
            tracing::debug!(
                channel_id = %ctx.channel.id,
                error = %e,
                "signal typing dispatch failed (best-effort)",
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn manifest_id_is_signal() {
        let m = SignalAdapterFactory.manifest();
        assert_eq!(m.id, "signal");
        assert_eq!(m.display_name, "Signal");
        assert!(m.setup_instructions.is_some());
    }

    #[test]
    fn manifest_external_links_include_tos_and_privacy() {
        let m = SignalAdapterFactory.manifest();
        let urls: Vec<&str> = m.external_links.iter().map(|l| l.url.as_str()).collect();
        assert!(urls.iter().any(|u| u.contains("signal.org/legal")));
        assert!(urls.iter().any(|u| u.contains("signal.org/privacy")));
    }

    #[test]
    fn factory_create_with_empty_config_succeeds() {
        SignalAdapterFactory
            .create(json!({}))
            .expect("empty config should yield an adapter");
    }
}
