use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;
use crate::storage::Attachment;

use super::storage::download_to_attachment;
use super::super::attachment;
use super::super::error::{ChannelError, ChannelErrorKind};
use super::super::models::{
    ChannelAdapter, ChannelCtx, ExternalMessage, external_chat_id,
};
#[cfg(test)]
use super::super::models::ChannelFactory;

pub(crate) const CLOUD_API_BASE: &str = "https://graph.facebook.com/v18.0";

#[derive(Debug, Clone, Deserialize)]
pub struct WhatsAppCloudConfig {
    pub phone_number_id: String,
    pub business_account_id: String,
    pub access_token: String,
    pub verify_token: String,
    pub app_secret: String,
}

/// WhatsApp Cloud `text` message body cap. Interactive components have
/// their own per-element caps (see `WA_BODY_TEXT_MAX` etc.) — those don't
/// flow through this splitter.
const WA_CLOUD_MAX_MESSAGE_LEN: usize = 4096;

#[derive(crate::ChannelFactory)]
#[channel(id = "whatsapp_cloud", from = WhatsAppCloudConfig)]
pub struct WhatsAppCloudAdapter {
    config: WhatsAppCloudConfig,
    http: reqwest::Client,
    splitter: super::split::MarkdownSplitter,
}

impl From<WhatsAppCloudConfig> for WhatsAppCloudAdapter {
    fn from(config: WhatsAppCloudConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            splitter: super::split::MarkdownSplitter::new(WA_CLOUD_MAX_MESSAGE_LEN, None),
        }
    }
}

#[async_trait]
impl ChannelAdapter for WhatsAppCloudAdapter {
    async fn on_connect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        let url = format!("{CLOUD_API_BASE}/{}", self.config.phone_number_id);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.config.access_token)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(
                    channel_id = %ctx.channel.id,
                    error = %e,
                    "WhatsApp Cloud reachability check failed (network/DNS?)",
                );
                AppError::Internal(format!("WA cloud reachability check failed: {e}"))
            })?;
        if !resp.status().is_success() {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                status = %resp.status(),
                "WhatsApp Cloud rejected access_token - channel will be marked Failed (check access_token / phone_number_id)",
            );
            return Err(AppError::Validation(format!(
                "WhatsApp Cloud rejected access_token (status {})",
                resp.status(),
            )));
        }

        let sub_url = format!(
            "{CLOUD_API_BASE}/{}/subscribed_apps",
            self.config.business_account_id,
        );
        match self
            .http
            .post(&sub_url)
            .bearer_auth(&self.config.access_token)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                tracing::info!(channel_id = %ctx.channel.id, "WhatsApp Cloud subscribed_apps OK");
            }
            Ok(r) => tracing::warn!(
                channel_id = %ctx.channel.id,
                status = %r.status(),
                "WhatsApp Cloud subscribed_apps returned non-2xx (continuing)",
            ),
            Err(e) => tracing::warn!(
                channel_id = %ctx.channel.id,
                error = %e,
                "WhatsApp Cloud subscribed_apps call failed (continuing)",
            ),
        }
        tracing::info!(
            channel_id = %ctx.channel.id,
            phone_number_id = %self.config.phone_number_id,
            webhook_url = %ctx.webhook_url,
            "WhatsApp Cloud channel registered (paste webhook_url into Meta dashboard if not done already)",
        );
        Ok(())
    }

    async fn on_disconnect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        tracing::info!(channel_id = %ctx.channel.id, "WhatsApp Cloud disconnected");
        Ok(())
    }

    async fn on_send(
        &self,
        msg: &Message,
        _tool_calls: &[crate::inference::tool_call::ToolCall],
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<(), ChannelError> {
        let to = parse_external_id(external_chat_id(chat)?)?;
        let raw_body = crate::chat::channel::render::render_message_body(msg);
        let body = if raw_body.trim().is_empty() {
            String::new()
        } else {
            super::markdown::to_whatsapp(&raw_body)
        };

        // Plain text path — no attachments.
        if msg.attachments.is_empty() {
            if body.is_empty() {
                return Ok(());
            }
            for chunk in self.splitter.split(&body) {
                self.send_text(&to, &chunk).await?;
            }
            tracing::info!(
                channel_id = %ctx.channel.id,
                msg_id = %msg.id,
                to = %to,
                "WhatsApp Cloud text message sent",
            );
            return Ok(());
        }

        // Sequential per-attachment delivery. Body only on message 1; subsequent
        // messages carry `📄 {filename}` as body/caption (WA Cloud APIs require
        // non-empty bodies on cta_url).
        let mut body_consumed = false;
        for att in &msg.attachments {
            let kind = attachment::classify(att);
            let attachment_body = if !body_consumed && !body.is_empty() {
                body_consumed = true;
                body.clone()
            } else {
                format!("📄 {}", att.filename)
            };

            match kind {
                attachment::AttachmentKind::Image
                | attachment::AttachmentKind::Audio
                | attachment::AttachmentKind::Video => {
                    let bytes = match attachment::read_attachment_bytes(att, ctx).await
                    {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!(
                                channel_id = %ctx.channel.id,
                                msg_id = %msg.id,
                                path = %att.path,
                                error = %e,
                                "WhatsApp Cloud: read_attachment_bytes failed; skipping",
                            );
                            continue;
                        }
                    };
                    let media_id = match self
                        .upload_media(&att.filename, &att.content_type, bytes)
                        .await
                    {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::warn!(
                                channel_id = %ctx.channel.id,
                                msg_id = %msg.id,
                                path = %att.path,
                                error = %e,
                                "WhatsApp Cloud: upload_media failed; skipping",
                            );
                            // If body was consumed by this failed attachment,
                            // hand it back to the next one.
                            if body_consumed && !body.is_empty() && attachment_body == body {
                                body_consumed = false;
                            }
                            continue;
                        }
                    };
                    if let Err(e) = self
                        .send_media(
                            &to,
                            &att.content_type,
                            &media_id,
                            Some(&attachment_body),
                        )
                        .await
                    {
                        tracing::warn!(
                            channel_id = %ctx.channel.id,
                            msg_id = %msg.id,
                            path = %att.path,
                            error = %e,
                            "WhatsApp Cloud: send_media failed; skipping",
                        );
                        if body_consumed && !body.is_empty() && attachment_body == body {
                            body_consumed = false;
                        }
                        continue;
                    }
                    tracing::info!(
                        channel_id = %ctx.channel.id,
                        msg_id = %msg.id,
                        to = %to,
                        content_type = %att.content_type,
                        "WhatsApp Cloud media sent",
                    );
                }
                attachment::AttachmentKind::Document => {
                    let url = match attachment::outbound_url(att, ctx, attachment::ChannelMode::Button).await
                    {
                        Ok(u) => u,
                        Err(e) => {
                            tracing::warn!(
                                channel_id = %ctx.channel.id,
                                msg_id = %msg.id,
                                path = %att.path,
                                error = %e,
                                "WhatsApp Cloud: canonical_file_url failed; skipping",
                            );
                            if body_consumed && !body.is_empty() && attachment_body == body {
                                body_consumed = false;
                            }
                            continue;
                        }
                    };
                    let display_text = attachment::button_label(att);
                    let payload = build_cta_url_payload(&to, &attachment_body, &display_text, &url);
                    if let Err(e) = self.send_message(payload).await {
                        tracing::warn!(
                            channel_id = %ctx.channel.id,
                            msg_id = %msg.id,
                            path = %att.path,
                            error = %e,
                            "WhatsApp Cloud: cta_url send failed; skipping",
                        );
                        if body_consumed && !body.is_empty() && attachment_body == body {
                            body_consumed = false;
                        }
                        continue;
                    }
                    tracing::info!(
                        channel_id = %ctx.channel.id,
                        msg_id = %msg.id,
                        to = %to,
                        "WhatsApp Cloud cta_url sent",
                    );
                }
            }
        }

        // If every attachment failed and body never got delivered, ship it as
        // a fallback text message so the user at least sees the prose.
        if !body_consumed && !body.is_empty() {
            for chunk in self.splitter.split(&body) {
                self.send_text(&to, &chunk).await?;
            }
        }

        Ok(())
    }

    async fn on_webhook(
        &self,
        ctx: &ChannelCtx,
        request: Request<Bytes>,
    ) -> Result<Response, ChannelError> {
        if request.method() == Method::GET {
            return self.handle_verify(request);
        }

        let signature = request
            .headers()
            .get("x-hub-signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        if !verify_signature(&self.config.app_secret, request.body(), &signature) {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                "WhatsApp Cloud webhook signature mismatch",
            );
            return Ok(StatusCode::UNAUTHORIZED.into_response());
        }

        let raw: serde_json::Value = serde_json::from_slice(request.body()).map_err(|e| {
            AppError::Validation(format!("invalid WhatsApp Cloud webhook body: {e}"))
        })?;
        self.emit_inbound(ctx, raw).await?;
        Ok(StatusCode::OK.into_response())
    }

    async fn on_pending_hitl(
        &self,
        batch: &[crate::inference::tool_call::ToolCall],
        _msg: &Message,
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<Vec<crate::inference::hitl::HitlDelivery>, ChannelError> {
        let to = parse_external_id(external_chat_id(chat)?)?;
        let mut out = Vec::with_capacity(batch.len());
        for tc in batch {
            let Some(h) = tc.hitl.as_ref() else { continue };
            let kind = crate::chat::channel::hitl::kind_for(&h.request);
            let payload = build_interactive_payload(&to, &tc.id, &h.prompt, &kind, &h.url);
            match self.send_message_capturing_id(payload).await {
                Ok(wamid) => out.push(crate::inference::hitl::HitlDelivery {
                    channel_id: ctx.channel.id.clone(),
                    external_message_id: wamid,
                    delivered_at: chrono::Utc::now(),
                }),
                Err(e) => {
                    tracing::warn!(
                        channel_id = %ctx.channel.id,
                        tool_call_id = %tc.id,
                        kind = ?e.kind,
                        error = %e.message,
                        "WhatsApp Cloud on_pending_hitl: send failed",
                    );
                    if !e.kind.is_terminal() && out.is_empty() {
                        return Err(e);
                    }
                    break;
                }
            }
        }
        Ok(out)
    }
}

