use async_trait::async_trait;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Chat;

#[async_trait]
pub trait ChatRepository: Repository<Chat> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<Chat>, AppError>;
    async fn find_by_space_id(&self, space_id: &str) -> Result<Vec<Chat>, AppError>;
    /// Excludes task-execution chats (`task_id IS NOT NONE`). Use for user-facing
    /// listings; use `find_by_space_id` when you need every row.
    async fn find_user_chats_by_space_id(&self, space_id: &str) -> Result<Vec<Chat>, AppError>;
    async fn find_standalone_by_user_id(&self, user_id: &str) -> Result<Vec<Chat>, AppError>;
    async fn find_archived_by_user_id(&self, user_id: &str) -> Result<Vec<Chat>, AppError>;
    async fn find_by_channel_thread(
        &self,
        channel_id: &str,
        channel_external_id: &str,
    ) -> Result<Option<Chat>, AppError>;
}
