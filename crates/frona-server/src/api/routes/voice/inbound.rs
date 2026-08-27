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
use crate::tool::voice::{
    VoiceSessionExtensions, find_user_by_phone, resolve_agent_by_query, validate_twilio_signature,
};

use super::{TwimlOptions, build_twiml};

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
/// 3. Resolves which platform user "owns" the call by scanning every user's
///    per-user DB allowlist (first match wins). Calls from numbers not on any
///    user's allowlist receive a `<Reject reason="busy"/>`.
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
    // Signature validation — skip if FRONA_VOICE_SKIP_SIG_CHECK is set (for debugging)
    let skip_sig_check = std::env::var("FRONA_VOICE_SKIP_SIG_CHECK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if skip_sig_check {
        tracing::warn!("Inbound call: skipping Twilio signature validation (FRONA_VOICE_SKIP_SIG_CHECK=1)");
    } else if let Some(auth_token) = &state.config.voice.twilio_auth_token {
        let sig = headers
            .get("x-twilio-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        tracing::debug!(
            param_count = params.len(),
            params = ?params,
            has_signature = !sig.is_empty(),
            content_type = ?headers.get("content-type").and_then(|v| v.to_str().ok()),
            "Inbound call: raw request details"
        );

        let base_url = state
            .config
            .voice
            .callback_base_url
            .clone()
            .or_else(|| state.config.server.base_url.clone())
            .unwrap_or_else(|| format!("http://localhost:{}", state.config.server.port));
        let full_url = format!("{base_url}/api/voice/twilio/inbound");

        // Also try the URL as Twilio sees it (from X-Forwarded-Proto and Host headers),
        // in case a reverse proxy (Cloudflare, Caddy, nginx) changed the scheme.
        let forwarded_proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("https");
        let host = headers
            .get("x-forwarded-host")
            .or_else(|| headers.get("host"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let proxy_url = format!("{forwarded_proto}://{host}/api/voice/twilio/inbound");

        tracing::debug!(
            expected_url = %full_url,
            proxy_url = %proxy_url,
            has_signature = !sig.is_empty(),
            param_count = params.len(),
            "Inbound call: validating Twilio signature"
        );

        // Build multiple URL candidates to try — reverse proxies (Cloudflare,
        // Caddy, nginx) may change the scheme, causing Twilio to sign a
        // different URL than what we expect.
        let mut url_candidates = vec![full_url.clone(), proxy_url.clone()];

        // Also try http:// variant of the callback_base_url (in case Twilio
        // signed the pre-redirect HTTP URL).
        if full_url.starts_with("https://") {
            url_candidates.push(full_url.replacen("https://", "http://", 1));
        }
        if proxy_url.starts_with("https://") {
            url_candidates.push(proxy_url.replacen("https://", "http://", 1));
        }
        // Also try http:// variant with the configured host (not the proxy host).
        if let Some(host_only) = base_url.strip_prefix("https://").or_else(|| base_url.strip_prefix("http://")) {
            url_candidates.push(format!("http://{host_only}/api/voice/twilio/inbound"));
            url_candidates.push(format!("https://{host_only}/api/voice/twilio/inbound"));
        }

        // Deduplicate while preserving order.
        let mut seen = std::collections::HashSet::new();
        url_candidates.retain(|u| seen.insert(u.clone()));

        let sig_valid = url_candidates.iter().any(|url| {
            if validate_twilio_signature(auth_token, url, &params, sig) {
                tracing::info!(matched_url = %url, "Inbound call: signature validated");
                true
            } else {
                false
            }
        });

        if !sig_valid {
            tracing::warn!(
                tried_urls = ?url_candidates,
                "Inbound call: invalid Twilio signature — rejecting"
            );
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
    let (user_id, caller_name_from_allowlist) = match state
        .find_user_for_caller(&from)
        .await
    {
        Some((uid, name)) => (uid, name),
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
    //    Try by ID first, then fall back to resolving by handle (username).
    // ------------------------------------------------------------------
    let user = match state.user_service.find_by_id(&user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            // Not found by ID — try resolving as a handle/username.
            match crate::core::Handle::try_new(&user_id)
                .ok()
            {
                Some(handle) => {
                    match state.user_service.find_by_handle(&handle).await {
                        Ok(Some(u)) => u,
                        _ => {
                            tracing::warn!(user_id = %user_id, "Inbound call: user not found by ID or handle — rejecting");
                            return twiml_reject(None);
                        }
                    }
                }
                None => {
                    tracing::warn!(user_id = %user_id, "Inbound call: user not found and not a valid handle — rejecting");
                    return twiml_reject(None);
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, user_id = %user_id, "Inbound call: failed to fetch user — rejecting");
            return twiml_reject(None);
        }
    };

    // Everything the call owns — contact, chat, token — is keyed by the real
    // user id. `user_id` above is only the allowlist's key, which the fallback
    // just above allows to be a handle; using it as an owner would create the
    // chat under a value the rest of the platform never matches, and every
    // voice turn would then fail its ownership check.
    let owner_id = user.id.clone();

    // ------------------------------------------------------------------
    // 6. Resolve answering agent
    //    The owning user's own choice wins; otherwise their "receptionist".
    // ------------------------------------------------------------------
    let agent_query = state
        .get_inbound_agent(&user_id)
        .await
        .unwrap_or_else(|| "receptionist".to_string());

    let agent = match resolve_agent_by_query(&state.agent_service, &owner_id, &agent_query).await {
        Some(a) => a,
        None => {
            tracing::warn!(agent_query = %agent_query, "Inbound call: agent not found by ID, handle, or name — rejecting");
            return twiml_reject(None);
        }
    };

    let agent_id = agent.id.clone();

    // ------------------------------------------------------------------
    // 7. Resolve a human-friendly display name for the caller
    //    Priority: an explicit allowlist name the owner set, otherwise the
    //    name on the registered user whose number matches the caller (an
    //    inbound caller is one of our users). Falls back to the raw number so
    //    labels are never empty.
    // ------------------------------------------------------------------
    let caller_display_name = match &caller_name_from_allowlist {
        Some(n) if !n.trim().is_empty() => Some(n.clone()),
        _ => find_user_by_phone(&state.user_service, &from)
            .await
            .map(|u| u.name)
            .filter(|n| !n.trim().is_empty()),
    };

    // ------------------------------------------------------------------
    // 8. Find or create the caller's contact record
    // ------------------------------------------------------------------
    let contact = match state
        .contact_service
        .find_or_create_by_phone(
            &owner_id,
            &from,
            caller_display_name.as_deref().unwrap_or("Incoming caller"),
        )
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, from = %from, "Inbound call: failed to upsert contact — rejecting");
            return twiml_reject(None);
        }
    };

    // ------------------------------------------------------------------
    // 9. Create a new chat for this call
    // ------------------------------------------------------------------
    let chat_title = match &caller_display_name {
        Some(name) => format!("Inbound call from {name}"),
        None => format!("Inbound call from {from}"),
    };
    let chat = match state
        .chat_service
        .create_chat(
            &owner_id,
            CreateChatRequest {
                space_id: None,
                task_id: None,
                agent_id: agent_id.clone(),
                title: Some(chat_title),
                metadata: None,
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
    // 10. Record the call (Ringing → Active immediately for inbound)
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
    // 11. Issue a voice-session JWT (goes directly to the WS handler;
    //     no intermediate callback token needed for inbound calls)
    // ------------------------------------------------------------------
    let ws_ext = match serde_json::to_value(VoiceSessionExtensions {
        chat_id: chat.id.clone(),
        contact_id: Some(contact.id.clone()),
        call_id: Some(call_id.clone()),
        direction: Some(CallDirection::Inbound),
        caller_phone: Some(from.clone()),
        // Prefer the resolved display name (allowlist or matched user) over the
        // contact's stored name, which may be "Incoming caller" from a prior call.
        caller_name: Some(caller_display_name.unwrap_or_else(|| contact.name.clone())),
        transfer_note: None,
    }) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "Inbound call: failed to encode session extensions — rejecting");
            return twiml_reject(None);
        }
    };

    // Build a minimal User value — the token service only needs id/handle/email.
    let token_user = User {
        id: user.id.clone(),
        handle: user.handle.clone(),
        groups: user.groups.clone(),
        deactivated_at: user.deactivated_at.clone(),
        email: user.email.clone(),
        name: user.name.clone(),
        password_hash: String::new(),
        timezone: None,
        phone: None,
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
    // Reuses the same session token — `connect_action` needs the same
    // chat/call/caller context the WS handler does, and Twilio only hits this
    // URL once the relay session it's paired with has already ended.
    let action_url = format!("{base_url}/api/voice/twilio/connect-action?token={}", created.jwt);

    // Per-user greeting wins; otherwise the server-level default.
    let greeting = state
        .get_inbound_greeting(&user_id)
        .await
        .or_else(|| state.config.voice.inbound_welcome_greeting.clone());

    let twiml = build_twiml(
        &ws_url,
        TwimlOptions {
            welcome_greeting: greeting.as_deref(),
            hints: None, // not applicable for inbound
            action: Some(&action_url),
            voice_id: agent.voice_id.as_deref(),
        },
        &state.config.voice,
    );

    tracing::info!(
        from = %from,
        chat_id = %chat.id,
        call_id = %call_id,
        user_id = %owner_id,
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
        params.insert("From".into(), "+155****0000".into());
        params.insert("CallSid".into(), "CA123".into());

        // Build expected: url + "CallSid" + "CA123" + "From" + "+155****0000" (sorted)
        let s = format!("{}CallSidCA123From+155****0000", url);
        let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
        mac.update(s.as_bytes());
        let result = mac.finalize().into_bytes();
        let expected_sig = base64::engine::general_purpose::STANDARD.encode(result);

        assert!(validate_twilio_signature(auth_token, url, &params, &expected_sig));
    }
}