impl WhatsAppCloudAdapter {
    fn handle_verify(&self, request: Request<Bytes>) -> Result<Response, ChannelError> {
        let mut mode: Option<String> = None;
        let mut token: Option<String> = None;
        let mut challenge: Option<String> = None;
        if let Some(query) = request.uri().query() {
            for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
                match k.as_ref() {
                    "hub.mode" => mode = Some(v.into_owned()),
                    "hub.verify_token" => token = Some(v.into_owned()),
                    "hub.challenge" => challenge = Some(v.into_owned()),
                    _ => {}
                }
            }
        }
        let (mode, token, challenge) = match (mode, token, challenge) {
            (Some(m), Some(t), Some(c)) => (m, t, c),
            _ => return Ok(StatusCode::BAD_REQUEST.into_response()),
        };
        let token_ok: bool = token
            .as_bytes()
            .ct_eq(self.config.verify_token.as_bytes())
            .into();
        if mode != "subscribe" || !token_ok {
            return Ok(StatusCode::FORBIDDEN.into_response());
        }
        Ok(([(axum::http::header::CONTENT_TYPE, "text/plain")], challenge).into_response())
    }

    async fn upload_media(
        &self,
        filename: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<String, AppError> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(content_type)
            .map_err(|e| AppError::Validation(format!("invalid content_type {content_type}: {e}")))?;
        let form = reqwest::multipart::Form::new()
            .text("messaging_product", "whatsapp")
            .text("type", content_type.to_string())
            .part("file", part);
        let url = format!("{CLOUD_API_BASE}/{}/media", self.config.phone_number_id);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.access_token)
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("WA media upload failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "WA media upload {status}: {body}",
            )));
        }
        #[derive(Deserialize)]
        struct UploadResp {
            id: String,
        }
        let parsed: UploadResp = resp
            .json()
            .await
            .map_err(|e| AppError::Internal(format!("WA media upload bad response: {e}")))?;
        Ok(parsed.id)
    }

    async fn send_text(&self, to: &str, body: &str) -> Result<(), ChannelError> {
        let payload = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "text",
            "text": { "body": body },
        });
        self.send_message(payload).await
    }

    async fn send_media(
        &self,
        to: &str,
        content_type: &str,
        media_id: &str,
        caption: Option<&str>,
    ) -> Result<(), ChannelError> {
        let media_kind = wa_media_kind(content_type);
        let mut media_obj = serde_json::json!({ "id": media_id });
        if matches!(media_kind, "image" | "document" | "video")
            && let Some(c) = caption
        {
            media_obj["caption"] = serde_json::Value::String(c.to_string());
        }
        let payload = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": media_kind,
            media_kind: media_obj,
        });
        self.send_message(payload).await
    }

    async fn send_message(&self, payload: serde_json::Value) -> Result<(), ChannelError> {
        let _ = self.send_message_capturing_id(payload).await?;
        Ok(())
    }

    /// Same wire call as `send_message`, but returns the provider message id
    /// (`messages[0].id`, aka `wamid`) so HITL deliveries can stamp
    /// `HitlDelivery.external_message_id`.
    async fn send_message_capturing_id(
        &self,
        payload: serde_json::Value,
    ) -> Result<String, ChannelError> {
        let url = format!("{CLOUD_API_BASE}/{}/messages", self.config.phone_number_id);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.access_token)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                classify_wa_cloud_error(
                    &WaCloudError::Transport,
                    format!("WA send failed: {e}"),
                )
            })?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let msg = format!("WA send {status}: {body}");
            return Err(classify_wa_cloud_error(
                &WaCloudError::from_http(status, &body),
                msg,
            ));
        }
        #[derive(Deserialize)]
        struct SendResp {
            #[serde(default)]
            messages: Vec<SentMessage>,
        }
        #[derive(Deserialize)]
        struct SentMessage {
            id: String,
        }
        let parsed: SendResp = resp.json().await.map_err(|e| {
            classify_wa_cloud_error(
                &WaCloudError::Transport,
                format!("WA send response parse failed: {e}"),
            )
        })?;
        Ok(parsed.messages.into_iter().next().map(|m| m.id).unwrap_or_default())
    }

    async fn emit_inbound(
        &self,
        ctx: &ChannelCtx,
        raw: serde_json::Value,
    ) -> Result<(), ChannelError> {
        let payload: WebhookPayload = serde_json::from_value(raw).map_err(|e| {
            AppError::Validation(format!("invalid WhatsApp Cloud webhook payload: {e}"))
        })?;
        for entry in payload.entry {
            for change in entry.changes {
                if change.field != "messages" {
                    continue;
                }
                let value = change.value;
                let contacts = value.contacts.unwrap_or_default();
                for message in value.messages.unwrap_or_default() {
                    if message.kind == "interactive"
                        && self.try_resolve_interactive(ctx, &message).await
                    {
                        continue;
                    }
                    let display_name = contacts
                        .iter()
                        .find(|c| c.wa_id == message.from)
                        .and_then(|c| c.profile.as_ref())
                        .map(|p| p.name.clone());
                    let event = match self.build_external_message(ctx, &message, display_name).await {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!(
                                channel_id = %ctx.channel.id,
                                msg_id = %message.id,
                                error = %e,
                                "WhatsApp Cloud inbound dropped (failed to build ExternalMessage)",
                            );
                            continue;
                        }
                    };
                    tracing::info!(
                        channel_id = %ctx.channel.id,
                        from = %message.from,
                        wa_msg_id = %message.id,
                        kind = %message.kind,
                        "WhatsApp Cloud webhook accepted - emitting to inbound pipeline",
                    );
                    ctx.emit.send(event).await.map_err(|e| {
                        AppError::Internal(format!("inbound emit channel closed: {e}"))
                    })?;
                }
            }
        }
        Ok(())
    }

    async fn build_external_message(
        &self,
        ctx: &ChannelCtx,
        message: &InboundMessage,
        display_name: Option<String>,
    ) -> Result<ExternalMessage, AppError> {
        let mut attachments = Vec::new();
        let mut content = String::new();

        match message.kind.as_str() {
            "text" => {
                if let Some(text) = &message.text {
                    content = text.body.clone();
                }
            }
            "image" | "audio" | "document" | "video" | "sticker" => {
                let media = match message.kind.as_str() {
                    "image" => message.image.as_ref(),
                    "audio" => message.audio.as_ref(),
                    "document" => message.document.as_ref(),
                    "video" => message.video.as_ref(),
                    "sticker" => message.sticker.as_ref(),
                    _ => None,
                };
                let Some(media) = media else {
                    return Err(AppError::Validation(format!(
                        "WhatsApp Cloud {} message missing media object",
                        message.kind,
                    )));
                };
                let attachment = self.download_media(ctx, media, &message.kind).await?;
                attachments.push(attachment);
                if let Some(caption) = &media.caption {
                    content = caption.clone();
                }
            }
            other => {
                tracing::debug!(
                    channel_id = %ctx.channel.id,
                    kind = %other,
                    "WhatsApp Cloud inbound kind not yet supported",
                );
                return Err(AppError::Validation(format!(
                    "unsupported WhatsApp Cloud message kind: {other}",
                )));
            }
        }

        Ok(ExternalMessage {
            external_chat_id: format!("wa:{}", message.from),
            sender_address: message.from.clone(),
            sender_external_id: Some(message.from.clone()),
            sender_display_name: display_name,
            content,
            attachments,
        })
    }

    async fn download_media(
        &self,
        ctx: &ChannelCtx,
        media: &MediaPayload,
        fallback_kind: &str,
    ) -> Result<Attachment, AppError> {
        // The returned URL is short-lived (~5 min) - fetch immediately.
        let meta_url = format!("{CLOUD_API_BASE}/{}", media.id);
        #[derive(Deserialize)]
        struct MediaMeta {
            url: String,
            #[serde(default)]
            mime_type: Option<String>,
        }
        let meta: MediaMeta = self
            .http
            .get(&meta_url)
            .bearer_auth(&self.config.access_token)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("WA media meta failed: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Internal(format!("WA media meta status: {e}")))?
            .json()
            .await
            .map_err(|e| AppError::Internal(format!("WA media meta parse: {e}")))?;

        let content_type = media
            .mime_type
            .clone()
            .or(meta.mime_type)
            .unwrap_or_else(|| default_mime_for(fallback_kind).to_string());
        let filename = media
            .filename
            .clone()
            .unwrap_or_else(|| default_filename(&media.id, &content_type));
        let handle = ctx
            .user_service
            .find_by_id(&ctx.channel.user_id)
            .await?
            .map(|u| u.handle)
            .ok_or_else(|| AppError::Validation(format!(
                "channel references missing user {:?}", ctx.channel.user_id
            )))?;
        download_to_attachment(
            &self.http,
            &ctx.storage_service,
            &handle,
            &meta.url,
            Some(&self.config.access_token),
            &filename,
            &content_type,
        )
        .await
    }

    /// Returns true when the inbound was consumed as a HITL resolution and
    /// should NOT be forwarded into the inbound pipeline. Returns false when
    /// the tap was unparseable / unknown / errored — the caller decides
    /// whether to fall through.
    async fn try_resolve_interactive(
        &self,
        ctx: &ChannelCtx,
        message: &InboundMessage,
    ) -> bool {
        let Some(interactive) = message.interactive.as_ref() else {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                msg_id = %message.id,
                "WhatsApp Cloud interactive message missing inner interactive payload",
            );
            return false;
        };
        let reply_id = match interactive.kind.as_str() {
            "button_reply" => interactive.button_reply.as_ref().map(|r| r.id.as_str()),
            "list_reply" => interactive.list_reply.as_ref().map(|r| r.id.as_str()),
            other => {
                tracing::debug!(
                    channel_id = %ctx.channel.id,
                    msg_id = %message.id,
                    kind = %other,
                    "WhatsApp Cloud interactive reply kind ignored",
                );
                return false;
            }
        };
        let Some(reply_id) = reply_id else {
            tracing::warn!(
                channel_id = %ctx.channel.id,
                msg_id = %message.id,
                kind = %interactive.kind,
                "WhatsApp Cloud interactive reply missing inner id",
            );
            return false;
        };

        let parsed = crate::chat::channel::hitl::parse_resolve_callback_data(
            reply_id,
            &ctx.chat_service,
        )
        .await;
        let (tool_call_id, response) = match parsed {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    channel_id = %ctx.channel.id,
                    reply_id = %reply_id,
                    error = %e,
                    "WhatsApp Cloud interactive reply id parse failed",
                );
                return false;
            }
        };
        let answer_label = crate::chat::channel::hitl::response_display(&response);
        match ctx.channel_manager.resolve_hitl(&tool_call_id, response).await {
            Ok(crate::inference::hitl::ResolveOutcome::Resolved { .. }) => {
                tracing::info!(
                    channel_id = %ctx.channel.id,
                    tool_call_id = %tool_call_id,
                    answer = %answer_label,
                    "WhatsApp Cloud interactive reply resolved HITL",
                );
            }
            Ok(crate::inference::hitl::ResolveOutcome::AlreadyResolved) => {
                tracing::info!(
                    channel_id = %ctx.channel.id,
                    tool_call_id = %tool_call_id,
                    "WhatsApp Cloud interactive reply hit an already-resolved HITL",
                );
            }
            Err(e) => {
                tracing::warn!(
                    channel_id = %ctx.channel.id,
                    tool_call_id = %tool_call_id,
                    error = %e,
                    "WhatsApp Cloud resolve_hitl failed",
                );
            }
        }
        true
    }
}

