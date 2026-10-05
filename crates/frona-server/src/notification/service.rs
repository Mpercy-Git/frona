use std::sync::Arc;

use crate::core::error::AppError;
use crate::db::repo::notifications::SurrealNotificationRepo;

use super::models::{
    Notification, NotificationCategory, NotificationData, NotificationLevel,
    NotificationPreferences,
};
use super::push_sender::PushSender;
use super::repository::NotificationRepository;
use crate::chat::broadcast::BroadcastService;
use crate::core::repository::Repository;
use crate::db::repo::generic::SurrealRepo;

#[derive(Clone)]
pub struct NotificationService {
    repo: SurrealNotificationRepo,
    preferences_repo: SurrealRepo<NotificationPreferences>,
    broadcast_service: BroadcastService,
    push_sender: Option<Arc<PushSender>>,
}

impl NotificationService {
    pub fn new(repo: SurrealNotificationRepo) -> Self {
        Self {
            preferences_repo: SurrealRepo::new(repo.db().clone()),
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
            preferences_repo: SurrealRepo::new(repo.db().clone()),
            repo,
            broadcast_service,
            push_sender,
        }
    }

    /// Create a notification, broadcast it to SSE clients, and — if the user
    /// wants this `category` pushed — fire-and-forget a Web Push to all of
    /// their subscribed devices.
    pub async fn create_and_notify(
        &self,
        user_id: &str,
        category: NotificationCategory,
        data: NotificationData,
        level: NotificationLevel,
        title: String,
        body: String,
    ) -> Result<Notification, AppError> {
        let notification = self.create(user_id, data, level, title, body).await?;
        self.broadcast_service
            .send_notification(user_id, notification.clone());
        if let Some(sender) = &self.push_sender
            && self.preferences(user_id).await.pushes(category)
        {
            let sender = Arc::clone(sender);
            let user_id = user_id.to_string();
            let notif = notification.clone();
            tokio::spawn(async move {
                sender.send_to_user(&user_id, &notif).await;
            });
        }
        Ok(notification)
    }

    /// The user's push preferences, or the defaults when none are stored or
    /// they cannot be read (a lookup failure must not silence notifications).
    pub async fn preferences(&self, user_id: &str) -> NotificationPreferences {
        match self.preferences_repo.find_by_id(user_id).await {
            Ok(Some(prefs)) => prefs,
            Ok(None) => NotificationPreferences::defaults_for(user_id),
            Err(e) => {
                tracing::warn!(error = %e, user_id, "Could not read notification preferences; using defaults");
                NotificationPreferences::defaults_for(user_id)
            }
        }
    }

    pub async fn set_preferences(
        &self,
        user_id: &str,
        mut prefs: NotificationPreferences,
    ) -> Result<NotificationPreferences, AppError> {
        prefs.id = user_id.to_string();
        prefs.user_id = user_id.to_string();
        if self.preferences_repo.find_by_id(user_id).await?.is_some() {
            self.preferences_repo.update(&prefs).await
        } else {
            self.preferences_repo.create(&prefs).await
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn service() -> NotificationService {
        let db = surrealdb::Surreal::new::<surrealdb::engine::local::Mem>(())
            .await
            .unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        NotificationService::new(SurrealRepo::new(db))
    }

    #[tokio::test]
    async fn unset_preferences_push_everything_but_activity() {
        let prefs = service().await.preferences("u1").await;
        assert!(prefs.pushes(NotificationCategory::Approval));
        assert!(prefs.pushes(NotificationCategory::AgentMessage));
        assert!(prefs.pushes(NotificationCategory::ChatReply));
        assert!(prefs.pushes(NotificationCategory::Failure));
        assert!(!prefs.pushes(NotificationCategory::Activity));
    }

    #[tokio::test]
    async fn saved_preferences_are_per_user_and_can_be_changed() {
        let svc = service().await;

        let mut prefs = NotificationPreferences::defaults_for("ignored");
        prefs.push_chat_reply = false;
        let saved = svc.set_preferences("u1", prefs).await.unwrap();
        assert_eq!(saved.id, "u1");
        assert_eq!(saved.user_id, "u1");
        assert!(
            !svc.preferences("u1")
                .await
                .pushes(NotificationCategory::ChatReply)
        );
        // Another user keeps the defaults.
        assert!(
            svc.preferences("u2")
                .await
                .pushes(NotificationCategory::ChatReply)
        );

        // A second save updates the existing row.
        let mut prefs = svc.preferences("u1").await;
        prefs.push_chat_reply = true;
        prefs.push_activity = true;
        svc.set_preferences("u1", prefs).await.unwrap();
        let reread = svc.preferences("u1").await;
        assert!(reread.pushes(NotificationCategory::ChatReply));
        assert!(reread.pushes(NotificationCategory::Activity));
    }
}
