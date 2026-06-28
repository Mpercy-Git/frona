use std::sync::Arc;

use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder,
    WebPushClient, WebPushError, WebPushMessageBuilder,
};

use crate::core::config::PushConfig;
use crate::core::error::AppError;
use crate::notification::models::NotificationData;
use crate::notification::push_repository::PushSubscriptionRepository;

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
            Some(k) => k.clone(),
            None => return Ok(None),
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
    pub async fn send_to_user(&self, user_id: &str, notification: &crate::notification::models::Notification) {
        let subs = match self.repo.find_by_user_id(user_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, user_id, "Failed to fetch push subscriptions");
                return;
            }
        };

        if subs.is_empty() {
            return;
        }

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
            let subscription_info = SubscriptionInfo::new(
                &sub.endpoint,
                &sub.p256dh_key,
                &sub.auth_secret,
            );

            // Build VAPID signature (needs per-subscription info).
            let sig = match VapidSignatureBuilder::from_base64_no_sub(&self.vapid_private_key) {
                Ok(builder) => builder.add_sub_info(&subscription_info).build(),
                Err(e) => {
                    tracing::warn!(error = %e, "VAPID signature build failed");
                    continue;
                }
            };

            let sig = match sig {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "VAPID signature build failed");
                    continue;
                }
            };

            let mut builder = WebPushMessageBuilder::new(&subscription_info);
            builder.set_payload(ContentEncoding::Aes128Gcm, &payload_bytes);
            builder.set_ttl(ttl);
            builder.set_vapid_signature(sig);

            let message = match builder.build() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, endpoint = %sub.endpoint, "Push message build failed");
                    continue;
                }
            };

            match self.client.send(message).await {
                Ok(_) => {}
                Err(WebPushError::EndpointNotValid(_)) | Err(WebPushError::EndpointNotFound(_)) => {
                    tracing::info!(endpoint = %sub.endpoint, "Removing expired push subscription");
                    let _ = self.repo.delete_by_endpoint(user_id, &sub.endpoint).await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, endpoint = %sub.endpoint, "Push send failed");
                }
            }
        }
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