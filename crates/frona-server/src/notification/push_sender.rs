use std::sync::Arc;

use serde::Serialize;
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, Urgency, VapidSignatureBuilder,
    WebPushClient, WebPushError, WebPushMessageBuilder,
};

use crate::core::config::PushConfig;
use crate::core::error::AppError;
use crate::notification::models::NotificationData;
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

pub struct PushSender {
    client: IsahcWebPushClient,
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

        let client = IsahcWebPushClient::new()
            .map_err(|e| AppError::Internal(format!("Failed to create Web Push client: {e}")))?;

        Ok(Some(Self {
            client,
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
    ) {
        let report = self.deliver_to_user(user_id, notification).await;
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

        let payload = serde_json::json!({
            "id": notification.id,
            "title": notification.title,
            "body": notification.body,
            "level": format!("{:?}", notification.level).to_lowercase(),
            "data": notification.data,
            "url": Self::deep_link(&notification.data),
        });

        let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();
        let ttl = 86400u32; // 24 hours

        for sub in subs {
            let service = Self::endpoint_host(&sub.endpoint);
            let subscription_info = SubscriptionInfo::new(
                &sub.endpoint,
                &sub.p256dh_key,
                &sub.auth_secret,
            );

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
                        reason: e.to_string(),
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
    fn deep_link(data: &NotificationData) -> String {
        match data {
            NotificationData::Agent { chat_id, .. } => format!("/chat?id={}", chat_id),
            NotificationData::App { app_handle, .. } => format!("/apps/{}", app_handle),
            NotificationData::Task { task_id } => format!("/?task={}", task_id),
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
}
