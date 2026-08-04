use async_trait::async_trait;

use super::models::ChatShare;
use crate::core::error::AppError;
use crate::core::repository::Repository;

#[async_trait]
pub trait ChatShareRepository: Repository<ChatShare> {
    /// The single share row for `(chat_id, recipient_id)`, if any.
    async fn find_one(
        &self,
        chat_id: &str,
        recipient_id: &str,
    ) -> Result<Option<ChatShare>, AppError>;

    /// Every share of a given chat (who it's shared with).
    async fn find_by_chat(&self, chat_id: &str) -> Result<Vec<ChatShare>, AppError>;

    /// Every share granted to a recipient (chats shared with me).
    async fn find_by_recipient(&self, recipient_id: &str) -> Result<Vec<ChatShare>, AppError>;

    /// Remove the share for `(chat_id, recipient_id)`. No-op if absent.
    async fn delete_one(&self, chat_id: &str, recipient_id: &str) -> Result<(), AppError>;
}
