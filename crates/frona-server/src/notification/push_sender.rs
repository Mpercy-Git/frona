use std::sync::Arc;

use serde::Serialize;
use web_push::{
    ContentEncoding, SubscriptionInfo, Urgency, VapidSignatureBuilder, WebPushClient, WebPushError,
    WebPushMessage, WebPushMessageBuilder,
};

use crate::core::config::PushConfig;
use crate::core::error::AppError;
use crate::notification::egress::{PublicOnlyResolver, validate_endpoint};
use crate::notification::models::{NotificationData, PushExtras};
use crate::notification::push_repository::PushSubscriptionRepository;

/// Outcome of pushing one notification to every device a user has registered.
///
/// Web Push is fire-and-forget for normal notifications, but the diagnostics
/// endpoint (`POST /api/push/test`) hands this back to the UI: when a device
/// stays silent, "the server had 0 subscriptions" and "FCM rejected our VAPID
/// signature" are very different problems and the user cannot tell them apart
/// from the notification tray.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PushDeliveryReport {
    /// Subscriptions we tried to deliver to.
    pub attempted: usize,
    /// Subscriptions the push service accepted.
    pub delivered: usize,
    /// Subscriptions that were expired and have now been pruned.
    pub removed: usize,
    /// Endpoints of the pruned subscriptions. Never serialised — an endpoint
    /// is a capability URL — but the test route compares them with the
    /// calling device's own endpoint so it can tell that device its local
    /// subscription is dead and must be replaced.
    #[serde(skip)]
    pub removed_endpoints: Vec<String>,
    /// Per-subscription failures, safe to show to the owning user.
    pub failures: Vec<PushFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushFailure {
    /// Host of the push service (e.g. `fcm.googleapis.com`). The full endpoint
    /// is a bearer-secret-ish URL, so only the host is reported.
    pub service: String,
    pub reason: String,
}

/// Most pushes one user can cause per [`DELIVERY_WINDOW`], across all their
/// devices. Each is an outbound request the server makes on their behalf.
const MAX_DELIVERIES_PER_WINDOW: usize = 120;
const DELIVERY_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);
/// Cap on the response a push service may send back; matches the web-push crate.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
/// Longest remote-supplied text shown to the user in a delivery report.
const MAX_REASON_CHARS: usize = 200;

/// Web Push client that cannot be turned on the server's own network.
///
/// The stock client follows redirects, never times out and connects wherever
/// the endpoint's name resolves. This one never follows a redirect, bounds the
/// request, and resolves names through [`PublicOnlyResolver`], so a
/// subscription cannot reach loopback, private or link-local addresses even if
/// its hostname is rebound after it was stored.
struct GuardedPushClient {
    http: reqwest::Client,
}

impl GuardedPushClient {
    fn new() -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .dns_resolver(PublicOnlyResolver)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Internal(format!("Failed to create Web Push client: {e}")))?;
        Ok(Self { http })
    }
}

fn transport_error(message: impl Into<String>) -> WebPushError {
    WebPushError::Io(std::io::Error::other(message.into()))
}

#[async_trait::async_trait]
impl WebPushClient for GuardedPushClient {
    async fn send(&self, message: WebPushMessage) -> Result<(), WebPushError> {
        // Covers an IP-literal host, which never reaches the resolver, and any
        // subscription stored before these checks existed.
        if validate_endpoint(&message.endpoint.to_string()).is_err() {
            return Err(WebPushError::InvalidUri);
        }

        // web-push builds an `http` 0.2 request and reqwest speaks `http` 1.x,
        // so carry it across by hand.
        let (parts, body) =
            web_push::request_builder::build_request::<Vec<u8>>(message).into_parts();
        let mut request = self.http.post(parts.uri.to_string()).body(body);
        for (name, value) in &parts.headers {
            request = request.header(name.as_str(), value.as_bytes());
        }
        let request = request.build().map_err(|_| WebPushError::InvalidUri)?;
        let mut response = self
            .http
            .execute(request)
            .await
            .map_err(|e| transport_error(transport_reason(&e)))?;

        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| transport_error(transport_reason(&e)))?
        {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(WebPushError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let status = http02::StatusCode::from_u16(status.as_u16())
            .map_err(|_| WebPushError::InvalidResponse)?;
        web_push::request_builder::parse_response(status, body)
    }
}

/// Why a request failed, without the URL (an endpoint is a capability URL).
fn transport_reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "The push service did not respond in time".into()
    } else if error.is_connect() {
        "Could not connect to the push service (the address may be blocked)".into()
    } else if error.is_redirect() {
        "The push service redirected the request, which is not followed".into()
    } else {
        "The request to the push service failed".into()
    }
}

