use async_trait::async_trait;

use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Notification;

#[async_trait]
pub trait NotificationRepository: Repository<Notification> {
    async fn find_by_user_id(&self, user_id: &str, limit: u32) -> Result<Vec<Notification>, AppError>;
    async fn find_unread_by_user_id(&self, user_id: &str) -> Result<Vec<Notification>, AppError>;
    async fn count_unread(&self, user_id: &str) -> Result<u64, AppError>;
    async fn mark_read(&self, user_id: &str, id: &str) -> Result<(), AppError>;
    async fn mark_all_read(&self, user_id: &str) -> Result<(), AppError>;
}
