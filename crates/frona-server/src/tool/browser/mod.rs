pub mod session;
pub mod tool;

use crate::credential::vault::service::VaultService;

/// The profile a user's browser runs under when they have no saved logins.
pub const DEFAULT_PROFILE: &str = "default";

/// The browser profile the agent drives for this user: the provider of their
/// first saved login, else [`DEFAULT_PROFILE`].
///
/// The browser tool and the takeover live view must agree on this, or a person
/// asked to solve a CAPTCHA is shown a different browser from the one the agent
/// is stuck in.
pub async fn active_profile(vault: &VaultService, user_id: &str) -> String {
    vault
        .list_credentials(user_id)
        .await
        .ok()
        .and_then(|creds| creds.into_iter().next())
        .map(|c| c.provider)
        .unwrap_or_else(|| DEFAULT_PROFILE.to_string())
}

/// Whether `profile` is one this user's browser can run under. The name becomes
/// a directory under the profiles volume, so only names the user's own logins
/// produce are accepted.
pub async fn is_user_profile(vault: &VaultService, user_id: &str, profile: &str) -> bool {
    if profile.is_empty() || profile.contains(['/', '\\']) || profile.contains("..") {
        return false;
    }
    if profile == DEFAULT_PROFILE {
        return true;
    }
    vault
        .list_credentials(user_id)
        .await
        .map(|creds| creds.iter().any(|c| c.provider == profile))
        .unwrap_or(false)
}
