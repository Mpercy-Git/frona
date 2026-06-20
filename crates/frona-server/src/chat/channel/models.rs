use std::collections::BTreeMap;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::Request;
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;
use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;
use crate::space::models::Space;
use crate::tool::mcp::models::CredentialBinding;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelManifest {
    pub id: String,
    pub display_name: String,
    /// Markdown.
    pub description: String,
    #[serde(default)]
    pub config_fields: Vec<ChannelConfigField>,
    #[serde(default)]
    pub webhook_url_visible: bool,
    /// Markdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_links: Vec<ExternalLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfigField {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub is_required: bool,
    #[serde(default)]
    pub is_secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_from: Option<ConfigRef>,
    /// `None` for secrets — values must never leave the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_resolved: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigRef {
    pub section: String,
    pub field: String,
}

#[async_trait]
pub trait ChannelFactory: Send + Sync {
    fn manifest(&self) -> ChannelManifest;
    fn create(&self, config: serde_json::Value) -> Result<Box<dyn ChannelAdapter>, AppError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity, PartialEq)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "channel")]
pub struct Channel {
    pub id: String,
    pub user_id: String,
    /// Per-user-unique. Appears in Cedar UIDs (`Channel::"{user_handle}/{handle}"`)
    /// and the FS layout (`{user_root}/channels/{handle}/`).
    pub handle: crate::core::Handle,
    pub space_id: String,
    pub provider: String,
    pub agent_id: String,
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    #[serde(default)]
    pub dispatch_mode: DispatchMode,
    pub status: ChannelStatus,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub last_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub user_address: Option<UserAddress>,
    #[serde(default)]
    pub setup: Option<SetupConfig>,
    #[serde(default)]
    pub retry: Option<crate::core::config::RetryConfig>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_deserializing, default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", rename_all = "snake_case")]
pub enum ChannelStatus {
    Disconnected,
    Connecting,
    Connected,
    Failed,
    Setup,
    Pairing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct UserAddress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_initiated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_at: Option<DateTime<Utc>>,
}

/// Provider setup (e.g. QR/code) — distinct from `UserAddress.pairing_*`
/// which authenticates the sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct SetupConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Stamped by the manager in `begin_setup`; adapters leave as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChannelRequest {
    pub space_id: String,
    pub provider: String,
    pub agent_id: String,
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    #[serde(default)]
    pub credentials: Vec<CredentialBinding>,
    #[serde(default)]
    pub dispatch_mode: DispatchMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<crate::core::config::RetryConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateChannelRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Vec<CredentialBinding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_mode: Option<DispatchMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Option<crate::core::config::RetryConfig>>,
}

#[derive(Debug, Clone)]
pub struct ExternalMessage {
    pub external_chat_id: String,
    pub sender_address: String,
    /// `None` → synthesize for Cedar only; no real Contact.
    pub sender_external_id: Option<String>,
    pub sender_display_name: Option<String>,
    pub content: String,
    pub attachments: Vec<crate::storage::Attachment>,
}

#[derive(Clone)]
pub struct ChannelCtx {
    pub space: Space,
    pub channel: Channel,
    pub emit: tokio::sync::mpsc::Sender<ExternalMessage>,
    pub channel_manager: std::sync::Arc<super::ChannelManager>,
    pub webhook_url: String,
    pub storage_service: crate::storage::StorageService,
    pub user_service: crate::auth::UserService,
    pub chat_service: crate::chat::service::ChatService,
    pub data_dir: std::path::PathBuf,
    /// External-or-local base URL used to build canonical `/api/files/...`
    /// URLs for button channels (TG/Discord/Slack/WA Cloud). Inline channels
    /// (SMS/Signal/WA User) get full `/s/{id}` URLs from `share_service`
    /// directly and do not touch this field.
    pub base_url: String,
    /// Issues `/s/{id}` short links for inline channels (SMS/Signal/WA User).
    pub share_service: crate::credential::share::service::ShareService,
    /// TTL in seconds applied to `share_service.issue_file` calls.
    pub share_ttl_secs: u64,
    /// Adapters with long-running tasks MUST observe this — sole `stop_channel` signal.
    pub cancel: tokio_util::sync::CancellationToken,
}