fn wa_media_kind(content_type: &str) -> &'static str {
    let lower = content_type.to_ascii_lowercase();
    if lower.starts_with("image/") {
        "image"
    } else if lower.starts_with("audio/") {
        "audio"
    } else if lower.starts_with("video/") {
        "video"
    } else {
        "document"
    }
}

fn default_mime_for(kind: &str) -> &'static str {
    match kind {
        "image" => "image/jpeg",
        "audio" => "audio/ogg",
        "video" => "video/mp4",
        "sticker" => "image/webp",
        _ => "application/octet-stream",
    }
}

fn default_filename(id: &str, content_type: &str) -> String {
    let ext = match content_type {
        ct if ct.starts_with("image/jpeg") => "jpg",
        ct if ct.starts_with("image/png") => "png",
        ct if ct.starts_with("image/webp") => "webp",
        ct if ct.starts_with("image/gif") => "gif",
        ct if ct.starts_with("audio/ogg") => "ogg",
        ct if ct.starts_with("audio/mpeg") => "mp3",
        ct if ct.starts_with("video/mp4") => "mp4",
        "application/pdf" => "pdf",
        _ => "bin",
    };
    format!("{id}.{ext}")
}

fn parse_external_id(s: &str) -> Result<String, AppError> {
    s.strip_prefix("wa:")
        .filter(|rest| !rest.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Validation(format!("unrecognised WhatsApp external_id: {s:?}")))
}

