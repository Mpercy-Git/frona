use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha1::Sha1;

use crate::chat::channel::adapter::markdown;
use crate::chat::channel::manager::CarrierStatus;
use crate::chat::message::models::Message;
use crate::chat::models::Chat;
use crate::core::error::AppError;

use super::super::models::{
    ChannelAdapter, ChannelCtx, ExternalMessage, external_chat_id,
};
#[cfg(test)]
use super::super::models::{ChannelFactory, ConfigRef};

const TWIML_EMPTY_RESPONSE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Response/>";
const TWILIO_API_BASE: &str = "https://api.twilio.com/2010-04-01";

#[derive(Debug, Clone, Deserialize, crate::ChannelFactory)]
#[channel(id = "sms")]
pub struct SmsAdapter {
    pub account_sid: String,
    pub auth_token: String,
    pub from_number: String,
}

impl SmsAdapter {
    fn twilio(&self) -> TwilioApi<'_> {
        TwilioApi {
            account_sid: &self.account_sid,
            auth_token: &self.auth_token,
        }
    }
}

#[async_trait]
impl ChannelAdapter for SmsAdapter {
    async fn on_connect(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        let sid = self
            .twilio()
            .register_messaging_webhook(&self.from_number, &ctx.webhook_url)
            .await
            .map_err(|e| {
                tracing::warn!(
                    channel_id = %ctx.channel.id,
                    from_number = %self.from_number,
                    url = %ctx.webhook_url,
                    error = %e,
                    "SMS channel could not auto-register Twilio webhook — channel will be marked Failed (fix the underlying issue and restart, or paste the URL into the Twilio console manually)",
                );
                e
            })?;
        tracing::info!(
            channel_id = %ctx.channel.id,
            phone_number_sid = %sid,
            from_number = %self.from_number,
            url = %ctx.webhook_url,
            "SMS channel registered Twilio Messaging webhook",
        );
        Ok(())
    }

    async fn on_disconnect(&self, _ctx: &ChannelCtx) -> Result<(), AppError> {
        Ok(())
    }

    async fn on_send(
        &self,
        msg: &Message,
        tool_calls: &[crate::inference::tool_call::ToolCall],
        chat: &Chat,
        ctx: &ChannelCtx,
    ) -> Result<(), AppError> {
        let body = compose_sms_body(tool_calls, &msg.content);

        if body.trim().is_empty() {
            return Ok(());
        }

        let to_number = parse_external_id(external_chat_id(chat)?)?;
        let status_callback = status_callback_url(&ctx.webhook_url, &msg.id);

        tracing::info!(
            channel_id = %ctx.channel.id,
            msg_id = %msg.id,
            from = %self.from_number,
            to = %to_number,
            content_len = body.len(),
            tool_count = tool_calls.len(),
            "SMS on_send: dispatching composed body to Twilio",
        );
        self.twilio()
            .send_message(&self.from_number, &to_number, &body, &status_callback)
            .await
            .map_err(|e| {
                tracing::warn!(
                    channel_id = %ctx.channel.id,
                    msg_id = %msg.id,
                    to = %to_number,
                    error = %e,
                    "SMS on_send: Twilio synchronously rejected message",
                );
                e
            })?;
        tracing::debug!(
            channel_id = %ctx.channel.id,
            msg_id = %msg.id,
            to = %to_number,
            "SMS on_send: Twilio accepted message (carrier delivery status will arrive via webhook)",
        );
        Ok(())
    }

    async fn on_webhook(
        &self,
        ctx: &ChannelCtx,
        request: Request<Bytes>,
    ) -> Result<Response, AppError> {
        let Some(signature) = header_str(&request, "X-Twilio-Signature") else {
            return Ok(forbidden("missing X-Twilio-Signature"));
        };
        let canonical_url = canonical_webhook_url(&ctx.webhook_url, request.uri().query());
        let raw_params = parse_form_body(request.body());
        if !verify_twilio_signature(&self.auth_token, &canonical_url, &raw_params, &signature) {
            return Ok(forbidden("Twilio signature mismatch"));
        }

        let webhook = TwilioWebhook::from_pairs(&raw_params);
        if webhook.is_status_callback() {
            webhook.apply_carrier_status(ctx, request.uri().query()).await;
            return Ok(ok_twiml());
        }
        if webhook.body.is_empty() && webhook.num_media == "0" {
            return Ok(ok_twiml());
        }
        if webhook.from.is_empty() {
            return Err(AppError::Validation(
                "Twilio webhook missing From".into(),
            ));
        }
        webhook.emit_inbound(ctx).await?;
        Ok(ok_twiml())
    }
}


