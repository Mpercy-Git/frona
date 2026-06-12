use std::collections::HashMap;

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Form;

use crate::auth::User;
use crate::auth::token::models::TokenType;
use crate::auth::token::service::CreateTokenRequest;
use crate::call::models::CallDirection;
use crate::chat::models::CreateChatRequest;
use crate::core::Principal;
use crate::core::state::AppState;
use crate::tool::voice::{VoiceSessionExtensions, validate_twilio_signature};

use super::build_twiml;

// ---------------------------------------------------------------------------
// TwiML helpers
// ---------------------------------------------------------------------------

/// Build a TwiML `<Reject/>` response, optionally with `reason="busy"`.
fn twiml_reject(reason: Option<&str>) -> Response {
    use xml::writer::{EmitterConfig, XmlEvent};

    let mut buf = Vec::new();
    let mut w = EmitterConfig::new()
        .perform_indent(false)
        .write_document_declaration(true)
        .create_writer(&mut buf);

    let mut reject = XmlEvent::start_element("Reject");
    if let Some(r) = reason {
        reject = reject.attr("reason", r);
    }

    w.write(XmlEvent::start_element("Response")).unwrap();
    w.write(reject).unwrap();
    w.write(XmlEvent::end_element()).unwrap(); // Reject
    w.write(XmlEvent::end_element()).unwrap(); // Response

    let twiml = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(_) => return twiml_reject(None),
    };
    let mut response = twiml.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

// ---------------------------------------------------------------------------
// Inbound webhook handler
// ---------------------------------------------------------------------------

