use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use std::collections::HashMap;
use twilio_async::{TwilioJson, TwilioRequest};

use crate::agent::models::Agent;
use crate::agent::prompt::PromptLoader;
use crate::agent::service::AgentService;
use crate::auth::User;
use crate::auth::UserService;
use crate::auth::token::models::TokenType;
use crate::auth::token::service::{CreateTokenRequest, TokenService};
use crate::call::models::CallDirection;
use crate::call::CallService;
use crate::contact::ContactService;
use crate::core::Principal;
use crate::core::config::VoiceConfig;
use crate::core::error::AppError;
use crate::credential::keypair::service::KeyPairService;
use crate::tool::{AgentTool, InferenceContext, ToolDefinition, ToolOutput, load_tool_definition};

// ---------------------------------------------------------------------------
// Phone number helpers
// ---------------------------------------------------------------------------

/// Normalise a phone number to a canonical E.164-ish form for comparison:
/// keep the leading `+` and strip everything that is not an ASCII digit.
/// "+1 (555) 555-1234" normalises to "+15555551234".
///
/// The `00` international dialling prefix (common in the UK and Europe) is
/// treated as equivalent to `+`, so "0044 20 7946 0958" becomes "+442079460958"
/// and will match a stored entry of "+442079460958".
pub fn normalize_phone(phone: &str) -> String {
    let trimmed = phone.trim();
    // Determine whether this is an international number and strip any prefix.
    let (has_plus, digits_only) = if let Some(rest) = trimmed.strip_prefix('+') {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("00") {
        // Common European/UK international trunk prefix — treat as '+'.
        (true, rest)
    } else {
        (false, trimmed)
    };

    let mut out = String::new();
    if has_plus {
        out.push('+');
    }
    for c in digits_only.chars() {
        if c.is_ascii_digit() {
            out.push(c);
        }
    }
    out
}

/// Find the active (non-deactivated) user whose phone number matches `phone`,
/// compared in the canonical form from [`normalize_phone`] so that formatting
/// differences (spaces, `+`/`00` prefixes, punctuation) don't cause a miss.
///
/// Returns `None` when there is no match, when the caller has no usable number,
/// or when the lookup fails — callers treat a `None` as "not one of our users".
pub async fn find_user_by_phone(user_service: &UserService, phone: &str) -> Option<User> {
    let target = normalize_phone(phone);
    if target.is_empty() {
        return None;
    }
    // Narrowed to rows that actually carry a phone; the format-insensitive
    // comparison still has to happen here rather than in SQL.
    match user_service.find_all_with_phone().await {
        Ok(users) => users
            .into_iter()
            .find(|u| u.phone.as_deref().is_some_and(|p| normalize_phone(p) == target)),
        Err(e) => {
            tracing::warn!(error = %e, "find_user_by_phone: user lookup failed");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Agent resolution
// ---------------------------------------------------------------------------

/// Whether `s` could be an agent id (ids are UUIDs — see `core::repository::new_id`).
/// Handles and display names never parse as one, so this lets callers on a
/// latency-sensitive path (like answering an inbound call) skip a
/// guaranteed-miss lookup.
pub fn looks_like_agent_id(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

/// Resolve `query` against `owner_id`'s agents by id, then handle, then
/// display name — the same chain the inbound-call agent-selection setting
/// and the `transfer_call` tool both use. `None` when none of the three
/// match.
///
/// The id branch isn't owner-scoped (unlike handle/name): an id is only
/// ever produced by the owner's own prior selection, so trusting it here
/// avoids an extra scoped lookup on the common miss.
pub async fn resolve_agent_by_query(
    agent_service: &AgentService,
    owner_id: &str,
    query: &str,
) -> Option<Agent> {
    if looks_like_agent_id(query)
        && let Ok(Some(agent)) = agent_service.find_by_id(query).await
    {
        return Some(agent);
    }
    if let Ok(Some(agent)) = agent_service.find_by_handle(owner_id, query).await {
        return Some(agent);
    }
    if let Ok(Some(agent)) = agent_service.find_by_name(owner_id, query).await {
        return Some(agent);
    }
    None
}

// ---------------------------------------------------------------------------
// Twilio webhook signature validation
// ---------------------------------------------------------------------------

type HmacSha1 = Hmac<Sha1>;

/// Validate the `X-Twilio-Signature` header on an incoming Twilio webhook.
///
/// Twilio computes HMAC-SHA1 over the full request URL concatenated with all
/// sorted POST body key/value pairs (no separators), then base64-encodes the
/// result.  Returns `true` when the computed digest matches `header_sig`.
pub fn validate_twilio_signature(
    auth_token: &str,
    url: &str,
    params: &HashMap<String, String>,
    header_sig: &str,
) -> bool {
    let mut sorted: Vec<(&str, &str)> = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    sorted.sort_by_key(|(k, _)| *k);

    let mut s = url.to_string();
    for (k, v) in sorted {
        s.push_str(k);
        s.push_str(v);
    }

    let mut mac = match HmacSha1::new_from_slice(auth_token.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(s.as_bytes());
    let result = mac.finalize().into_bytes();
    let expected = base64::engine::general_purpose::STANDARD.encode(result);
    expected == header_sig
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VoiceCallbackExtensions {
    pub chat_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub welcome_greeting: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_id: Option<String>,
    /// Set only for `transfer_call`'s callback leg — carried through to the
    /// WS session's own `VoiceSessionExtensions::transfer_note` once this
    /// outbound call is answered. See `place_outbound_call`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VoiceSessionExtensions {
    pub chat_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// Present on inbound calls; `None` for outbound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<CallDirection>,
    /// Caller's phone number for inbound calls (stored so the WS handler does
    /// not need an extra DB round-trip to build the `[INBOUND_CALL]` message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_phone: Option<String>,
    /// Caller's display name for inbound calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_name: Option<String>,
    /// Set when this leg is `transfer_call`'s callback — a fresh outbound
    /// call the target agent places to the original caller once the source
    /// call has actually ended (see `place_outbound_call` and
    /// `api::routes::voice::websocket`). Seeds the target agent's first turn
    /// with a `[CALL_TRANSFERRED: ...]` prefix, same mechanism as
    /// `[INBOUND_CALL: ...]` on a fresh inbound answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_note: Option<String>,
}

// ---------------------------------------------------------------------------
// VoiceProvider trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait VoiceProvider: Send + Sync {
    fn name(&self) -> &str;
    /// Initiate an outbound call. Returns the provider's call identifier (e.g. Twilio SID).
    #[allow(clippy::too_many_arguments)]
    async fn initiate_call(
        &self,
        to: &str,
        chat_id: &str,
        user: &User,
        agent_id: &str,
        welcome_greeting: Option<&str>,
        hints: Option<&str>,
        contact_id: Option<String>,
        transfer_note: Option<&str>,
    ) -> Result<String, AppError>;
}

// ---------------------------------------------------------------------------
// TwilioProvider
// ---------------------------------------------------------------------------

pub struct TwilioProvider {
    pub account_sid: String,
    pub auth_token: String,
    pub from_number: String,
    pub base_url: String,
    pub voice_id: Option<String>,
    pub speech_model: Option<String>,
    pub token_service: TokenService,
    pub keypair_service: KeyPairService,
    /// Callback token TTL in seconds — short enough that a leaked callback URL
    /// can't be replayed beyond the call setup window.
    pub callback_ttl_secs: u64,
}

#[async_trait]
impl VoiceProvider for TwilioProvider {
    fn name(&self) -> &str {
        "twilio"
    }

    async fn initiate_call(
        &self,
        to: &str,
        chat_id: &str,
        user: &User,
        agent_id: &str,
        welcome_greeting: Option<&str>,
        hints: Option<&str>,
        contact_id: Option<String>,
        transfer_note: Option<&str>,
    ) -> Result<String, AppError> {
        let extensions = serde_json::to_value(VoiceCallbackExtensions {
            chat_id: chat_id.to_string(),
            welcome_greeting: welcome_greeting.map(str::to_string),
            hints: hints.map(str::to_string),
            contact_id,
            transfer_note: transfer_note.map(str::to_string),
        })
        .map_err(|e| AppError::Internal(format!("voice callback claims encode: {e}")))?;

        let created = self
            .token_service
            .create_token(
                &self.keypair_service,
                user,
                CreateTokenRequest {
                    token_type: TokenType::Access,
                    principal: Principal::agent(agent_id),
                    ttl_secs: self.callback_ttl_secs,
                    name: "voice_callback".into(),
                    scopes: Vec::new(),
                    refresh_pair_id: None,
                    extensions: Some(extensions),
                },
            )
            .await?;

        let callback_url = format!(
            "{}/api/voice/twilio/callback?token={}",
            self.base_url, created.jwt
        );

        let client = twilio_async::Twilio::new(&self.account_sid, &self.auth_token)
            .map_err(|e| AppError::Tool(format!("Twilio client init failed: {e}")))?;

        let result = client
            .call(&self.from_number, to, &callback_url)
            .run()
            .await
            .map_err(|e| AppError::Tool(format!("Twilio call failed: {e}")))?;

        match result {
            TwilioJson::Success(call) => Ok(call.sid),
            TwilioJson::Fail { status, message, .. } => Err(AppError::Tool(format!(
                "Twilio API error {status}: {message}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// PlivoProvider
// ---------------------------------------------------------------------------

pub struct PlivoProvider {
    pub auth_id: String,
    pub auth_token: String,
    pub from_number: String,
    pub base_url: String,
    pub token_service: TokenService,
    pub keypair_service: KeyPairService,
    pub callback_ttl_secs: u64,
    pub http_client: reqwest::Client,
}

#[async_trait]
impl VoiceProvider for PlivoProvider {
    fn name(&self) -> &str {
        "plivo"
    }

    async fn initiate_call(
        &self,
        to: &str,
        chat_id: &str,
        user: &User,
        agent_id: &str,
        welcome_greeting: Option<&str>,
        hints: Option<&str>,
        contact_id: Option<String>,
        transfer_note: Option<&str>,
    ) -> Result<String, AppError> {
        let extensions = serde_json::to_value(VoiceCallbackExtensions {
            chat_id: chat_id.to_string(),
            welcome_greeting: welcome_greeting.map(str::to_string),
            hints: hints.map(str::to_string),
            contact_id,
            transfer_note: transfer_note.map(str::to_string),
        })
        .map_err(|e| AppError::Internal(format!("voice callback claims encode: {e}")))?;

        let created = self
            .token_service
            .create_token(
                &self.keypair_service,
                user,
                CreateTokenRequest {
                    token_type: TokenType::Access,
                    principal: Principal::agent(agent_id),
                    ttl_secs: self.callback_ttl_secs,
                    name: "voice_callback".into(),
                    scopes: Vec::new(),
                    refresh_pair_id: None,
                    extensions: Some(extensions),
                },
            )
            .await?;

        let answer_url = format!(
            "{}/api/voice/twilio/callback?token={}",
            self.base_url, created.jwt
        );

        // Plivo REST API: POST https://api.plivo.com/v1/Account/{auth_id}/Call/
        let url = format!(
            "https://api.plivo.com/v1/Account/{}/Call/",
            self.auth_id
        );

        let body = serde_json::json!({
            "from": self.from_number,
            "to": to,
            "answer_url": answer_url,
            "answer_method": "POST",
        });

        // Plivo doesn't have ConversationRelay like Twilio — we pass the
        // answer_url which returns our TwiML/XML when the call is answered.
        // The callback handler will generate the appropriate response.


        let resp = self
            .http_client
            .post(&url)
            .basic_auth(&self.auth_id, Some(&self.auth_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("Plivo API request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Tool(format!(
                "Plivo API error {status}: {text}"
            )));
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::Tool(format!("Plivo response parse error: {e}")))?;

        // Plivo returns request_uuid as the call identifier
        let call_uuid = result
            .get("request_uuid")
            .and_then(|v| v.as_str())
            .or_else(|| result.get("request_uuid").and_then(|v| v.as_array()).and_then(|a| a.first()).and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();

        if call_uuid.is_empty() {
            return Err(AppError::Tool("Plivo API returned no request_uuid".into()));
        }

        Ok(call_uuid)
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub fn create_voice_provider(
    config: &VoiceConfig,
    base_url: &str,
    token_service: TokenService,
    keypair_service: KeyPairService,
) -> Option<Arc<dyn VoiceProvider>> {
    let provider = config
        .provider
        .as_deref()
        .or_else(|| {
            if config.twilio_account_sid.is_some() {
                Some("twilio")
            } else if config.plivo_auth_id.is_some() {
                Some("plivo")
            } else {
                None
            }
        })?;

    match provider.to_lowercase().as_str() {
        "twilio" => {
            let account_sid = config.twilio_account_sid.clone()?;
            let auth_token = config.twilio_auth_token.clone()?;
            let from_number = config.twilio_from_number.clone()?;
            Some(Arc::new(TwilioProvider {
                account_sid,
                auth_token,
                from_number,
                base_url: base_url.to_string(),
                voice_id: config.twilio_voice_id.clone(),
                speech_model: config.twilio_speech_model.clone(),
                token_service,
                keypair_service,
                callback_ttl_secs: 300,
            }))
        }
        "plivo" => {
            let auth_id = config.plivo_auth_id.clone()?;
            let auth_token = config.plivo_auth_token.clone()?;
            let from_number = config.plivo_from_number.clone()?;
            Some(Arc::new(PlivoProvider {
                auth_id,
                auth_token,
                from_number,
                base_url: base_url.to_string(),
                token_service,
                keypair_service,
                callback_ttl_secs: 300,
                http_client: reqwest::Client::new(),
            }))
        }
        other => {
            tracing::warn!(provider = %other, "Unknown voice provider; voice calling disabled");
            None
        }
    }
}

/// Place an outbound call attached to `chat_id`, as `agent_id`: finds or
/// creates the contact, asks the provider to dial, and records the `Call`
/// row. Shared by `VoiceCallTool` (an agent-requested call) and the
/// `transfer_call` callback (`api::routes::voice::websocket`) placing a
/// fresh call to the original caller once the source call has actually
/// ended. Returns the resolved contact, so a caller that needs its name
/// (e.g. for a tool-result prompt block) doesn't have to look it up again.
#[allow(clippy::too_many_arguments)]
pub async fn place_outbound_call(
    provider: &dyn VoiceProvider,
    contact_service: &ContactService,
    call_service: &CallService,
    chat_id: &str,
    user: &User,
    agent_id: &str,
    phone_number: &str,
    name: &str,
    welcome_greeting: Option<&str>,
    hints: Option<&str>,
    transfer_note: Option<&str>,
) -> Result<crate::contact::models::ContactResponse, AppError> {
    let contact = contact_service
        .find_or_create_by_phone(&user.id, phone_number, name)
        .await?;

    let sid = provider
        .initiate_call(
            phone_number,
            chat_id,
            user,
            agent_id,
            welcome_greeting,
            hints,
            Some(contact.id.clone()),
            transfer_note,
        )
        .await?;
    tracing::info!(sid = %sid, to = %phone_number, chat_id = %chat_id, "Voice call initiated");

    call_service
        .create(chat_id, &contact.id, &sid, CallDirection::Outbound)
        .await?;

    Ok(contact)
}

// ---------------------------------------------------------------------------
// VoiceCallTool (external — pauses loop until Twilio callback)
// ---------------------------------------------------------------------------

pub struct VoiceCallTool {
    pub provider: Option<Arc<dyn VoiceProvider>>,
    pub prompts: PromptLoader,
    pub contact_service: ContactService,
    pub call_service: CallService,
}

#[async_trait]
impl AgentTool for VoiceCallTool {
    fn name(&self) -> &str {
        "make_voice_call"
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        load_tool_definition(&self.prompts, "tools/voice_call.md")
            .map(|d| vec![d])
            .unwrap_or_default()
    }

    async fn execute(&self, _tool_name: &str, arguments: Value, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let phone_number = arguments
            .get("phone_number")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: phone_number".into()))?;

        let name = arguments
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: name".into()))?;

        let objective = arguments
            .get("objective")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: objective".into()))?;

        let initial_greeting = arguments.get("initial_greeting").and_then(|v| v.as_str());
        let hints = arguments.get("hints").and_then(|v| v.as_str());

        let provider = self.provider.as_ref().ok_or_else(|| {
            AppError::Tool("Voice calling is not configured. Set voice.twilio_account_sid, twilio_auth_token, and twilio_from_number in config.".into())
        })?;

        let chat_id = &ctx.chat.id;

        let contact = place_outbound_call(
            provider.as_ref(),
            &self.contact_service,
            &self.call_service,
            chat_id,
            &ctx.user,
            &ctx.agent.id,
            phone_number,
            name,
            initial_greeting,
            hints,
            None,
        )
        .await?;

        let call_connected_block = self.prompts
            .read_with_vars("active_call.md", &[
                ("caller_name", &contact.name),
                ("phone_number", phone_number),
                ("objective", objective),
            ])
            .unwrap_or_default();

        Ok(ToolOutput::text(call_connected_block).as_pending_external())
    }
}

// ---------------------------------------------------------------------------
// SendDtmfTool (external — pauses tool loop)
// ---------------------------------------------------------------------------

pub struct SendDtmfTool {
    pub prompts: PromptLoader,
}

#[async_trait]
impl AgentTool for SendDtmfTool {
    fn name(&self) -> &str {
        "send_dtmf"
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        load_tool_definition(&self.prompts, "tools/send_dtmf.md")
            .map(|d| vec![d])
            .unwrap_or_default()
    }

    async fn execute(&self, _tool_name: &str, arguments: Value, _ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let digits = arguments
            .get("digits")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: digits".into()))?;
        // The result IS the digits string — the voice handler reads external_tool.result
        Ok(ToolOutput::text(digits).as_pending_external())
    }
}

// ---------------------------------------------------------------------------
// HangupCallTool (external — pauses tool loop)
// ---------------------------------------------------------------------------

pub struct HangupCallTool {
    pub prompts: PromptLoader,
}

#[async_trait]
impl AgentTool for HangupCallTool {
    fn name(&self) -> &str {
        "hangup_call"
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        load_tool_definition(&self.prompts, "tools/hangup_call.md")
            .map(|d| vec![d])
            .unwrap_or_default()
    }

    async fn execute(&self, _tool_name: &str, _arguments: Value, _ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        Ok(ToolOutput::text("hangup").as_pending_external())
    }
}

// ---------------------------------------------------------------------------
// TransferCallTool (external — pauses tool loop)
// ---------------------------------------------------------------------------

/// Past this many transfers on one call, further `transfer_call` attempts are
/// rejected rather than silently retried — a loop guard against agents
/// bouncing a caller back and forth.
const MAX_CALL_TRANSFERS: u32 = 5;

pub struct TransferCallTool {
    pub prompts: PromptLoader,
    pub agent_service: AgentService,
    pub call_service: CallService,
    pub chat_service: crate::chat::service::ChatService,
}

#[async_trait]
impl AgentTool for TransferCallTool {
    fn name(&self) -> &str {
        "transfer_call"
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        load_tool_definition(&self.prompts, "tools/transfer_call.md")
            .map(|d| vec![d])
            .unwrap_or_default()
    }

    async fn execute(&self, _tool_name: &str, arguments: Value, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let target_query = arguments
            .get("target_agent")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: target_agent".into()))?;
        let note = arguments
            .get("handoff_note")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let owner_id = &ctx.user.id;
        let target = resolve_agent_by_query(&self.agent_service, owner_id, target_query)
            .await
            .ok_or_else(|| AppError::Validation(format!("Agent '{target_query}' not found")))?;

        if target.id == ctx.agent.id {
            return Err(AppError::Validation(
                "The caller is already speaking with that agent".into(),
            ));
        }
        if !target.enabled {
            return Err(AppError::Validation(format!("Agent '{}' is disabled", target.name)));
        }
        // The id branch of resolve_agent_by_query isn't owner-scoped (see its
        // doc comment) — this is what actually enforces that the target is
        // one the caller's agent is allowed to hand off to.
        self.agent_service.get_accessible(owner_id, &target.id).await?;

        let call = self
            .call_service
            .find_by_chat_id(&ctx.chat.id)
            .await?
            .ok_or_else(|| AppError::Validation("No active call on this chat".into()))?;
        if call.transfer_count >= MAX_CALL_TRANSFERS {
            return Err(AppError::Validation(
                "This call has already been transferred too many times".into(),
            ));
        }
        self.call_service.increment_transfer_count(&call.id).await?;

        // Reassign now, not once the callback connects: the transcript
        // marker and the chat's owning agent should reflect the transfer
        // immediately, independent of whether the callback ever succeeds.
        let updated_chat = self
            .chat_service
            .reassign_agent(owner_id, &ctx.chat.id, &target.id)
            .await?;
        let _ = self
            .chat_service
            .save_system_message(
                owner_id,
                updated_chat.space_id.as_deref(),
                &ctx.chat.id,
                format!("Transferred to {}.", target.name),
            )
            .await;

        // Read by the voice websocket handler: it ends the current call
        // exactly like hangup_call, then places a fresh outbound call to the
        // same caller as the target agent — see
        // api::routes::voice::websocket::place_transfer_callback. No DB
        // round-trip or Twilio-relayed data needed for that hand-off; it's
        // all in-process from here.
        let payload = serde_json::json!({
            "target_agent_id": target.id,
            "note": note,
        });
        Ok(ToolOutput::text(payload.to_string()).as_pending_external())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::generic::SurrealRepo;
    use crate::core::config::VoiceConfig;

    async fn test_contact_service() -> ContactService {
        use surrealdb::Surreal;
        use surrealdb::engine::local::Mem;
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        ContactService::new(SurrealRepo::new(db), crate::chat::broadcast::BroadcastService::new())
    }

    #[test]
    fn create_voice_provider_none_with_empty_config() {
        let config = VoiceConfig::default();
        assert!(config.twilio_account_sid.is_none());
        assert!(config.provider.is_none());
    }

    #[test]
    fn looks_like_agent_id_only_matches_uuids() {
        // Real ids are UUIDs; handles/names must not trigger the id lookup.
        assert!(looks_like_agent_id(&uuid::Uuid::new_v4().to_string()));
        assert!(!looks_like_agent_id("receptionist"));
        assert!(!looks_like_agent_id("my-agent_2"));
        assert!(!looks_like_agent_id(""));
    }

    #[test]
    fn send_dtmf_tool_name() {
        use crate::agent::prompt::PromptLoader;
        use std::path::PathBuf;
        let prompts = PromptLoader::new(PathBuf::from("/tmp/nonexistent"));
        let tool = SendDtmfTool { prompts };
        assert_eq!(tool.name(), "send_dtmf");
    }

    #[test]
    fn hangup_call_tool_name() {
        use crate::agent::prompt::PromptLoader;
        use std::path::PathBuf;
        let prompts = PromptLoader::new(PathBuf::from("/tmp/nonexistent"));
        let tool = HangupCallTool { prompts };
        assert_eq!(tool.name(), "hangup_call");
    }

    async fn test_call_service() -> crate::call::CallService {
        use surrealdb::Surreal;
        use surrealdb::engine::local::Mem;
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        crate::call::CallService::new(SurrealRepo::new(db))
    }

    #[tokio::test]
    async fn voice_call_tool_name() {
        use crate::agent::prompt::PromptLoader;
        use std::path::PathBuf;
        let prompts = PromptLoader::new(PathBuf::from("/tmp/nonexistent"));
        let tool = VoiceCallTool {
            provider: None,
            prompts,
            contact_service: test_contact_service().await,
            call_service: test_call_service().await,
        };
        assert_eq!(tool.name(), "make_voice_call");
    }
}
