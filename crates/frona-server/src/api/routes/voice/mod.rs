mod models;
mod websocket;
pub mod allowlist;
pub mod inbound;

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Router};

use crate::auth::models::Claims;
use crate::auth::token::models::TokenType;
use crate::auth::token::service::CreateTokenRequest;
use crate::auth::User;
use crate::core::Principal;
use crate::core::config::VoiceConfig;
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::tool::voice::{VoiceCallbackExtensions, VoiceSessionExtensions};

use models::TokenQuery;

/// Per-call knobs for [`build_twiml`] that vary by call site, as opposed to
/// [`VoiceConfig`]'s server-wide defaults.
#[derive(Default)]
pub(super) struct TwimlOptions<'a> {
    pub welcome_greeting: Option<&'a str>,
    pub hints: Option<&'a str>,
    /// `<Connect action="...">` — when set, Twilio re-fetches TwiML from this
    /// URL once the relay session ends instead of just hanging up. This is
    /// what makes a live transfer possible: the websocket handler ends the
    /// session normally (same as a plain hangup) and this URL decides what
    /// happens next based on whether a transfer was requested.
    pub action: Option<&'a str>,
    /// Overrides `voice.twilio_voice_id` for this leg — the answering or
    /// transferred-to agent's own `voice_id`, when it has one.
    pub voice_id: Option<&'a str>,
}

/// Build the `<ConversationRelay>` TwiML that connects the call to our
/// WebSocket.
pub(super) fn build_twiml(ws_url: &str, opts: TwimlOptions, voice: &VoiceConfig) -> String {
    use xml::writer::{EmitterConfig, XmlEvent};

    let mut buf = Vec::new();
    let mut w = EmitterConfig::new()
        .perform_indent(false)
        .write_document_declaration(true)
        .create_writer(&mut buf);

    // How eagerly the relay treats caller speech as a barge-in. Tuning this is
    // the main lever against either talking over the caller (too low) or
    // yielding to background noise on a bad line (too high).
    let interrupt_sensitivity = voice
        .twilio_interrupt_sensitivity
        .as_deref()
        .unwrap_or("medium");

    let mut relay = XmlEvent::start_element("ConversationRelay")
        .attr("url", ws_url)
        .attr("language", "en-US")
        .attr("interruptible", "any")
        .attr("interruptSensitivity", interrupt_sensitivity)
        .attr("welcomeGreetingInterruptible", "any");

    if let Some(g) = opts.welcome_greeting {
        relay = relay.attr("welcomeGreeting", g);
    }
    let effective_voice = opts.voice_id.or(voice.twilio_voice_id.as_deref());
    if let Some(v) = effective_voice {
        relay = relay.attr("voice", v);
    }
    if let Some(m) = voice.twilio_speech_model.as_deref() {
        relay = relay.attr("speechModel", m);
    }
    if let Some(tp) = voice.twilio_tts_provider.as_deref() {
        relay = relay.attr("ttsProvider", tp);
    }
    if let Some(h) = opts.hints {
        relay = relay.attr("hints", h);
    }

    let mut connect = XmlEvent::start_element("Connect");
    if let Some(a) = opts.action {
        connect = connect.attr("action", a);
    }

    w.write(XmlEvent::start_element("Response")).unwrap();
    w.write(connect).unwrap();
    w.write(relay).unwrap();
    w.write(XmlEvent::end_element()).unwrap(); // ConversationRelay
    w.write(XmlEvent::end_element()).unwrap(); // Connect
    w.write(XmlEvent::end_element()).unwrap(); // Response

    String::from_utf8(buf).expect("xml-rs always emits valid UTF-8")
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/voice/twilio/callback", post(twilio_callback))
        .route("/api/voice/twilio/ws", get(websocket::twilio_ws_handler))
        .route(
            "/api/voice/twilio/inbound",
            post(inbound::twilio_inbound_handler),
        )
        .route(
            "/api/voice/twilio/connect-action",
            post(connect_action),
        )
        .merge(allowlist::router())
}

/// Verify the voice token through the standard `TokenService` — voice tokens
/// are plain access tokens, so they're DB-backed and respect revocation.
pub(super) async fn verify_voice_jwt(state: &AppState, token: &str) -> Result<Claims, AppError> {
    state
        .token_service
        .validate(&state.keypair_service, token)
        .await
}

