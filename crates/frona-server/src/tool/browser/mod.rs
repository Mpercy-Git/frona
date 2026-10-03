pub mod session;
pub mod tool;

use crate::credential::vault::models::CredentialResponse;
use crate::credential::vault::service::VaultService;

/// The credential whose browser profile the agent drives: the user's most
/// recently created credential (`list_credentials` is ordered newest first).
/// The browser tools and the takeover handover both resolve the profile
/// through this, so a handover always opens the profile the agent was using.
pub(crate) async fn active_profile_credential(
    vault_service: &VaultService,
    user_id: &str,
) -> Option<CredentialResponse> {
    vault_service
        .list_credentials(user_id)
        .await
        .ok()
        .and_then(|creds| creds.into_iter().next())
}