/// Remote-supplied text, bounded and stripped of control characters, before it
/// reaches a delivery report.
fn bounded_reason(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_REASON_CHARS)
        .collect()
}

/// Sliding-window cap on deliveries per user.
#[derive(Default)]
struct DeliveryLimiter {
    sent: std::sync::Mutex<
        std::collections::HashMap<String, std::collections::VecDeque<std::time::Instant>>,
    >,
}

impl DeliveryLimiter {
    /// Reserves one delivery for `user_id`; false once the window is full.
    fn allow(&self, user_id: &str) -> bool {
        let now = std::time::Instant::now();
        let mut sent = self.sent.lock().unwrap_or_else(|e| e.into_inner());
        sent.retain(|_, times| {
            while times
                .front()
                .is_some_and(|t| now.duration_since(*t) > DELIVERY_WINDOW)
            {
                times.pop_front();
            }
            !times.is_empty()
        });
        let times = sent.entry(user_id.to_string()).or_default();
        if times.len() >= MAX_DELIVERIES_PER_WINDOW {
            return false;
        }
        times.push_back(now);
        true
    }
}

pub struct PushSender {
    client: GuardedPushClient,
    limiter: DeliveryLimiter,
    /// Base64-encoded VAPID private key (no subscription info bound).
    vapid_private_key: String,
    vapid_subject: String,
    repo: Arc<dyn PushSubscriptionRepository>,
}

impl PushSender {
    /// Returns `Some(PushSender)` if VAPID keys are configured, else `None`.
    pub fn new(
        config: &PushConfig,
        repo: Arc<dyn PushSubscriptionRepository>,
    ) -> Result<Option<Self>, AppError> {
        let private_key = match &config.vapid_private_key {
            Some(k) if !k.trim().is_empty() => k.trim().to_string(),
            _ => {
                // A public key on its own is worse than no config at all:
                // browsers subscribe happily and the server can never send, so
                // notifications silently never arrive. Say so at startup.
                if config
                    .vapid_public_key
                    .as_ref()
                    .is_some_and(|k| !k.trim().is_empty())
                {
                    tracing::error!(
                        "Push notifications disabled: a VAPID public key is configured but the \
                         private key is missing. Devices will subscribe successfully and then \
                         never receive anything. Set FRONA_PUSH_VAPID_PRIVATE_KEY."
                    );
                }
                return Ok(None);
            }
        };

        // Validate the key is usable by trying to build a partial signature.
        let _ = VapidSignatureBuilder::from_base64_no_sub(&private_key)
            .map_err(|e| AppError::Internal(format!("Invalid VAPID private key: {e}")))?;

        let client = GuardedPushClient::new()?;

        Ok(Some(Self {
            client,
            limiter: DeliveryLimiter::default(),
            vapid_private_key: private_key,
            vapid_subject: config.subject.clone(),
            repo,
        }))
    }

    /// Send a push notification to all of the user's subscriptions.
    /// Fire-and-forget — logs errors but doesn't fail the caller.
    pub async fn send_to_user(
        &self,
        user_id: &str,
        notification: &crate::notification::models::Notification,
        extras: &PushExtras,
    ) {
        let report = self.deliver_to_user(user_id, notification, extras).await;
        if report.attempted == 0 {
            tracing::debug!(user_id, "No push subscriptions registered; nothing to send");
        } else if report.delivered == 0 {
            tracing::warn!(
                user_id,
                attempted = report.attempted,
                "Push notification reached none of the user's devices"
            );
        } else {
            tracing::debug!(
                user_id,
                delivered = report.delivered,
                attempted = report.attempted,
                "Push notification sent"
            );
        }
    }