fn verify_signature(app_secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hex_part) = header.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(provided) = hex::decode(hex_part) else {
        return false;
    };
    let mut mac = match Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&provided).is_ok()
}

// Meta interactive-message field limits, per Cloud API docs:
//   developers.facebook.com/docs/whatsapp/cloud-api/messages/interactive-{reply-buttons,list,cta-url}-messages
const WA_REPLY_BUTTON_TITLE_MAX: usize = 20;
const WA_LIST_ROW_TITLE_MAX: usize = 24;
const WA_BODY_TEXT_MAX: usize = 1024;
const WA_LIST_ROWS_MAX: usize = 10;
const WA_REPLY_BUTTONS_MAX: usize = 3;

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

fn is_http_url(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

/// Build the JSON payload for a single HITL prompt. Per-kind dispatch:
/// - Approval → reply-button (Yes/No). URL goes in the body since Meta rejects
///   mixing reply buttons with a URL button in the same `interactive` payload.
/// - Choice (≤3 opts) → reply-button. Choice (4–10) → list. Choice (>10) →
///   list truncated to the first 10 rows with a notice in the body.
/// - Choice with empty options → plain `text` message (Cloud rejects empty
///   `buttons`/`rows`).
/// - External → `cta_url` (URL button). Falls back to plain text if the URL
///   isn't absolute http(s) (Cloud rejects non-http(s) URLs).
fn build_interactive_payload(
    to: &str,
    tcid: &str,
    prompt: &str,
    kind: &crate::chat::channel::hitl::HitlKind,
    url: &str,
) -> serde_json::Value {
    use crate::chat::channel::hitl::HitlKind;
    match kind {
        HitlKind::Approval => {
            let body_text = if is_http_url(url) {
                truncate_chars(&format!("{prompt}\n\n{url}"), WA_BODY_TEXT_MAX)
            } else {
                truncate_chars(prompt, WA_BODY_TEXT_MAX)
            };
            build_button_payload(
                to,
                &body_text,
                &[
                    (format!("r:{tcid}:y"), "Yes".to_string()),
                    (format!("r:{tcid}:n"), "No".to_string()),
                ],
            )
        }
        HitlKind::Choice { options } if options.is_empty() => build_text_payload(
            to,
            &truncate_chars(prompt, WA_BODY_TEXT_MAX),
        ),
        HitlKind::Choice { options } if options.len() <= WA_REPLY_BUTTONS_MAX => {
            let buttons: Vec<(String, String)> = options
                .iter()
                .enumerate()
                .map(|(i, opt)| {
                    (
                        format!("r:{tcid}:c:{i}"),
                        truncate_chars(opt, WA_REPLY_BUTTON_TITLE_MAX),
                    )
                })
                .collect();
            build_button_payload(to, &truncate_chars(prompt, WA_BODY_TEXT_MAX), &buttons)
        }
        HitlKind::Choice { options } => {
            let truncated = options.len() > WA_LIST_ROWS_MAX;
            let body_text = if truncated {
                let extra = options.len() - WA_LIST_ROWS_MAX;
                truncate_chars(
                    &format!("{prompt}\n\n(+{extra} more options, please pick from the list)"),
                    WA_BODY_TEXT_MAX,
                )
            } else {
                truncate_chars(prompt, WA_BODY_TEXT_MAX)
            };
            let rows: Vec<(String, String)> = options
                .iter()
                .take(WA_LIST_ROWS_MAX)
                .enumerate()
                .map(|(i, opt)| {
                    (
                        format!("r:{tcid}:c:{i}"),
                        truncate_chars(opt, WA_LIST_ROW_TITLE_MAX),
                    )
                })
                .collect();
            build_list_payload(to, &body_text, "Options", "Choose", &rows)
        }
        HitlKind::External => {
            if is_http_url(url) {
                build_cta_url_payload(
                    to,
                    &truncate_chars(prompt, WA_BODY_TEXT_MAX),
                    "Open on web",
                    url,
                )
            } else {
                build_text_payload(
                    to,
                    &truncate_chars(&format!("{prompt}\n\n{url}"), WA_BODY_TEXT_MAX),
                )
            }
        }
    }
}

fn build_text_payload(to: &str, body: &str) -> serde_json::Value {
    serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": to,
        "type": "text",
        "text": { "body": body },
    })
}

