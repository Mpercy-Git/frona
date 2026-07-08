use async_trait::async_trait;

use crate::core::error::AppError;
use crate::notification::models::Notification;
use crate::notification::repository::NotificationRepository;

use super::generic::SurrealRepo;

pub type SurrealNotificationRepo = SurrealRepo<Notification>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl NotificationRepository for SurrealRepo<Notification> {
    async fn find_by_user_id(&self, user_id: &str, limit: u32) -> Result<Vec<Notification>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM notification WHERE user_id = $user_id ORDER BY created_at DESC LIMIT $limit"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .bind(("limit", limit))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let notifications: Vec<Notification> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(notifications)
    }

    async fn find_unread_by_user_id(&self, user_id: &str) -> Result<Vec<Notification>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM notification WHERE user_id = $user_id AND read = false ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let notifications: Vec<Notification> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(notifications)
    }

    async fn count_unread(&self, user_id: &str) -> Result<u64, AppError> {
        let mut result = self
            .db()
            .query("SELECT count() as count FROM notification WHERE user_id = $user_id AND read = false GROUP ALL")
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let row: Option<serde_json::Value> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(row
            .and_then(|v| v.get("count").and_then(|c| c.as_u64()))
            .unwrap_or(0))
    }

    async fn mark_read(&self, user_id: &str, id: &str) -> Result<(), AppError> {
        self.db()
            .query("UPDATE type::record('notification', $id) SET read = true WHERE user_id = $user_id")
            .bind(("id", id.to_string()))
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn mark_all_read(&self, user_id: &str) -> Result<(), AppError> {
        self.db()
            .query("UPDATE notification SET read = true WHERE user_id = $user_id AND read = false")
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}