#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    async fn on_connect(&self, ctx: &ChannelCtx) -> Result<(), AppError>;

    async fn on_disconnect(&self, ctx: &ChannelCtx) -> Result<(), AppError>;

    /// Adapters overriding this MUST check persisted state — returning `Some`
    /// for an already-paired channel causes duplicate sessions.
    async fn on_setup_begin(
        &self,
        _ctx: &ChannelCtx,
    ) -> Result<Option<SetupConfig>, super::ChannelError> {
        Ok(None)
    }

    async fn on_setup_complete(
        &self,
        _ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError> {
        Ok(())
    }

    async fn on_tool(
        &self,
        _tool_call: &crate::inference::tool_call::ToolCall,
        _msg: &Message,
        _chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError> {
        Ok(())
    }

    async fn on_send(
        &self,
        msg: &Message,
        tool_calls: &[crate::inference::tool_call::ToolCall],
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError>;

    async fn on_webhook(
        &self,
        _ctx: &ChannelCtx,
        _request: Request<Bytes>,
    ) -> Result<Response, super::ChannelError> {
        Err(super::ChannelError::terminal(
            format!(
                "channel provider {:?} does not accept inbound webhooks",
                _ctx.channel.provider,
            ),
            super::ChannelErrorKind::PayloadInvalid,
        ))
    }

    /// Called ONCE at the start of an inference turn (initial submit, or
    /// after a HITL resume). Adapters that show a "thinking/typing" affordance
    /// should kick it off here. Default: no-op.
    async fn on_inference_start(
        &self,
        _chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError> {
        Ok(())
    }

    /// Called once when the inference turn ends (Done / Cancelled / Failed /
    /// Paused). Adapters that started a typing affordance in
    /// `on_inference_start` should tear it down here.
    async fn on_inference_done(
        &self,
        _chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError> {
        Ok(())
    }

    /// Per-token streaming text. Default: no-op. Channels that want live
    /// streaming (Telegram bots that edit messages, etc.) override this.
    async fn on_text(
        &self,
        _chat: &Chat,
        _text: &str,
        _ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError> {
        Ok(())
    }

    /// Per-token streaming reasoning (model thinking). Default: no-op.
    /// Channels can override to surface chain-of-thought.
    async fn on_reasoning(
        &self,
        _chat: &Chat,
        _text: &str,
        _ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError> {
        Ok(())
    }

    /// Model emitted a tool call mid-turn. Default: no-op.
    async fn on_tool_call(
        &self,
        _chat: &Chat,
        _name: &str,
        _arguments: &serde_json::Value,
        _ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError> {
        Ok(())
    }

    /// Tool finished executing mid-turn. Default: no-op.
    async fn on_tool_result(
        &self,
        _chat: &Chat,
        _name: &str,
        _success: bool,
        _result_summary: &str,
        _ctx: &ChannelCtx,
    ) -> Result<(), super::ChannelError> {
        Ok(())
    }

    /// Render a contiguous prefix of pending HITL prompts starting from the
    /// delivery cursor. The returned Vec corresponds to `batch[0..returned.len()]`
    /// in order; the delivery cursor advances by exactly `returned.len()`.
    ///
    /// On partway failure, return `Ok(deliveries_so_far)` — the remaining
    /// entries stay Pending and the retry poller re-attempts them on the next
    /// pass.
    ///
    /// Contract: every batch entry has `hitl.is_some()` with `status == Pending`.
    ///
    /// Default impl returns an empty Vec, which signals "I cannot render
    /// HITLs". The delivery cursor parks and the user must resolve via the
    /// web URL on each Hitl. Override for adapters that can render natively
    /// (Telegram inline buttons, SMS prompt + URL fallback, etc.).
    async fn on_pending_hitl(
        &self,
        _batch: &[crate::inference::tool_call::ToolCall],
        _msg: &Message,
        _chat: &Chat,
        _ctx: &ChannelCtx,
    ) -> Result<Vec<crate::inference::hitl::HitlDelivery>, super::ChannelError> {
        Ok(Vec::new())
    }
}

pub fn external_chat_id(chat: &Chat) -> Result<&str, AppError> {
    chat.channel_external_id.as_deref().ok_or_else(|| {
        AppError::Validation(
            "missing channel_external_id on Chat — cannot deliver outbound".into(),
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", rename_all = "snake_case")]
pub enum DispatchMode {
    #[default]
    Message,
    Signal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatType {
    Direct,
    Group,
}

impl ChatType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Group => "group",
        }
    }

    pub fn from_external_id(external_id: &str) -> Self {
        if external_id.starts_with("dm:") {
            Self::Direct
        } else {
            Self::Group
        }
    }

    pub fn from_chat(chat: &crate::chat::models::Chat) -> Self {
        chat.channel_external_id
            .as_deref()
            .map(Self::from_external_id)
            .unwrap_or(Self::Group)
    }
}

impl AsRef<str> for ChatType {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for ChatType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_struct_constructs() {
        let m = ChannelManifest {
            id: "telegram".into(),
            display_name: "Telegram".into(),
            description: "x".into(),
            config_fields: vec![ChannelConfigField {
                name: "bot_token".into(),
                description: Some("token from BotFather".into()),
                is_required: true,
                is_secret: true,
                format: Some("password".into()),
                default_from: None,
                default_resolved: None,
            }],
            webhook_url_visible: false,
            setup_instructions: None,
            external_links: vec![],
        };
        assert_eq!(m.id, "telegram");
        assert_eq!(m.config_fields.len(), 1);
        assert!(m.config_fields[0].is_secret);
    }

    #[test]
    fn channel_adapter_is_object_safe() {
        fn _accepts_dyn(_: &dyn ChannelAdapter) {}
    }

    #[test]
    fn channel_status_serializes_snake_case() {
        let v = serde_json::to_string(&ChannelStatus::Disconnected).unwrap();
        assert_eq!(v, "\"disconnected\"");
        let v = serde_json::to_string(&ChannelStatus::Connecting).unwrap();
        assert_eq!(v, "\"connecting\"");
        let v = serde_json::to_string(&ChannelStatus::Connected).unwrap();
        assert_eq!(v, "\"connected\"");
        let v = serde_json::to_string(&ChannelStatus::Failed).unwrap();
        assert_eq!(v, "\"failed\"");
        let v = serde_json::to_string(&ChannelStatus::Setup).unwrap();
        assert_eq!(v, "\"setup\"");
        let v = serde_json::to_string(&ChannelStatus::Pairing).unwrap();
        assert_eq!(v, "\"pairing\"");
    }


    #[test]
    fn dispatch_mode_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&DispatchMode::Message).unwrap(), "\"message\"");
        assert_eq!(serde_json::to_string(&DispatchMode::Signal).unwrap(), "\"signal\"");
        assert_eq!(DispatchMode::default(), DispatchMode::Message);
    }

    #[test]
    fn chat_type_from_external_id() {
        assert_eq!(ChatType::from_external_id("dm:12345"), ChatType::Direct);
        assert_eq!(ChatType::from_external_id("group:67890"), ChatType::Group);
        assert_eq!(
            ChatType::from_external_id("group:67890:topic:1"),
            ChatType::Group,
        );
        assert_eq!(ChatType::from_external_id("anything-else"), ChatType::Group);
        assert_eq!(ChatType::from_external_id(""), ChatType::Group);
    }

    #[test]
    fn chat_type_strings() {
        assert_eq!(ChatType::Direct.as_str(), "direct");
        assert_eq!(ChatType::Group.as_str(), "group");
    }
}
