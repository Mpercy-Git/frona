use async_trait::async_trait;

use super::models::PasswordResetToken;
use crate::core::error::AppError;
use crate::core::repository::Repository;

#[async_trait]
pub trait PasswordResetRepository: Repository<PasswordResetToken> {
    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<PasswordResetToken>, AppError>;
    async fn delete_by_user_id(&self, user_id: &str) -> Result<(), AppError>;
    async fn delete_expired(&self) -> Result<(), AppError>;
}