struct TwilioApi<'a> {
    account_sid: &'a str,
    auth_token: &'a str,
}

impl TwilioApi<'_> {
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.basic_auth(self.account_sid, Some(self.auth_token))
    }

    fn url(&self, path: &str) -> String {
        format!("{TWILIO_API_BASE}/Accounts/{}/{path}", self.account_sid)
    }

    async fn send_message(
        &self,
        from: &str,
        to: &str,
        body: &str,
        status_callback: &str,
    ) -> Result<String, AppError> {
        #[derive(Deserialize)]
        struct Out {
            sid: String,
        }
        let resp = self
            .auth(crate::build_http_client().post(self.url("Messages.json")))
            .form(&[
                ("From", from),
                ("To", to),
                ("Body", body),
                ("StatusCallback", status_callback),
            ])
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Twilio send Messages: {e}")))?;
        let parsed: Out = ok_json(resp, "send Messages").await?;
        Ok(parsed.sid)
    }

    async fn register_messaging_webhook(
        &self,
        phone_number: &str,
        webhook_url: &str,
    ) -> Result<String, AppError> {
        #[derive(Deserialize)]
        struct PhoneNumber {
            sid: String,
        }
        #[derive(Deserialize)]
        struct ListResponse {
            incoming_phone_numbers: Vec<PhoneNumber>,
        }

        let client = crate::build_http_client();
        let list: ListResponse = ok_json(
            self.auth(client.get(self.url("IncomingPhoneNumbers.json")))
                .query(&[("PhoneNumber", phone_number)])
                .send()
                .await
                .map_err(|e| AppError::Internal(format!("Twilio list IncomingPhoneNumbers: {e}")))?,
            "list IncomingPhoneNumbers",
        )
        .await?;
        let sid = list
            .incoming_phone_numbers
            .into_iter()
            .next()
            .map(|n| n.sid)
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "Twilio account does not own {phone_number} — buy or port the number, or correct the from_number config",
                ))
            })?;

        ok_empty(
            self.auth(client.post(self.url(&format!("IncomingPhoneNumbers/{sid}.json"))))
                .form(&[("SmsUrl", webhook_url), ("SmsMethod", "POST")])
                .send()
                .await
                .map_err(|e| AppError::Internal(format!("Twilio update IncomingPhoneNumber: {e}")))?,
            "update IncomingPhoneNumber",
        )
        .await?;
        Ok(sid)
    }
}

async fn ok_json<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
    op: &str,
) -> Result<T, AppError> {
    let resp = ensure_status(resp, op).await?;
    resp.json()
        .await
        .map_err(|e| AppError::Internal(format!("Twilio {op} parse: {e}")))
}

async fn ok_empty(resp: reqwest::Response, op: &str) -> Result<(), AppError> {
    ensure_status(resp, op).await.map(|_| ())
}

async fn ensure_status(resp: reqwest::Response, op: &str) -> Result<reqwest::Response, AppError> {
    if resp.status().is_success() {
        Ok(resp)
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(AppError::Internal(format!("Twilio {op} HTTP {status}: {body}")))
    }
}

#[derive(Debug, Default)]
struct TwilioWebhook {
    from: String,
    body: String,
    message_sid: String,
    num_media: String,
    message_status: String,
    error_code: String,
    error_message: String,
}

impl TwilioWebhook {
    fn from_pairs(pairs: &[(String, String)]) -> Self {
        let mut p = Self {
            num_media: "0".into(),
            ..Self::default()
        };
        for (k, v) in pairs {
            match k.as_str() {
                "From" => p.from = v.clone(),
                "Body" => p.body = v.clone(),
                "MessageSid" => p.message_sid = v.clone(),
                "NumMedia" => p.num_media = v.clone(),
                "MessageStatus" => p.message_status = v.clone(),
                "ErrorCode" => p.error_code = v.clone(),
                "ErrorMessage" => p.error_message = v.clone(),
                _ => {}
            }
        }
        p
    }