fn build_button_payload(
    to: &str,
    body_text: &str,
    buttons: &[(String, String)],
) -> serde_json::Value {
    let buttons_json: Vec<serde_json::Value> = buttons
        .iter()
        .map(|(id, title)| {
            serde_json::json!({
                "type": "reply",
                "reply": { "id": id, "title": title },
            })
        })
        .collect();
    serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": to,
        "type": "interactive",
        "interactive": {
            "type": "button",
            "body": { "text": body_text },
            "action": { "buttons": buttons_json },
        },
    })
}

fn build_list_payload(
    to: &str,
    body_text: &str,
    section_title: &str,
    action_button: &str,
    rows: &[(String, String)],
) -> serde_json::Value {
    let rows_json: Vec<serde_json::Value> = rows
        .iter()
        .map(|(id, title)| {
            serde_json::json!({ "id": id, "title": title })
        })
        .collect();
    serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": to,
        "type": "interactive",
        "interactive": {
            "type": "list",
            "body": { "text": body_text },
            "action": {
                "button": action_button,
                "sections": [ { "title": section_title, "rows": rows_json } ],
            },
        },
    })
}

fn build_cta_url_payload(
    to: &str,
    body_text: &str,
    display_text: &str,
    url: &str,
) -> serde_json::Value {
    serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": to,
        "type": "interactive",
        "interactive": {
            "type": "cta_url",
            "body": { "text": body_text },
            "action": {
                "name": "cta_url",
                "parameters": { "display_text": display_text, "url": url },
            },
        },
    })
}

/// Either a transport-level failure (network, bad JSON), or an HTTP response
/// whose body carries Meta's numeric `error.code`. Unknown codes default to
/// Transient; the cap bounds the retry budget.
/// See https://developers.facebook.com/docs/whatsapp/cloud-api/support/error-codes/
#[derive(Debug, Clone)]
enum WaCloudError {
    Transport,
    /// Upstream proxy/CDN responded directly (no Meta JSON body).
    HttpStatusOnly(u16),
    /// 190
    AccessTokenInvalid,
    /// 200..=299
    PermissionMissing,
    /// 100
    InvalidParameter,
    /// 131000
    MessageFailedGeneric,
    /// 131005
    AccessDenied,
    /// 131008
    RequiredParamMissing,
    /// 131009 — bad phone format.
    ParameterValueInvalid,
    /// 131021
    RecipientCannotBeSender,
    /// 131026 — recipient hasn't accepted WA business policy.
    MessageUndeliverable,
    /// 131031
    AccountLocked,
    /// 131047 — 24h session expired, must use template.
    ReEngagementRequired,
    /// 131048 — per phone-pair throttle.
    SpamRateLimit,
    /// 131051
    UnsupportedMessageType,
    /// 131052 — URL we passed didn't resolve.
    MediaDownloadError,
    /// 131053
    MediaUploadError,
    /// 131057
    AccountMaintenance,
    /// 130429 / 131056
    RateLimit,
    /// 132000..=132099 — param mismatch, unapproved, deleted.
    TemplateProblem,
    /// 133000..=133015 — registration / 2FA / phone number state.
    RegistrationProblem,
    Unknown,
}

