use async_trait::async_trait;

use super::models::PasswordResetToken;
use crate::core::error::AppError;
use crate::core::repository::Repository;

#[async_trait]
pub trait PasswordResetRepository: Repository<PasswordResetToken> {
    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<PasswordResetToken>, AppError>;
    /// Deletes the unexpired token with this hash, in one statement, and
    /// returns the id of the user it was issued to. Of any number of
    /// concurrent callers, at most one gets `Some`.
    async fn take_by_hash(
        &self,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<String>, AppError>;
    /// When the user's most recent outstanding token was issued.
    async fn latest_created_at(
        &self,
        user_id: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError>;
    async fn delete_by_user_id(&self, user_id: &str) -> Result<(), AppError>;
    async fn delete_expired(&self) -> Result<(), AppError>;
}