async fn twilio_callback(
    State(state): State<AppState>,
    Query(q): Query<TokenQuery>,
) -> Response {
    let claims = match verify_voice_jwt(&state, &q.token).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Voice callback JWT verification failed");
            return (StatusCode::FORBIDDEN, "Invalid token").into_response();
        }
    };

    let ext: VoiceCallbackExtensions = match claims
        .extensions
        .clone()
        .ok_or_else(|| AppError::Validation("voice callback token missing extensions".into()))
        .and_then(|v| {
            serde_json::from_value(v)
                .map_err(|e| AppError::Validation(format!("voice callback extensions: {e}")))
        }) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "Voice callback token extensions invalid");
            return (StatusCode::BAD_REQUEST, "Invalid voice token payload").into_response();
        }
    };

    let user_id = claims.sub.clone();
    let chat_id = ext.chat_id.clone();
    let agent_id = claims.principal.id.clone();

    let call_id = match state.call_service.find_by_chat_id(&chat_id).await {
        Ok(Some(call)) => {
            match state.call_service.mark_active(&call.id).await {
                Ok(updated) => {
                    tracing::info!(call_id = %updated.id, chat_id = %chat_id, "Call marked Active");
                    Some(updated.id)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to mark call active");
                    Some(call.id)
                }
            }
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to look up call by chat_id");
            None
        }
    };

    // The WS handler gates silence-filling on whether the remote party is one
    // of our users. On an outbound call the remote party is the contact we're
    // dialling, so surface its phone number (caller_name stays None — that
    // field drives the inbound-greeting logic and must not fire for outbound).
    let remote_phone = match ext.contact_id.as_deref() {
        Some(cid) => state
            .contact_service
            .get(&user_id, cid)
            .await
            .ok()
            .and_then(|c| c.phone),
        None => None,
    };

    let ws_ext = match serde_json::to_value(VoiceSessionExtensions {
        chat_id: chat_id.clone(),
        contact_id: ext.contact_id.clone(),
        call_id: call_id.clone(),
        direction: None,
        caller_phone: remote_phone,
        caller_name: None,
        transfer_note: None,
    }) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "Failed to encode voice session extensions");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };

    let user = User {
        id: user_id.clone(),
        handle: claims.handle.clone(),
        groups: Vec::new(),
        deactivated_at: None,
        email: claims.email.clone(),
        name: String::new(),
        password_hash: String::new(),
        timezone: None,
        phone: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let created = match state
        .token_service
        .create_token(
            &state.keypair_service,
            &user,
            CreateTokenRequest {
                token_type: TokenType::Access,
                principal: Principal::agent(&agent_id),
                ttl_secs: state.config.auth.presign_expiry_secs,
                name: "voice_session".into(),
                scopes: Vec::new(),
                refresh_pair_id: None,
                extensions: Some(ws_ext),
            },
        )
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to sign voice session JWT");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };

    let base_url = state.config.voice.callback_base_url.clone()
        .or_else(|| state.config.server.base_url.clone())
        .unwrap_or_else(|| format!("http://localhost:{}", state.config.server.port));
    let ws_base = base_url
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    let ws_url = format!("{ws_base}/api/voice/twilio/ws?token={}", created.jwt);

    let twiml = build_twiml(
        &ws_url,
        TwimlOptions {
            welcome_greeting: ext.welcome_greeting.as_deref(),
            hints: ext.hints.as_deref(),
            ..Default::default()
        },
        &state.config.voice,
    );

    tracing::info!(chat_id = %chat_id, user_id = %user_id, ws_url = %ws_url, "Voice callback: issuing TwiML with ConversationRelay");

    let mut response = twiml.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

/// A plain `<Response></Response>` — TwiML for "nothing more to do", which
/// Twilio takes as hanging up. Returned by `connect_action` whenever there's
/// no transfer to act on, so a caller-initiated hangup or `hangup_call`
/// terminate the call exactly as they did before this endpoint existed.
fn twiml_empty_response() -> Response {
    let mut response = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Response></Response>".into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

/// `POST /api/voice/twilio/connect-action`
///
/// The `action` callback on `<Connect>` — Twilio hits this once a
/// `<ConversationRelay>` session it's paired with ends, whatever the reason
/// (agent-initiated hangup, agent-initiated transfer, or the caller just
/// hanging up). The token carries the same session context the WS handler
/// used; what decides what happens next is `HandoffData`, which the just-
/// ended session's `end` message may have carried:
///
/// - Absent (plain hangup, either party) → end the call.
/// - `{"target_agent_id": "...", "note": "..."}` (see `TransferCallTool`) →
///   reassign the chat to that agent, mark the hand-off in the transcript,
///   and issue fresh TwiML that reconnects with that agent's own voice.
async fn connect_action(
    State(state): State<AppState>,
    Query(q): Query<TokenQuery>,
    Form(params): Form<HashMap<String, String>>,
) -> Response {
    let claims = match verify_voice_jwt(&state, &q.token).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Voice connect-action JWT verification failed");
            return (StatusCode::FORBIDDEN, "Invalid token").into_response();
        }
    };

    let ext: VoiceSessionExtensions = match claims
        .extensions
        .clone()
        .ok_or_else(|| AppError::Validation("voice connect-action token missing extensions".into()))
        .and_then(|v| {
            serde_json::from_value(v)
                .map_err(|e| AppError::Validation(format!("voice connect-action extensions: {e}")))
        }) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "Voice connect-action token extensions invalid");
            return (StatusCode::BAD_REQUEST, "Invalid voice token payload").into_response();
        }
    };

    // Twilio's exact param name/casing for this hasn't been confirmed against
    // a real call yet (see the call-transfer plan) — log the raw body once so
    // that's checkable from the first real transfer, and try both casings in
    // the meantime.
    tracing::info!(params = ?params, "Voice connect-action: raw callback params");
    let handoff_data = params.get("HandoffData").or_else(|| params.get("handoffData"));

    let transfer = handoff_data
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|v| {
            let target_agent_id = v.get("target_agent_id")?.as_str()?.to_string();
            let note = v.get("note").and_then(|n| n.as_str()).unwrap_or("").to_string();
            Some((target_agent_id, note))
        });

    let Some((target_agent_id, note)) = transfer else {
        tracing::info!(chat_id = %ext.chat_id, "Voice connect-action: no transfer pending — ending call");
        return twiml_empty_response();
    };

    let owner_id = claims.sub.clone();
    let chat_id = ext.chat_id.clone();

    let target_agent = match state.agent_service.find_by_id(&target_agent_id).await {
        Ok(Some(a)) => a,
        _ => {
            tracing::warn!(target_agent_id = %target_agent_id, "Voice connect-action: target agent not found — ending call");
            return twiml_empty_response();
        }
    };

    let updated_chat = match state.chat_service.reassign_agent(&owner_id, &chat_id, &target_agent.id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, chat_id = %chat_id, "Voice connect-action: failed to reassign chat agent — ending call");
            return twiml_empty_response();
        }
    };

    let _ = state
        .chat_service
        .save_system_message(
            &owner_id,
            updated_chat.space_id.as_deref(),
            &chat_id,
            format!("Transferred to {}.", target_agent.name),
        )
        .await;

    let ws_ext = match serde_json::to_value(VoiceSessionExtensions {
        chat_id: chat_id.clone(),
        contact_id: ext.contact_id.clone(),
        call_id: ext.call_id.clone(),
        direction: ext.direction,
        caller_phone: ext.caller_phone.clone(),
        caller_name: ext.caller_name.clone(),
        transfer_note: Some(note),
    }) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "Voice connect-action: failed to encode session extensions — ending call");
            return twiml_empty_response();
        }
    };

    let token_user = User {
        id: owner_id.clone(),
        handle: claims.handle.clone(),
        groups: Vec::new(),
        deactivated_at: None,
        email: claims.email.clone(),
        name: String::new(),
        password_hash: String::new(),
        timezone: None,
        phone: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let created = match state
        .token_service
        .create_token(
            &state.keypair_service,
            &token_user,
            CreateTokenRequest {
                token_type: TokenType::Access,
                principal: Principal::agent(&target_agent.id),
                ttl_secs: state.config.auth.presign_expiry_secs,
                name: "voice_session".into(),
                scopes: Vec::new(),
                refresh_pair_id: None,
                extensions: Some(ws_ext),
            },
        )
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Voice connect-action: failed to sign session JWT — ending call");
            return twiml_empty_response();
        }
    };

    let base_url = state
        .config
        .voice
        .callback_base_url
        .clone()
        .or_else(|| state.config.server.base_url.clone())
        .unwrap_or_else(|| format!("http://localhost:{}", state.config.server.port));
    let ws_base = base_url.replace("https://", "wss://").replace("http://", "ws://");
    let ws_url = format!("{ws_base}/api/voice/twilio/ws?token={}", created.jwt);
    let action_url = format!("{base_url}/api/voice/twilio/connect-action?token={}", created.jwt);

    let twiml = build_twiml(
        &ws_url,
        TwimlOptions {
            // Twilio's own `welcomeGreeting` fires immediately on connect;
            // nothing else prompts the new agent to speak until the caller
            // does, so this is what makes the hand-off audible.
            welcome_greeting: Some(&format!("You're speaking with {} now.", target_agent.name)),
            hints: None,
            action: Some(&action_url),
            voice_id: target_agent.voice_id.as_deref(),
        },
        &state.config.voice,
    );

    tracing::info!(
        chat_id = %chat_id,
        target_agent_id = %target_agent.id,
        "Voice connect-action: transfer complete — issuing TwiML for the new leg"
    );

    let mut response = twiml.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}
