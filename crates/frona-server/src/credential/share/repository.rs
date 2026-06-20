use async_trait::async_trait;

use super::models::Share;
use crate::core::error::AppError;
use crate::core::repository::Repository;

#[async_trait]
pub trait ShareRepository: Repository<Share> {
    /// Returns the row only if not yet expired. Used by the resolve route.
    async fn find_active_by_id(&self, id: &str) -> Result<Option<Share>, AppError>;
    /// Reused by `lookup_or_issue_chat` so one chat = one stable `/s/{id}`
    /// short link across SMS overflow events.
    async fn find_active_chat_share(
        &self,
        user_id: &str,
        chat_id: &str,
    ) -> Result<Option<Share>, AppError>;
    /// Deletes all rows whose `expires_at <= now`. Returns count of deleted.
    async fn delete_expired(&self) -> Result<u64, AppError>;
}