#[derive(Debug, Deserialize)]
struct WaErrorBody {
    error: WaErrorPayload,
}

#[derive(Debug, Deserialize)]
struct WaErrorPayload {
    code: u32,
    #[serde(default)]
    #[allow(dead_code)] // useful for logs, not for classification
    message: Option<String>,
}

impl WaCloudError {
    fn from_http(status: u16, body: &str) -> Self {
        if let Ok(parsed) = serde_json::from_str::<WaErrorBody>(body) {
            return Self::from_code(parsed.error.code);
        }
        Self::HttpStatusOnly(status)
    }

    fn from_code(code: u32) -> Self {
        match code {
            190 => Self::AccessTokenInvalid,
            200..=299 => Self::PermissionMissing,
            100 => Self::InvalidParameter,
            131000 => Self::MessageFailedGeneric,
            131005 => Self::AccessDenied,
            131008 => Self::RequiredParamMissing,
            131009 => Self::ParameterValueInvalid,
            131021 => Self::RecipientCannotBeSender,
            131026 => Self::MessageUndeliverable,
            131031 => Self::AccountLocked,
            131047 => Self::ReEngagementRequired,
            131048 => Self::SpamRateLimit,
            131051 => Self::UnsupportedMessageType,
            131052 => Self::MediaDownloadError,
            131053 => Self::MediaUploadError,
            131057 => Self::AccountMaintenance,
            130429 | 131056 => Self::RateLimit,
            132000..=132099 => Self::TemplateProblem,
            133000..=133015 => Self::RegistrationProblem,
            _ => Self::Unknown,
        }
    }

    fn to_channel_error(&self, msg: String) -> ChannelError {
        use ChannelErrorKind::*;
        match self {
            Self::Transport | Self::Unknown | Self::MessageFailedGeneric => {
                ChannelError::transient(msg)
            }
            Self::RateLimit | Self::SpamRateLimit | Self::AccountMaintenance => {
                ChannelError::transient(msg)
            }
            Self::HttpStatusOnly(status) => match status {
                401 => ChannelError::terminal(msg, Unauthorized),
                403 => ChannelError::terminal(msg, Forbidden),
                404 => ChannelError::terminal(msg, NotFound),
                413 => ChannelError::terminal(msg, PayloadTooLarge),
                429 => ChannelError::transient(msg),
                500..=599 => ChannelError::transient(msg),
                400 | 422 => ChannelError::terminal(msg, PayloadInvalid),
                _ => ChannelError::transient(msg),
            },
            Self::AccessTokenInvalid => ChannelError::terminal(msg, Unauthorized),
            Self::PermissionMissing
            | Self::AccessDenied
            | Self::AccountLocked
            | Self::MessageUndeliverable
            | Self::ReEngagementRequired
            | Self::RegistrationProblem => ChannelError::terminal(msg, Forbidden),
            Self::RecipientCannotBeSender => ChannelError::terminal(msg, NotFound),
            Self::InvalidParameter
            | Self::RequiredParamMissing
            | Self::ParameterValueInvalid
            | Self::UnsupportedMessageType
            | Self::MediaDownloadError
            | Self::MediaUploadError
            | Self::TemplateProblem => ChannelError::terminal(msg, PayloadInvalid),
        }
    }
}

fn classify_wa_cloud_error(err: &WaCloudError, msg: String) -> ChannelError {
    err.to_channel_error(msg)
}

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    #[serde(default)]
    entry: Vec<WebhookEntry>,
}

#[derive(Debug, Deserialize)]
struct WebhookEntry {
    #[serde(default)]
    changes: Vec<WebhookChange>,
}

#[derive(Debug, Deserialize)]
struct WebhookChange {
    field: String,
    value: WebhookValue,
}

#[derive(Debug, Deserialize)]
struct WebhookValue {
    #[serde(default)]
    contacts: Option<Vec<InboundContact>>,
    #[serde(default)]
    messages: Option<Vec<InboundMessage>>,
}

#[derive(Debug, Deserialize)]
struct InboundContact {
    wa_id: String,
    #[serde(default)]
    profile: Option<ContactProfile>,
}

#[derive(Debug, Deserialize)]
struct ContactProfile {
    name: String,
}

#[derive(Debug, Deserialize)]
struct InboundMessage {
    id: String,
    from: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<TextPayload>,
    #[serde(default)]
    image: Option<MediaPayload>,
    #[serde(default)]
    audio: Option<MediaPayload>,
    #[serde(default)]
    document: Option<MediaPayload>,
    #[serde(default)]
    video: Option<MediaPayload>,
    #[serde(default)]
    sticker: Option<MediaPayload>,
    #[serde(default)]
    interactive: Option<InteractivePayload>,
}

#[derive(Debug, Deserialize)]
struct TextPayload {
    body: String,
}

#[derive(Debug, Deserialize)]
struct MediaPayload {
    id: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    caption: Option<String>,
}

/// Wraps a tap on an interactive `button` / `list` message. Only one of
/// `button_reply` / `list_reply` is populated depending on `kind`.
#[derive(Debug, Deserialize)]
struct InteractivePayload {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    button_reply: Option<InteractiveReply>,
    #[serde(default)]
    list_reply: Option<InteractiveReply>,
}

#[derive(Debug, Deserialize)]
struct InteractiveReply {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_config() -> serde_json::Value {
        json!({
            "phone_number_id": "p",
            "business_account_id": "b",
            "access_token": "t",
            "verify_token": "v",
            "app_secret": "s",
        })
    }

