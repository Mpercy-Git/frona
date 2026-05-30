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

#[derive(crate::ChannelFactory)]
#[channel(id = "whatsapp_cloud", from = WhatsAppCloudConfig)]
pub struct WhatsAppCloudAdapter {
    config: WhatsAppCloudConfig,
    http: reqwest::Client,
}

impl From<WhatsAppCloudConfig> for WhatsAppCloudAdapter {
    fn from(config: WhatsAppCloudConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
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
    ) -> Result<(), AppError> {
        let to = parse_external_id(external_chat_id(chat)?)?;
        let body = if msg.content.trim().is_empty() {
            String::new()
        } else {
            super::markdown::to_whatsapp(&msg.content)
        };
        let mut text_consumed = false;

        for attachment in &msg.attachments {
            let bytes = read_attachment_bytes(ctx, attachment)?;
            let media_id = self
                .upload_media(&attachment.filename, &attachment.content_type, bytes)
                .await?;
            let caption = if !text_consumed && !body.is_empty() {
                text_consumed = true;
                Some(body.as_str())
            } else {
                None
            };
            self.send_media(&to, &attachment.content_type, &media_id, caption)
                .await?;
            tracing::info!(
                channel_id = %ctx.channel.id,
                msg_id = %msg.id,
                to = %to,
                content_type = %attachment.content_type,
                "WhatsApp Cloud media sent",
            );
        }

        if !text_consumed && !body.is_empty() {
            self.send_text(&to, &body).await?;
            tracing::info!(
                channel_id = %ctx.channel.id,
                msg_id = %msg.id,
                to = %to,
                "WhatsApp Cloud text message sent",
            );
        }
        Ok(())
    }

    async fn on_webhook(
        &self,
        ctx: &ChannelCtx,
        request: Request<Bytes>,
    ) -> Result<Response, AppError> {
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
}

impl WhatsAppCloudAdapter {
    fn handle_verify(&self, request: Request<Bytes>) -> Result<Response, AppError> {
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

    async fn send_text(&self, to: &str, body: &str) -> Result<(), AppError> {
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
    ) -> Result<(), AppError> {
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

    async fn send_message(&self, payload: serde_json::Value) -> Result<(), AppError> {
        let url = format!("{CLOUD_API_BASE}/{}/messages", self.config.phone_number_id);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.access_token)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("WA send failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!("WA send {status}: {body}")));
        }
        Ok(())
    }

    async fn emit_inbound(
        &self,
        ctx: &ChannelCtx,
        raw: serde_json::Value,
    ) -> Result<(), AppError> {
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
}

fn read_attachment_bytes(ctx: &ChannelCtx, att: &Attachment) -> Result<Vec<u8>, AppError> {
    let owner_str = att
        .owner
        .strip_prefix("user:")
        .ok_or_else(|| AppError::Validation(format!("unsupported attachment owner: {}", att.owner)))?;
    let owner_handle = crate::core::Handle::try_new(owner_str)
        .map_err(|e| AppError::Validation(format!("invalid owner handle in {}: {e}", att.owner)))?;
    let workspace = ctx.storage_service.user_workspace(&owner_handle);
    let abs = workspace
        .resolve_path(&att.path)
        .ok_or_else(|| AppError::NotFound(format!("attachment {} not in workspace", att.path)))?;
    std::fs::read(&abs).map_err(|e| AppError::Internal(format!("read attachment {}: {e}", att.path)))
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
}
