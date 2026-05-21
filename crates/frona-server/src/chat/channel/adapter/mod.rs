pub mod discord;
pub mod markdown;
pub mod signal;
pub mod slack;
pub mod sms;
pub mod storage;
pub mod telegram;
pub mod whatsapp_cloud;
pub mod whatsapp_user;

use sha2::{Digest, Sha256};

use super::models::ChannelCtx;

/// Stable per-space suffix so multiple channels under the same user are
/// distinguishable in linked-device listings.
fn space_id_suffix(space_id: &str) -> String {
    let d = Sha256::digest(space_id.as_bytes());
    format!("{:02x}{:02x}", d[0], d[1])
}

/// `"{username}-{space_suffix}"`, falling back to `"frona-{suffix}"` if the
/// user lookup fails.
pub async fn resolve_device_label(ctx: &ChannelCtx) -> String {
    let username = ctx
        .user_service
        .find_by_id(&ctx.channel.user_id)
        .await
        .ok()
        .flatten()
        .map(|u| u.username)
        .unwrap_or_else(|| "frona".to_string());
    format!("{username}-{}", space_id_suffix(&ctx.channel.space_id))
}
