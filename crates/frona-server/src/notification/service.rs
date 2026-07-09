use std::sync::Arc;

use crate::core::error::AppError;
use crate::db::repo::notifications::SurrealNotificationRepo;

use super::models::{Notification, NotificationData, NotificationLevel};
use super::push_sender::PushSender;
use super::repository::NotificationRepository;
use crate::chat::broadcast::BroadcastService;

#[derive(Clone)]
pub struct NotificationService {
    repo: SurrealNotificationRepo,
    broadcast_service: BroadcastService,
    push_sender: Option<Arc<PushSender>>,
}

impl NotificationService {
    pub fn new(repo: SurrealNotificationRepo) -> Self {
        Self {
            repo,
            broadcast_service: BroadcastService::new(),
            push_sender: None,
        }
    }

    pub fn with_broadcast(
        repo: SurrealNotificationRepo,
        broadcast_service: BroadcastService,
        push_sender: Option<Arc<PushSender>>,
    ) -> Self {
        Self {
            repo,
            broadcast_service,
            push_sender,
        }
    }

    /// Create a notification, broadcast it to SSE clients, and fire-and-forget
    /// a Web Push to all of the user's subscribed devices.
    pub async fn create_and_notify(
        &self,
        user_id: &str,
        data: NotificationData,
        level: NotificationLevel,
        title: String,
        body: String,
    ) -> Result<Notification, AppError> {
        let notification = self
            .create(user_id, data, level, title, body)
            .await?;
        self.broadcast_service
            .send_notification(user_id, notification.clone());
        if let Some(sender) = &self.push_sender {
            let sender = Arc::clone(sender);
            let user_id = user_id.to_string();
            let notif = notification.clone();
            tokio::spawn(async move {
                sender.send_to_user(&user_id, &notif).await;
            });
        }
        Ok(notification)
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