    // Twilio inbound also sets MessageStatus, so discriminate on body/media.
    fn is_status_callback(&self) -> bool {
        !self.message_status.is_empty() && self.body.is_empty() && self.num_media == "0"
    }

    async fn apply_carrier_status(&self, ctx: &ChannelCtx, query: Option<&str>) {
        let our_msg_id = msg_id_from_query(query);
        match self.message_status.as_str() {
            "delivered" => {
                if let Some(id) = our_msg_id.as_deref() {
                    let _ = ctx
                        .channel_manager
                        .record_carrier_status(id, CarrierStatus::Delivered)
                        .await;
                }
                tracing::debug!(
                    channel_id = %ctx.channel.id,
                    twilio_sid = %self.message_sid,
                    msg_id = ?our_msg_id,
                    "SMS delivery status: delivered",
                );
            }
            "failed" | "undelivered" => {
                let error = self.format_carrier_error();
                if let Some(id) = our_msg_id.as_deref() {
                    let _ = ctx
                        .channel_manager
                        .record_carrier_status(id, CarrierStatus::Failed { error: error.clone() })
                        .await;
                }
                tracing::warn!(
                    channel_id = %ctx.channel.id,
                    twilio_sid = %self.message_sid,
                    msg_id = ?our_msg_id,
                    status = %self.message_status,
                    error_code = %self.error_code,
                    error_message = %self.error_message,
                    "SMS delivery failed at carrier (e.g. unregistered 10DLC, blocked recipient, invalid number)",
                );
            }
            other => tracing::debug!(
                channel_id = %ctx.channel.id,
                twilio_sid = %self.message_sid,
                status = %other,
                "SMS delivery status update",
            ),
        }
    }

    async fn emit_inbound(&self, ctx: &ChannelCtx) -> Result<(), AppError> {
        tracing::debug!(
            channel_id = %ctx.channel.id,
            from = %self.from,
            message_sid = %self.message_sid,
            "SMS webhook accepted — emitting to inbound pipeline",
        );
        let event = ExternalMessage {
            external_chat_id: format!("sms:{}", self.from),
            sender_address: self.from.clone(),
            sender_external_id: Some(self.from.clone()),
            sender_display_name: Some(self.from.clone()),
            content: self.body.clone(),
        };
        ctx.emit
            .send(event)
            .await
            .map_err(|e| AppError::Internal(format!("inbound emit channel closed: {e}")))
    }

    fn format_carrier_error(&self) -> String {
        if self.error_message.is_empty() {
            format!("Twilio {} ({})", self.message_status, self.error_code)
        } else {
            format!("{} ({} {})", self.error_message, self.message_status, self.error_code)
        }
    }
}

fn parse_form_body(body: &[u8]) -> Vec<(String, String)> {
    let mut params: Vec<(String, String)> =
        url::form_urlencoded::parse(body).into_owned().collect();
    params.sort_by(|a, b| a.0.cmp(&b.0));
    params
}

