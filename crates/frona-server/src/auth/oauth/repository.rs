use async_trait::async_trait;

use super::models::OAuthIdentity;
use crate::core::error::AppError;
use crate::core::repository::Repository;

#[async_trait]
pub trait OAuthRepository: Repository<OAuthIdentity> {
    async fn find_identity_by_sub(
        &self,
        external_sub: &str,
    ) -> Result<Option<OAuthIdentity>, AppError>;
    /// The identity for `external_sub` as issued by `issuer`. A legacy row
    /// with no recorded issuer also matches, so existing links keep working;
    /// an exact match always wins over one.
    async fn find_identity(
        &self,
        issuer: &str,
        external_sub: &str,
    ) -> Result<Option<OAuthIdentity>, AppError>;
    async fn find_identities_by_user(&self, user_id: &str) -> Result<Vec<OAuthIdentity>, AppError>;
}