    /// Send a push notification to all of the user's subscriptions and report
    /// what happened to each one.
    pub async fn deliver_to_user(
        &self,
        user_id: &str,
        notification: &crate::notification::models::Notification,
        extras: &PushExtras,
    ) -> PushDeliveryReport {
        let mut report = PushDeliveryReport::default();

        let subs = match self.repo.find_by_user_id(user_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, user_id, "Failed to fetch push subscriptions");
                report.failures.push(PushFailure {
                    service: "server".into(),
                    reason: format!("Could not read push subscriptions: {e}"),
                });
                return report;
            }
        };

        if subs.is_empty() {
            return report;
        }
        report.attempted = subs.len();

        let payload = Self::payload(notification, extras);

        let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();
        let ttl = 86400u32; // 24 hours

        for sub in subs {
            let service = Self::endpoint_host(&sub.endpoint);
            if !self.limiter.allow(user_id) {
                report.failures.push(PushFailure {
                    service,
                    reason: "Too many push notifications in the last minute; this one was skipped."
                        .into(),
                });
                continue;
            }
            let subscription_info =
                SubscriptionInfo::new(&sub.endpoint, &sub.p256dh_key, &sub.auth_secret);

            // Build VAPID signature (needs per-subscription info). The `sub`
            // claim is REQUIRED by FCM (Android/Chrome): without it the push is
            // rejected and no system notification appears, even though more
            // lenient push services (e.g. Mozilla autopush) still deliver.
            let sig = match VapidSignatureBuilder::from_base64_no_sub(&self.vapid_private_key) {
                Ok(builder) => {
                    let mut builder = builder.add_sub_info(&subscription_info);
                    builder.add_claim("sub", self.vapid_subject.as_str());
                    builder.build()
                }
                Err(e) => {
                    tracing::warn!(error = %e, "VAPID signature build failed");
                    report.failures.push(PushFailure {
                        service,
                        reason: format!("VAPID signature build failed: {e}"),
                    });
                    continue;
                }
            };

            let sig = match sig {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "VAPID signature build failed");
                    report.failures.push(PushFailure {
                        service,
                        reason: format!("VAPID signature build failed: {e}"),
                    });
                    continue;
                }
            };

            let mut builder = WebPushMessageBuilder::new(&subscription_info);
            builder.set_payload(ContentEncoding::Aes128Gcm, &payload_bytes);
            builder.set_ttl(ttl);
            // Android is the reason this is not left at the default.
            //
            // FCM maps Web Push urgency onto Android message priority, and a
            // "normal" priority message is held while the device is in Doze:
            // it is delivered at the next maintenance window, which can be many
            // minutes later, or coalesced away entirely. The notification then
            // never reaches the system tray when the phone is idle in a pocket
            // — exactly when a push is worth having. `high` wakes the device
            // and delivers immediately, which is the correct urgency for a
            // user-visible notification (and what `userVisibleOnly` promises
            // the push service we will show).
            builder.set_urgency(Urgency::High);
            builder.set_vapid_signature(sig);

            let message = match builder.build() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, endpoint = %sub.endpoint, "Push message build failed");
                    report.failures.push(PushFailure {
                        service,
                        reason: format!("Message build failed: {e}"),
                    });
                    continue;
                }
            };

            match self.client.send(message).await {
                Ok(_) => report.delivered += 1,
                Err(WebPushError::EndpointNotValid(_)) | Err(WebPushError::EndpointNotFound(_)) => {
                    tracing::info!(endpoint = %sub.endpoint, "Removing expired push subscription");
                    let _ = self.repo.delete_by_endpoint(user_id, &sub.endpoint).await;
                    report.removed += 1;
                    report.removed_endpoints.push(sub.endpoint.clone());
                    report.failures.push(PushFailure {
                        service,
                        reason: "Subscription expired and was removed — re-enable notifications \
                                 on that device."
                            .into(),
                    });
                }
                Err(e @ WebPushError::Unauthorized(_)) => {
                    // The push service rejected our VAPID JWT. Almost always a
                    // key mismatch: the device subscribed with a different
                    // applicationServerKey than the one the server now signs
                    // with (keys regenerated, or public/private from different
                    // pairs). Every push fails silently, so make it loud.
                    tracing::error!(
                        error = %e,
                        endpoint = %sub.endpoint,
                        "Push rejected as unauthorized — the VAPID key pair does not match the \
                         key this device subscribed with. Check that the configured public and \
                         private keys are from the same pair, then re-subscribe the device."
                    );
                    report.failures.push(PushFailure {
                        service,
                        reason: "Push service rejected the server's VAPID key. The device \
                                 subscribed with a different key — re-enable notifications on \
                                 it, or check the server's VAPID key pair."
                            .into(),
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, endpoint = %sub.endpoint, "Push send failed");
                    report.failures.push(PushFailure {
                        service,
                        reason: bounded_reason(&e.to_string()),
                    });
                }
            }
        }

        report
    }

    /// Host of a push endpoint, for user-facing diagnostics. The full URL is a
    /// capability token, so it is never reported back.
    fn endpoint_host(endpoint: &str) -> String {
        endpoint
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("push service")
            .to_string()
    }

    /// Map notification data to a deep-link URL for click-through.
    /// The JSON the service worker reads (`web/public/sw.js`). `image` and
    /// `actions` appear only when there is something to show, keeping an
    /// ordinary push the same as before.
    fn payload(
        notification: &crate::notification::models::Notification,
        extras: &PushExtras,
    ) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "id": notification.id,
            "title": notification.title,
            "body": notification.body,
            "level": format!("{:?}", notification.level).to_lowercase(),
            "data": notification.data,
            "url": Self::deep_link(&notification.data),
        });
        if let Some(image) = &extras.image {
            payload["image"] = serde_json::json!(image);
        }
        if !extras.actions.is_empty() {
            payload["actions"] = serde_json::json!(extras.actions);
        }
        payload
    }

    fn deep_link(data: &NotificationData) -> String {
        match data {
            NotificationData::Agent { chat_id, .. } => format!("/chat?id={}", chat_id),
            NotificationData::App { app_handle, .. } => format!("/apps/{}", app_handle),
            NotificationData::Task { task_id } => format!("/chat?task={}", task_id),
            _ => "/".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_host_strips_the_secret_path() {
        assert_eq!(
            PushSender::endpoint_host("https://fcm.googleapis.com/fcm/send/abc123"),
            "fcm.googleapis.com"
        );
        assert_eq!(
            PushSender::endpoint_host("https://updates.push.services.mozilla.com/wpush/v2/xyz"),
            "updates.push.services.mozilla.com"
        );
        assert_eq!(PushSender::endpoint_host("not a url"), "push service");
    }

    fn notification() -> crate::notification::models::Notification {
        crate::notification::models::Notification {
            id: "n1".into(),
            user_id: "u1".into(),
            data: NotificationData::Agent {
                agent_id: "a1".into(),
                chat_id: "c1".into(),
            },
            level: crate::notification::models::NotificationLevel::Warning,
            title: "Agent needs your input".into(),
            body: "Courier needs a signature".into(),
            read: false,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_plain_push_has_no_image_or_actions() {
        let payload = PushSender::payload(&notification(), &PushExtras::default());
        assert!(payload.get("image").is_none());
        assert!(payload.get("actions").is_none());
        assert_eq!(payload["url"], "/chat?id=c1");
    }

    #[test]
    fn extras_ride_along_in_the_payload() {
        let extras = PushExtras {
            image: Some("https://frona.example/api/files/x?presign=t".into()),
            actions: vec![crate::notification::models::PushAction {
                action: "choice-0".into(),
                title: "I'm coming".into(),
                token: "tok".into(),
            }],
        };
        let payload = PushSender::payload(&notification(), &extras);
        assert_eq!(
            payload["image"],
            "https://frona.example/api/files/x?presign=t"
        );
        assert_eq!(payload["actions"][0]["action"], "choice-0");
        assert_eq!(payload["actions"][0]["title"], "I'm coming");
        assert_eq!(payload["actions"][0]["token"], "tok");
    }
}
