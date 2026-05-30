use crate::core::error::AppError;
use crate::db::repo::notifications::SurrealNotificationRepo;

use super::models::{Notification, NotificationData, NotificationLevel};
use super::repository::NotificationRepository;

#[derive(Clone)]
pub struct NotificationService {
    repo: SurrealNotificationRepo,
}

impl NotificationService {
    pub fn new(repo: SurrealNotificationRepo) -> Self {
        Self { repo }
    }

    pub async fn create(
        &self,
        user_id: &str,
        data: NotificationData,
        level: NotificationLevel,
        title: String,
        body: String,
    ) -> Result<Notification, AppError> {
        let notification = Notification {
            id: crate::core::repository::new_id(),
            user_id: user_id.to_string(),
            data,
            level,
            title,
            body,
            read: false,
            created_at: chrono::Utc::now(),
        };

        use crate::core::repository::Repository;
        self.repo.create(&notification).await
    }

    pub async fn list(&self, user_id: &str, limit: u32) -> Result<Vec<Notification>, AppError> {
        self.repo.find_by_user_id(user_id, limit).await
    }

    pub async fn unread_count(&self, user_id: &str) -> Result<u64, AppError> {
        self.repo.count_unread(user_id).await
    }

    pub async fn mark_read(&self, user_id: &str, id: &str) -> Result<(), AppError> {
        self.repo.mark_read(user_id, id).await
    }

    pub async fn mark_all_read(&self, user_id: &str) -> Result<(), AppError> {
        self.repo.mark_all_read(user_id).await
    }
}