    #[test]
    fn manifest_has_required_fields() {
        let m = WhatsAppCloudAdapterFactory.manifest();
        assert_eq!(m.id, "whatsapp_cloud");
        assert!(m.webhook_url_visible);
        assert!(m.setup_instructions.is_some());
        assert!(
            !m.external_links.is_empty(),
            "manifest should advertise external policy/docs links",
        );
        let expected = [
            ("phone_number_id", false),
            ("business_account_id", false),
            ("access_token", true),
            ("verify_token", true),
            ("app_secret", true),
        ];
        for (name, is_secret) in expected {
            let f = m
                .config_fields
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("manifest missing field {name}"));
            assert!(f.is_required, "{name} should be required");
            assert_eq!(f.is_secret, is_secret, "{name} secrecy");
        }
    }

    #[test]
    fn factory_create_with_valid_config_succeeds() {
        WhatsAppCloudAdapterFactory
            .create(valid_config())
            .expect("valid config should produce an adapter");
    }

    #[test]
    fn factory_rejects_missing_required_field() {
        for field in [
            "phone_number_id",
            "business_account_id",
            "access_token",
            "verify_token",
            "app_secret",
        ] {
            let mut cfg = valid_config();
            cfg.as_object_mut().unwrap().remove(field);
            match WhatsAppCloudAdapterFactory.create(cfg) {
                Err(AppError::Validation(_)) => {}
                Err(e) => panic!("expected Validation for missing {field}, got: {e}"),
                Ok(_) => panic!("expected error for missing {field}, got Ok"),
            }
        }
    }

    #[test]
    fn parse_external_id_strips_prefix() {
        assert_eq!(parse_external_id("wa:+15551234567").unwrap(), "+15551234567");
        assert!(parse_external_id("sms:+15551234567").is_err());
        assert!(parse_external_id("wa:").is_err());
    }

    #[test]
    fn signature_verifies_known_vector() {
        let secret = "shh";
        let body = b"hello world";
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let header = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_signature(secret, body, &header));
        assert!(!verify_signature("other", body, &header));
        assert!(!verify_signature(secret, b"different body", &header));
    }

    #[test]
    fn signature_rejects_missing_prefix() {
        assert!(!verify_signature("s", b"x", "sha1=abc"));
        assert!(!verify_signature("s", b"x", ""));
    }

    #[test]
    fn wa_media_kind_buckets() {
        assert_eq!(wa_media_kind("image/png"), "image");
        assert_eq!(wa_media_kind("audio/ogg"), "audio");
        assert_eq!(wa_media_kind("video/mp4"), "video");
        assert_eq!(wa_media_kind("application/pdf"), "document");
        assert_eq!(wa_media_kind("application/octet-stream"), "document");
    }

    #[test]
    fn webhook_payload_parses_text_and_image() {
        let body = json!({
            "entry": [{
                "changes": [{
                    "field": "messages",
                    "value": {
                        "contacts": [{"wa_id": "15551234567", "profile": {"name": "Alice"}}],
                        "messages": [
                            {"id": "m1", "from": "15551234567", "type": "text", "text": {"body": "hi"}},
                            {"id": "m2", "from": "15551234567", "type": "image",
                             "image": {"id": "media1", "mime_type": "image/jpeg", "caption": "look"}}
                        ]
                    }
                }]
            }]
        });
        let parsed: WebhookPayload = serde_json::from_value(body).unwrap();
        let value = &parsed.entry[0].changes[0].value;
        let messages = value.messages.as_ref().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].kind, "text");
        assert_eq!(messages[0].text.as_ref().unwrap().body, "hi");
        assert_eq!(messages[1].kind, "image");
        let img = messages[1].image.as_ref().unwrap();
        assert_eq!(img.id, "media1");
        assert_eq!(img.caption.as_deref(), Some("look"));
    }

    use crate::chat::channel::hitl::HitlKind;

    #[test]
    fn approval_renders_yes_no_buttons_with_url_in_body() {
        let v = build_interactive_payload(
            "15551234567",
            "tc-1",
            "Deploy app foo?",
            &HitlKind::Approval,
            "https://app.example/chats/abc",
        );
        assert_eq!(v["type"], "interactive");
        assert_eq!(v["interactive"]["type"], "button");
        let body = v["interactive"]["body"]["text"].as_str().unwrap();
        assert!(body.contains("Deploy app foo?"));
        assert!(body.contains("https://app.example/chats/abc"));
        let buttons = v["interactive"]["action"]["buttons"].as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0]["reply"]["id"], "r:tc-1:y");
        assert_eq!(buttons[0]["reply"]["title"], "Yes");
        assert_eq!(buttons[1]["reply"]["id"], "r:tc-1:n");
        assert_eq!(buttons[1]["reply"]["title"], "No");
    }

    #[test]
    fn choice_le_three_renders_reply_buttons() {
        let kind = HitlKind::Choice {
            options: vec!["EU".into(), "US".into(), "APAC".into()],
        };
        let v = build_interactive_payload("15550000000", "tc-2", "Region?", &kind, "");
        assert_eq!(v["interactive"]["type"], "button");
        let buttons = v["interactive"]["action"]["buttons"].as_array().unwrap();
        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[0]["reply"]["id"], "r:tc-2:c:0");
        assert_eq!(buttons[0]["reply"]["title"], "EU");
        assert_eq!(buttons[2]["reply"]["id"], "r:tc-2:c:2");
    }

    #[test]
    fn choice_four_to_ten_renders_list() {
        let opts: Vec<String> = (0..6).map(|i| format!("opt-{i}")).collect();
        let kind = HitlKind::Choice { options: opts };
        let v = build_interactive_payload("15550000000", "tc-3", "Pick one", &kind, "");
        assert_eq!(v["interactive"]["type"], "list");
        let sections = v["interactive"]["action"]["sections"].as_array().unwrap();
        assert_eq!(sections.len(), 1);
        let rows = sections[0]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0]["id"], "r:tc-3:c:0");
        assert_eq!(rows[5]["id"], "r:tc-3:c:5");
        assert_eq!(v["interactive"]["action"]["button"], "Choose");
    }

    #[test]
    fn choice_over_ten_truncates_to_ten_rows() {
        let opts: Vec<String> = (0..15).map(|i| format!("opt-{i}")).collect();
        let kind = HitlKind::Choice { options: opts };
        let v = build_interactive_payload("15550000000", "tc-4", "Pick", &kind, "");
        assert_eq!(v["interactive"]["type"], "list");
        let rows = v["interactive"]["action"]["sections"][0]["rows"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 10);
        let body = v["interactive"]["body"]["text"].as_str().unwrap();
        assert!(body.contains("+5 more"));
    }

    #[test]
    fn choice_empty_options_falls_back_to_text() {
        let kind = HitlKind::Choice { options: vec![] };
        let v = build_interactive_payload("15550000000", "tc-5", "Anything?", &kind, "");
        assert_eq!(v["type"], "text");
        assert_eq!(v["text"]["body"], "Anything?");
        assert!(v.get("interactive").is_none());
    }

    #[test]
    fn external_with_http_url_renders_cta_url() {
        let v = build_interactive_payload(
            "15550000000",
            "tc-6",
            "Pick a credential",
            &HitlKind::External,
            "https://app.example/vault/pick?q=postgres",
        );
        assert_eq!(v["interactive"]["type"], "cta_url");
        assert_eq!(v["interactive"]["action"]["name"], "cta_url");
        assert_eq!(
            v["interactive"]["action"]["parameters"]["display_text"],
            "Open on web",
        );
        assert_eq!(
            v["interactive"]["action"]["parameters"]["url"],
            "https://app.example/vault/pick?q=postgres",
        );
    }

    #[test]
    fn external_with_non_http_url_falls_back_to_text() {
        let v = build_interactive_payload(
            "15550000000",
            "tc-7",
            "Pick a credential",
            &HitlKind::External,
            "vault://pick",
        );
        assert_eq!(v["type"], "text");
        let body = v["text"]["body"].as_str().unwrap();
        assert!(body.contains("Pick a credential"));
        assert!(body.contains("vault://pick"));
    }

    #[test]
    fn long_option_titles_truncate_to_meta_limits() {
        let long = "x".repeat(50);
        let kind = HitlKind::Choice {
            options: vec![long.clone()],
        };
        let v = build_interactive_payload("15550000000", "tc-8", "Pick", &kind, "");
        let title = v["interactive"]["action"]["buttons"][0]["reply"]["title"]
            .as_str()
            .unwrap();
        assert!(title.chars().count() <= 20, "got len {}", title.chars().count());
        assert!(title.ends_with('…'));
    }

    #[test]
    fn interactive_button_reply_payload_parses() {
        let body = json!({
            "entry": [{
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messages": [{
                            "id": "wamid.1", "from": "15551234567",
                            "type": "interactive",
                            "interactive": {
                                "type": "button_reply",
                                "button_reply": {"id": "r:tc-1:y", "title": "Yes"}
                            }
                        }]
                    }
                }]
            }]
        });
        let parsed: WebhookPayload = serde_json::from_value(body).unwrap();
        let msg = &parsed.entry[0].changes[0].value.messages.as_ref().unwrap()[0];
        assert_eq!(msg.kind, "interactive");
        let i = msg.interactive.as_ref().unwrap();
        assert_eq!(i.kind, "button_reply");
        assert_eq!(i.button_reply.as_ref().unwrap().id, "r:tc-1:y");
        assert!(i.list_reply.is_none());
    }

    #[test]
    fn interactive_list_reply_payload_parses() {
        let body = json!({
            "entry": [{
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messages": [{
                            "id": "wamid.2", "from": "15551234567",
                            "type": "interactive",
                            "interactive": {
                                "type": "list_reply",
                                "list_reply": {"id": "r:tc-2:c:3", "title": "opt-3"}
                            }
                        }]
                    }
                }]
            }]
        });
        let parsed: WebhookPayload = serde_json::from_value(body).unwrap();
        let msg = &parsed.entry[0].changes[0].value.messages.as_ref().unwrap()[0];
        let i = msg.interactive.as_ref().unwrap();
        assert_eq!(i.kind, "list_reply");
        assert_eq!(i.list_reply.as_ref().unwrap().id, "r:tc-2:c:3");
        assert!(i.button_reply.is_none());
    }

    #[test]
    fn wa_cloud_classifies_token_expired_as_unauthorized() {
        let body = r#"{"error":{"code":190,"message":"invalid token"}}"#;
        let e = WaCloudError::from_http(401, body).to_channel_error(body.into());
        assert_eq!(e.kind, ChannelErrorKind::Unauthorized);
    }

    #[test]
    fn wa_cloud_classifies_opt_in_required_as_forbidden() {
        let body = r#"{"error":{"code":131047,"message":"Re-engagement message"}}"#;
        let e = WaCloudError::from_http(400, body).to_channel_error(body.into());
        assert_eq!(e.kind, ChannelErrorKind::Forbidden);
    }

    #[test]
    fn wa_cloud_classifies_rate_limit_as_transient() {
        for code in [130429u32, 131056, 131048] {
            let body = format!(r#"{{"error":{{"code":{code},"message":"rate"}}}}"#);
            let e = WaCloudError::from_http(429, &body).to_channel_error(body.clone());
            assert_eq!(e.kind, ChannelErrorKind::Transient, "code {code}");
        }
    }

    #[test]
    fn wa_cloud_classifies_template_problem_as_payload_invalid() {
        for code in [132000u32, 132012, 132068] {
            let body = format!(r#"{{"error":{{"code":{code},"message":"template"}}}}"#);
            let e = WaCloudError::from_http(400, &body).to_channel_error(body.clone());
            assert_eq!(e.kind, ChannelErrorKind::PayloadInvalid, "code {code}");
        }
    }

    #[test]
    fn wa_cloud_falls_back_to_http_status_when_body_unparseable() {
        let e = WaCloudError::from_http(500, "<html>Bad Gateway</html>")
            .to_channel_error("transient".into());
        assert_eq!(e.kind, ChannelErrorKind::Transient);

        let e = WaCloudError::from_http(401, "no body")
            .to_channel_error("auth".into());
        assert_eq!(e.kind, ChannelErrorKind::Unauthorized);
    }

    #[test]
    fn wa_cloud_unknown_code_defaults_to_transient() {
        let body = r#"{"error":{"code":999999,"message":"never seen this"}}"#;
        let e = WaCloudError::from_http(400, body).to_channel_error(body.into());
        assert_eq!(e.kind, ChannelErrorKind::Transient);
    }

    #[test]
    fn truncate_chars_uses_ellipsis() {
        assert_eq!(truncate_chars("short", 10), "short");
        let out = truncate_chars(&"abc".repeat(50), 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.ends_with('…'));
    }
}