fn header_str<B>(request: &Request<B>, name: &str) -> Option<String> {
    request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn msg_id_from_query(query: Option<&str>) -> Option<String> {
    let q = query?;
    url::form_urlencoded::parse(q.as_bytes())
        .find(|(k, _)| k == "msg_id")
        .map(|(_, v)| v.into_owned())
}

// Twilio echoes the query back, letting carrier-status skip a DB lookup.
fn status_callback_url(webhook_url: &str, msg_id: &str) -> String {
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("msg_id", msg_id)
        .finish();
    format!("{webhook_url}?{encoded}")
}

fn canonical_webhook_url(webhook_url: &str, query: Option<&str>) -> String {
    match query {
        Some(q) if !q.is_empty() => format!("{webhook_url}?{q}"),
        _ => webhook_url.to_string(),
    }
}

// HMAC-SHA1 of url + sorted(k+v).concat(), base64; constant-time compare.
fn verify_twilio_signature(
    auth_token: &str,
    url: &str,
    params: &[(String, String)],
    provided_b64: &str,
) -> bool {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut canonical = String::with_capacity(url.len());
    canonical.push_str(url);
    for (k, v) in &sorted {
        canonical.push_str(k);
        canonical.push_str(v);
    }

    let mut mac = match Hmac::<Sha1>::new_from_slice(auth_token.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(canonical.as_bytes());
    let expected = mac.finalize().into_bytes();

    let provided = match base64::engine::general_purpose::STANDARD.decode(provided_b64) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    if provided.len() != expected.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (a, b) in expected.iter().zip(provided.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn parse_external_id(s: &str) -> Result<String, AppError> {
    match s.strip_prefix("sms:") {
        Some(rest) if !rest.is_empty() => Ok(rest.to_string()),
        _ => Err(AppError::Validation(format!(
            "unrecognised SMS external_id format: {s:?}"
        ))),
    }
}

fn compose_sms_body(
    tool_calls: &[crate::inference::tool_call::ToolCall],
    trailing: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    for tc in tool_calls {
        if let Some(text) = tc.turn_text.as_deref() {
            let plain = markdown::to_plain(text);
            let trimmed = plain.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    let trailing_plain = markdown::to_plain(trailing);
    let trailing_trimmed = trailing_plain.trim();
    if !trailing_trimmed.is_empty() {
        parts.push(trailing_trimmed.to_string());
    }
    parts.join("\n\n")
}

fn forbidden(detail: &str) -> Response {
    let mut response = (StatusCode::FORBIDDEN, detail.to_string()).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain"),
    );
    response
}

fn ok_twiml() -> Response {
    let mut response = TWIML_EMPTY_RESPONSE.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn manifest_has_required_fields_with_default_from() {
        let m = SmsAdapterFactory.manifest();
        assert_eq!(m.id, "sms");
        assert_eq!(m.display_name, "Twilio SMS");
        let by_name = |name: &str| {
            m.config_fields
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("field {name} missing"))
        };
        let sid = by_name("account_sid");
        assert!(sid.is_required && sid.is_secret);
        assert_eq!(
            sid.default_from,
            Some(ConfigRef {
                section: "voice".into(),
                field: "twilio_account_sid".into(),
            })
        );
        assert_eq!(sid.default_resolved, None);
        let token = by_name("auth_token");
        assert!(token.is_required && token.is_secret);
        assert_eq!(
            token.default_from,
            Some(ConfigRef {
                section: "voice".into(),
                field: "twilio_auth_token".into(),
            })
        );
        let from = by_name("from_number");
        assert!(from.is_required && !from.is_secret);
        assert_eq!(
            from.default_from,
            Some(ConfigRef {
                section: "voice".into(),
                field: "twilio_from_number".into(),
            })
        );
    }

    #[test]
    fn factory_create_with_valid_config_succeeds() {
        let cfg = json!({
            "account_sid": "ACxxx",
            "auth_token": "tok",
            "from_number": "+15550000000",
        });
        SmsAdapterFactory
            .create(cfg)
            .expect("valid config should produce a Channel");
    }

    #[test]
    fn factory_create_rejects_missing_account_sid() {
        let cfg = json!({"auth_token": "tok", "from_number": "+1"});
        assert!(matches!(SmsAdapterFactory.create(cfg), Err(AppError::Validation(_))));
    }

    #[test]
    fn factory_create_rejects_missing_auth_token() {
        let cfg = json!({"account_sid": "AC", "from_number": "+1"});
        assert!(matches!(SmsAdapterFactory.create(cfg), Err(AppError::Validation(_))));
    }

    #[test]
    fn factory_create_rejects_missing_from_number() {
        let cfg = json!({"account_sid": "AC", "auth_token": "tok"});
        assert!(matches!(SmsAdapterFactory.create(cfg), Err(AppError::Validation(_))));
    }

    #[test]
    fn parse_external_id_e164() {
        assert_eq!(
            parse_external_id("sms:+15551234567").unwrap(),
            "+15551234567"
        );
    }

    #[test]
    fn parse_external_id_rejects_garbage() {
        assert!(parse_external_id("nonsense").is_err());
        assert!(parse_external_id("sms:").is_err());
        assert!(parse_external_id("dm:+15551234567").is_err());
    }

    #[test]
    fn webhook_params_extracts_known_fields() {
        let pairs = vec![
            ("From".into(), "+1".into()),
            ("Body".into(), "hi".into()),
            ("MessageSid".into(), "SM1".into()),
            ("NumMedia".into(), "2".into()),
            ("MessageStatus".into(), "delivered".into()),
            ("ErrorCode".into(), "30001".into()),
            ("ErrorMessage".into(), "queue overflow".into()),
            ("Unknown".into(), "ignored".into()),
        ];
        let p = TwilioWebhook::from_pairs(&pairs);
        assert_eq!(p.from, "+1");
        assert_eq!(p.body, "hi");
        assert_eq!(p.message_sid, "SM1");
        assert_eq!(p.num_media, "2");
        assert_eq!(p.message_status, "delivered");
        assert_eq!(p.error_code, "30001");
        assert_eq!(p.error_message, "queue overflow");
    }

    #[test]
    fn webhook_params_defaults_num_media_to_zero() {
        let p = TwilioWebhook::from_pairs(&[]);
        assert_eq!(p.num_media, "0");
        assert_eq!(p.from, "");
    }

    #[test]
    fn is_status_callback_true_for_twilio_status_payload() {
        let p = TwilioWebhook {
            message_status: "delivered".into(),
            message_sid: "SM1".into(),
            num_media: "0".into(),
            ..Default::default()
        };
        assert!(p.is_status_callback());
    }

    #[test]
    fn is_status_callback_false_for_inbound_with_received_status() {
        let p = TwilioWebhook {
            message_status: "received".into(),
            from: "+15551234567".into(),
            body: "hello".into(),
            message_sid: "SM2".into(),
            ..Default::default()
        };
        assert!(!p.is_status_callback());
    }

    #[test]
    fn is_status_callback_false_for_inbound_mms_with_no_body() {
        let p = TwilioWebhook {
            message_status: "received".into(),
            from: "+15551234567".into(),
            body: String::new(),
            num_media: "1".into(),
            message_sid: "SM3".into(),
            ..Default::default()
        };
        assert!(!p.is_status_callback());
    }

    #[test]
    fn is_status_callback_false_for_empty_ping() {
        let p = TwilioWebhook::default();
        assert!(!p.is_status_callback());
    }

    #[test]
    fn msg_id_from_query_round_trips_uuid() {
        let id = "5c6450c3-19aa-4ab2-84fd-b08b7359f81d";
        let q = format!("msg_id={id}");
        assert_eq!(msg_id_from_query(Some(&q)).as_deref(), Some(id));
        assert_eq!(msg_id_from_query(None), None);
        assert_eq!(msg_id_from_query(Some("other=1")), None);
    }

    #[test]
    fn status_callback_url_appends_msg_id_query() {
        let url = status_callback_url(
            "https://x.com/api/webhooks/channels/sms/abc",
            "msg-1",
        );
        assert_eq!(
            url,
            "https://x.com/api/webhooks/channels/sms/abc?msg_id=msg-1",
        );
    }

    #[test]
    fn format_carrier_error_with_message() {
        let p = TwilioWebhook {
            message_status: "failed".into(),
            error_code: "30007".into(),
            error_message: "carrier rejected".into(),
            ..Default::default()
        };
        assert_eq!(p.format_carrier_error(), "carrier rejected (failed 30007)");
    }

    #[test]
    fn format_carrier_error_without_message() {
        let p = TwilioWebhook {
            message_status: "undelivered".into(),
            error_code: "30005".into(),
            ..Default::default()
        };
        assert_eq!(p.format_carrier_error(), "Twilio undelivered (30005)");
    }

    fn fixture_signature() -> (
        String,
        String,
        Vec<(String, String)>,
        String,
    ) {
        let token = "12345".to_string();
        let url = "https://mycompany.com/myapp.php?foo=1&bar=2".to_string();
        let params: Vec<(String, String)> = vec![
            ("CallSid".into(), "CA1234567890ABCDE".into()),
            ("Caller".into(), "+14158675309".into()),
            ("Digits".into(), "1234".into()),
            ("From".into(), "+14158675309".into()),
            ("To".into(), "+18005551212".into()),
        ];
        let mut canonical = String::from(&url);
        let mut sorted = params.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, v) in &sorted {
            canonical.push_str(k);
            canonical.push_str(v);
        }
        let mut mac = Hmac::<Sha1>::new_from_slice(token.as_bytes()).unwrap();
        mac.update(canonical.as_bytes());
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        (token, url, params, sig)
    }

    #[test]
    fn verify_twilio_signature_known_vector() {
        let (token, url, params, sig) = fixture_signature();
        assert!(verify_twilio_signature(&token, &url, &params, &sig));
    }

    #[test]
    fn verify_twilio_signature_mismatch_returns_false() {
        let (token, url, params, _sig) = fixture_signature();
        assert!(!verify_twilio_signature(
            &token,
            &url,
            &params,
            "AAAAAAAAAAAAAAAAAAAAAAAAAAA="
        ));
    }

    #[test]
    fn verify_twilio_signature_param_reorder_still_validates() {
        let (token, url, params, sig) = fixture_signature();
        let mut shuffled = params;
        shuffled.reverse();
        assert!(verify_twilio_signature(&token, &url, &shuffled, &sig));
    }

    #[test]
    fn canonical_webhook_url_includes_query_when_present() {
        let base = "https://x.com/api/webhooks/channels/ch:1";
        assert_eq!(canonical_webhook_url(base, None), base);
        assert_eq!(
            canonical_webhook_url(base, Some("a=1&b=2")),
            format!("{base}?a=1&b=2"),
        );
    }

    fn tool_call_with_turn_text(text: Option<&str>) -> crate::inference::tool_call::ToolCall {
        crate::inference::tool_call::ToolCall {
            id: "tc-1".into(),
            chat_id: "chat-1".into(),
            message_id: "msg-1".into(),
            turn: 0,
            provider_call_id: "pc-1".into(),
            name: "any".into(),
            arguments: serde_json::Value::Null,
            result: String::new(),
            success: true,
            duration_ms: 0,
            tool_data: None,
            system_prompt: None,
            description: None,
            turn_text: text.map(String::from),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn compose_sms_body_joins_turn_texts_and_trailing_with_blank_line() {
        let tcs = vec![
            tool_call_with_turn_text(Some("hello")),
            tool_call_with_turn_text(Some("here is the result")),
        ];
        let body = compose_sms_body(&tcs, "anything else?");
        assert_eq!(body, "hello\n\nhere is the result\n\nanything else?");
    }

    #[test]
    fn compose_sms_body_skips_empty_and_whitespace_turn_texts() {
        let tcs = vec![
            tool_call_with_turn_text(Some("")),
            tool_call_with_turn_text(Some("   \n  ")),
            tool_call_with_turn_text(None),
            tool_call_with_turn_text(Some("real")),
        ];
        let body = compose_sms_body(&tcs, "tail");
        assert_eq!(body, "real\n\ntail");
    }

    #[test]
    fn compose_sms_body_returns_only_trailing_when_no_tools() {
        let body = compose_sms_body(&[], "just trailing");
        assert_eq!(body, "just trailing");
    }

    #[test]
    fn compose_sms_body_returns_only_turn_texts_when_trailing_empty() {
        let tcs = vec![tool_call_with_turn_text(Some("a")), tool_call_with_turn_text(Some("b"))];
        let body = compose_sms_body(&tcs, "");
        assert_eq!(body, "a\n\nb");
    }

    #[test]
    fn compose_sms_body_empty_when_no_input() {
        let body = compose_sms_body(&[], "");
        assert_eq!(body, "");
    }

    #[test]
    fn compose_sms_body_strips_markdown_formatting() {
        let tcs = vec![tool_call_with_turn_text(Some("**bold** here"))];
        let body = compose_sms_body(&tcs, "");
        assert!(!body.contains("**"));
        assert!(body.contains("bold"));
    }
}