/// `POST /api/voice/twilio/inbound`
///
/// Twilio calls this when an inbound call arrives at the configured phone
/// number.  This handler:
///
/// 1. Validates the Twilio signature (when `voice.twilio_auth_token` is set).
/// 2. Rejects the call when `voice.inbound_enabled` is `false`.
/// 3. Resolves which platform user "owns" the call by checking:
///    a. Every user's per-user DB allowlist
///    b. The static `voice.inbound_allowlist` config (falls back to
///       `voice.inbound_user_id`)
///    Calls from numbers not on any allowlist receive a `<Reject reason="busy"/>`.
/// 4. Creates a contact, chat, and call record under the owning user's account.
/// 5. Issues a short-lived voice-session JWT and returns the
///    `<ConversationRelay>` TwiML that connects Twilio to the agent's
///    WebSocket endpoint.
pub(super) async fn twilio_inbound_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(params): Form<HashMap<String, String>>,
) -> Response {
    // ------------------------------------------------------------------
    // 1. Validate Twilio signature
    // ------------------------------------------------------------------
    if let Some(auth_token) = &state.config.voice.twilio_auth_token {
        let sig = headers
            .get("x-twilio-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let base_url = state
            .config
            .voice
            .callback_base_url
            .clone()
            .or_else(|| state.config.server.base_url.clone())
            .unwrap_or_else(|| format!("http://localhost:{}", state.config.server.port));
        let full_url = format!("{base_url}/api/voice/twilio/inbound");

        if !validate_twilio_signature(auth_token, &full_url, &params, sig) {
            tracing::warn!("Inbound call: invalid Twilio signature — rejecting");
            return (StatusCode::FORBIDDEN, "Invalid signature").into_response();
        }
    }

    // ------------------------------------------------------------------
    // 2. Check master inbound switch
    // ------------------------------------------------------------------
    if !state.config.voice.inbound_enabled {
        tracing::info!("Inbound call: inbound calling is disabled — rejecting");
        return twiml_reject(None);
    }

    // ------------------------------------------------------------------
    // 3. Extract call parameters from the Twilio POST body
    // ------------------------------------------------------------------
    let from = params
        .get("From")
        .map(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let call_sid = params
        .get("CallSid")
        .map(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    if from.is_empty() || call_sid.is_empty() {
        tracing::warn!("Inbound call: missing From or CallSid — rejecting");
        return (StatusCode::BAD_REQUEST, "Missing call parameters").into_response();
    }

    // ------------------------------------------------------------------
    // 4. Resolve call ownership from allowlists
    // ------------------------------------------------------------------
    let user_id = match state
        .find_user_for_caller(
            &from,
            state.config.voice.inbound_user_id.as_deref(),
            &state.config.voice.inbound_allowlist,
        )
        .await
    {
        Some(uid) => uid,
        None => {
            tracing::info!(
                from = %from,
                "Inbound call: caller not in any allowlist — rejecting"
            );
            return twiml_reject(Some("busy"));
        }
    };

    // ------------------------------------------------------------------
    // 5. Fetch the owning user record (needed to sign the JWT)
    // ------------------------------------------------------------------
    let user = match state.user_service.find_by_id(&user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!(user_id = %user_id, "Inbound call: resolved user not found — rejecting");
            return twiml_reject(None);
        }
        Err(e) => {
            tracing::error!(error = %e, user_id = %user_id, "Inbound call: failed to fetch user — rejecting");
            return twiml_reject(None);
        }
    };

    // ------------------------------------------------------------------
    // 6. Resolve answering agent
    // ------------------------------------------------------------------
    let agent_id = state
        .config
        .voice
        .inbound_agent_id
        .clone()
        .unwrap_or_else(|| "receptionist".to_string());

    if state
        .agent_service
        .find_by_id(&agent_id)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        tracing::warn!(agent_id = %agent_id, "Inbound call: configured agent not found — rejecting");
        return twiml_reject(None);
    }

    // ------------------------------------------------------------------
    // 7. Find or create the caller's contact record
    // ------------------------------------------------------------------
    let contact = match state
        .contact_service
        .find_or_create_by_phone(&user_id, &from, "Incoming caller")
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, from = %from, "Inbound call: failed to upsert contact — rejecting");
            return twiml_reject(None);
        }
    };

    // ------------------------------------------------------------------
    // 8. Create a new chat for this call
    // ------------------------------------------------------------------
    let chat = match state
        .chat_service
        .create_chat(
            &user_id,
            CreateChatRequest {
                space_id: None,
                task_id: None,
                agent_id: agent_id.clone(),
                title: Some(format!("Inbound call from {from}")),
            },
        )
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Inbound call: failed to create chat — rejecting");
            return twiml_reject(None);
        }
    };

    // ------------------------------------------------------------------
    // 9. Record the call (Ringing → Active immediately for inbound)
    // ------------------------------------------------------------------
    let call = match state
        .call_service
        .create(&chat.id, &contact.id, &call_sid, CallDirection::Inbound)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Inbound call: failed to create call record — rejecting");
            return twiml_reject(None);
        }
    };

    // Mark the call active because we're about to answer it.
    let call_id = match state.call_service.mark_active(&call.id).await {
        Ok(c) => c.id,
        Err(e) => {
            tracing::warn!(error = %e, call_id = %call.id, "Inbound call: failed to mark call active (continuing)");
            call.id.clone()
        }
    };

    // ------------------------------------------------------------------
    // 10. Issue a voice-session JWT (goes directly to the WS handler;
    //     no intermediate callback token needed for inbound calls)
    // ------------------------------------------------------------------
    let ws_ext = match serde_json::to_value(VoiceSessionExtensions {
        chat_id: chat.id.clone(),
        contact_id: Some(contact.id.clone()),
        call_id: Some(call_id.clone()),
        direction: Some(CallDirection::Inbound),
        caller_phone: Some(from.clone()),
        caller_name: Some(contact.name.clone()),
    }) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "Inbound call: failed to encode session extensions — rejecting");
            return twiml_reject(None);
        }
    };

    // Build a minimal User value — the token service only needs id/username/email.
    let token_user = User {
        id: user.id.clone(),
        username: user.username.clone(),
        email: user.email.clone(),
        name: user.name.clone(),
        password_hash: String::new(),
        timezone: None,
        created_at: user.created_at,
        updated_at: user.updated_at,
    };

    let created = match state
        .token_service
        .create_token(
            &state.keypair_service,
            &token_user,
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
            tracing::error!(error = %e, "Inbound call: failed to sign session JWT — rejecting");
            return twiml_reject(None);
        }
    };

    // ------------------------------------------------------------------
    // 11. Build the ConversationRelay TwiML
    // ------------------------------------------------------------------
    let base_url = state
        .config
        .voice
        .callback_base_url
        .clone()
        .or_else(|| state.config.server.base_url.clone())
        .unwrap_or_else(|| format!("http://localhost:{}", state.config.server.port));
    let ws_base = base_url
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    let ws_url = format!("{ws_base}/api/voice/twilio/ws?token={}", created.jwt);

    let twiml = build_twiml(
        &ws_url,
        state.config.voice.inbound_welcome_greeting.as_deref(),
        None, // hints — not applicable for inbound
        state.config.voice.twilio_voice_id.as_deref(),
        state.config.voice.twilio_speech_model.as_deref(),
    );

    tracing::info!(
        from = %from,
        chat_id = %chat.id,
        call_id = %call_id,
        user_id = %user_id,
        agent_id = %agent_id,
        "Inbound call answered — TwiML issued with ConversationRelay"
    );

    let mut response = twiml.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use crate::tool::voice::validate_twilio_signature;
    use std::collections::HashMap;

    #[test]
    fn signature_valid_known_vector() {
        // Construct a known HMAC-SHA1 signature by hand.
        // url = "https://example.com/inbound", no form params.
        use hmac::{Hmac, Mac};
        use sha1::Sha1;
        type HmacSha1 = Hmac<Sha1>;

        let auth_token = "test_token";
        let url = "https://example.com/inbound";
        let params: HashMap<String, String> = HashMap::new();

        let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
        mac.update(url.as_bytes());
        let result = mac.finalize().into_bytes();
        let expected_sig = base64::engine::general_purpose::STANDARD.encode(result);

        assert!(validate_twilio_signature(auth_token, url, &params, &expected_sig));
    }

    #[test]
    fn signature_invalid_wrong_token() {
        let params: HashMap<String, String> = HashMap::new();
        assert!(!validate_twilio_signature(
            "wrong_token",
            "https://example.com/inbound",
            &params,
            "notavalidsig"
        ));
    }

    #[test]
    fn signature_includes_sorted_params() {
        use hmac::{Hmac, Mac};
        use sha1::Sha1;
        type HmacSha1 = Hmac<Sha1>;

        let auth_token = "abc123";
        let url = "https://example.com/inbound";
        let mut params: HashMap<String, String> = HashMap::new();
        params.insert("From".into(), "+15555550000".into());
        params.insert("CallSid".into(), "CA123".into());

        // Build expected: url + "CallSid" + "CA123" + "From" + "+15555550000" (sorted)
        let s = format!("{}CallSidCA123From+15555550000", url);
        let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
        mac.update(s.as_bytes());
        let result = mac.finalize().into_bytes();
        let expected_sig = base64::engine::general_purpose::STANDARD.encode(result);

        assert!(validate_twilio_signature(auth_token, url, &params, &expected_sig));
    }
}
