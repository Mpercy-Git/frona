use std::sync::Arc;

use crate::core::error::AppError;
use crate::db::repo::notifications::SurrealNotificationRepo;

use super::models::{
    Notification, NotificationCategory, NotificationData, NotificationLevel,
    NotificationPreferences, PushAction, PushChoices, PushExtras,
};
use super::push_sender::PushSender;
use super::repository::NotificationRepository;
use crate::chat::broadcast::BroadcastService;
use crate::chat::models::{Chat, PUSH_IMAGE_METADATA_KEY};
use crate::core::repository::Repository;
use crate::credential::presign::PresignService;
use crate::db::repo::generic::SurrealRepo;

#[derive(Clone)]
pub struct NotificationService {
    repo: SurrealNotificationRepo,
    preferences_repo: SurrealRepo<NotificationPreferences>,
    broadcast_service: BroadcastService,
    push_sender: Option<Arc<PushSender>>,
    push_links: Option<PushLinks>,
}

/// What a push needs to carry a picture and answer buttons: signed URLs for
/// the image and signed tokens for the buttons.
#[derive(Clone)]
struct PushLinks {
    presign: PresignService,
    chats: SurrealRepo<Chat>,
}

/// How long a push's image link and answer buttons stay usable. Matches the
/// push's own 24-hour TTL, after which the device drops an undelivered push.
const PUSH_LINK_EXPIRY_SECS: u64 = 86_400;
/// Most platforms show two or three buttons; the service worker trims to what
/// the device supports.
const MAX_PUSH_ACTIONS: usize = 3;
const MAX_ACTION_TITLE_CHARS: usize = 40;

impl NotificationService {
    pub fn new(repo: SurrealNotificationRepo) -> Self {
        Self {
            preferences_repo: SurrealRepo::new(repo.db().clone()),
            repo,
            broadcast_service: BroadcastService::new(),
            push_sender: None,
            push_links: None,
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
            push_links: None,
        }
    }

    /// Let pushes carry a chat's image and answer buttons. Without it, pushes
    /// are title and body only, as before.
    pub fn with_push_links(mut self, presign: PresignService) -> Self {
        self.push_links = Some(PushLinks {
            presign,
            chats: SurrealRepo::new(self.repo.db().clone()),
        });
        self
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
        self.create_and_notify_with_choices(user_id, category, data, level, title, body, None)
            .await
    }

    /// [`Self::create_and_notify`] for a pending question: the push also offers
    /// the question's options as buttons that answer it without opening the app.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_and_notify_with_choices(
        &self,
        user_id: &str,
        category: NotificationCategory,
        data: NotificationData,
        level: NotificationLevel,
        title: String,
        body: String,
        choices: Option<PushChoices>,
    ) -> Result<Notification, AppError> {
        let notification = self.create(user_id, data, level, title, body).await?;
        self.broadcast_service
            .send_notification(user_id, notification.clone());
        if let Some(sender) = &self.push_sender
            && self.preferences(user_id).await.pushes(category)
        {
            let sender = Arc::clone(sender);
            let links = self.push_links.clone();
            let user_id = user_id.to_string();
            let notif = notification.clone();
            tokio::spawn(async move {
                let extras = match &links {
                    Some(links) => links.extras(&user_id, &notif, choices.as_ref()).await,
                    None => PushExtras::default(),
                };
                sender.send_to_user(&user_id, &notif, &extras).await;
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

impl PushLinks {
    /// Best effort: a link that can't be signed is left out and the push goes
    /// out without it.
    async fn extras(
        &self,
        user_id: &str,
        notification: &Notification,
        choices: Option<&PushChoices>,
    ) -> PushExtras {
        let mut extras = PushExtras::default();

        if let NotificationData::Agent { chat_id, .. } = &notification.data {
            extras.image = self.chat_image(user_id, chat_id).await;
        }

        if let Some(choices) = choices {
            for (i, option) in choices.options.iter().take(MAX_PUSH_ACTIONS).enumerate() {
                match self
                    .presign
                    .sign_push_action(
                        user_id,
                        &choices.chat_id,
                        &choices.tool_call_id,
                        option,
                        PUSH_LINK_EXPIRY_SECS,
                    )
                    .await
                {
                    Ok(token) => extras.actions.push(PushAction {
                        action: format!("choice-{i}"),
                        title: action_title(option),
                        token,
                    }),
                    Err(e) => {
                        tracing::warn!(error = %e, "Could not sign a notification answer button");
                    }
                }
            }
        }
        extras
    }

    async fn chat_image(&self, user_id: &str, chat_id: &str) -> Option<String> {
        let chat = self.chats.find_by_id(chat_id).await.ok().flatten()?;
        if chat.user_id != user_id {
            return None;
        }
        let image = chat.metadata.get(PUSH_IMAGE_METADATA_KEY)?;
        let owner = image.get("owner")?.as_str()?;
        let path = image.get("path")?.as_str()?;
        // Only ever the recipient's own file.
        if owner != format!("user:{user_id}") {
            return None;
        }
        let url = self
            .presign
            .sign_with_expiry_by_user_id(owner, path, user_id, PUSH_LINK_EXPIRY_SECS)
            .await
            .ok()
            .filter(|url| !url.is_empty())?;
        // Presigned URLs carry the server's local base, which a phone can't
        // reach. Send the path and let the service worker resolve it against
        // the origin the app was opened on.
        let url = url::Url::parse(&url).ok()?;
        Some(match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        })
    }
}

fn action_title(option: &str) -> String {
    let option = option.trim();
    if option.chars().count() <= MAX_ACTION_TITLE_CHARS {
        return option.to_string();
    }
    let cut: String = option.chars().take(MAX_ACTION_TITLE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
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

    #[test]
    fn long_options_are_shortened_for_a_button() {
        assert_eq!(action_title("  I'm coming  "), "I'm coming");
        let long = "Please ask them to leave it with the neighbour at number 12";
        let title = action_title(long);
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= MAX_ACTION_TITLE_CHARS);
    }

    async fn links_with_user() -> (PushLinks, crate::auth::User) {
        let db = surrealdb::Surreal::new::<surrealdb::engine::local::Mem>(())
            .await
            .unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        let now = chrono::Utc::now();
        let user = crate::auth::User {
            id: crate::core::repository::new_id(),
            handle: crate::handle!("owner"),
            email: "owner@example.com".into(),
            name: "Owner".into(),
            password_hash: String::new(),
            timezone: None,
            phone: None,
            groups: Vec::new(),
            deactivated_at: None,
            created_at: now,
            updated_at: now,
        };
        SurrealRepo::<crate::auth::User>::new(db.clone())
            .create(&user)
            .await
            .unwrap();
        let keypairs = crate::credential::keypair::service::KeyPairService::new(
            "test-secret",
            Arc::new(SurrealRepo::new(db.clone())),
        );
        let users = crate::auth::UserService::new(
            SurrealRepo::new(db.clone()),
            &crate::core::config::CacheConfig::default(),
        );
        let presign = PresignService::new(keypairs, users, "https://frona.example".into(), 3600);
        let links = PushLinks {
            presign,
            chats: SurrealRepo::new(db),
        };
        (links, user)
    }

    fn chat(user_id: &str, image_owner: &str) -> Chat {
        let now = chrono::Utc::now();
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert(
            PUSH_IMAGE_METADATA_KEY.to_string(),
            serde_json::json!({ "owner": image_owner, "path": "triggers/front-door/ring-0.jpg" }),
        );
        Chat {
            id: crate::core::repository::new_id(),
            user_id: user_id.to_string(),
            space_id: None,
            task_id: None,
            agent_id: "front-door".into(),
            title: Some("Doorbell".into()),
            archived_at: None,
            channel_id: None,
            channel_external_id: None,
            metadata,
            created_at: now,
            updated_at: now,
        }
    }

    fn agent_notification(user_id: &str, chat_id: &str) -> Notification {
        Notification {
            id: "n1".into(),
            user_id: user_id.into(),
            data: NotificationData::Agent {
                agent_id: "front-door".into(),
                chat_id: chat_id.into(),
            },
            level: NotificationLevel::Warning,
            title: "Agent needs your input".into(),
            body: "Courier needs a signature".into(),
            read: false,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn a_question_gets_one_signed_button_per_option() {
        let (links, user) = links_with_user().await;
        let choices = PushChoices {
            chat_id: "c1".into(),
            tool_call_id: "tc1".into(),
            options: vec![
                "I'm coming".into(),
                "Leave it in the porch".into(),
                "Ask them to wait".into(),
                "Ignore".into(),
            ],
        };
        let extras = links
            .extras(
                &user.id,
                &agent_notification(&user.id, "c1"),
                Some(&choices),
            )
            .await;

        assert_eq!(extras.actions.len(), MAX_PUSH_ACTIONS);
        for (i, action) in extras.actions.iter().enumerate() {
            let claims = links
                .presign
                .verify_push_action(&action.token)
                .await
                .unwrap();
            assert_eq!(claims.sub, user.id);
            assert_eq!(claims.chat_id, "c1");
            assert_eq!(claims.tool_call_id, "tc1");
            assert_eq!(claims.choice, choices.options[i]);
        }
        // An answer token is not a file presign, and a file presign is not an
        // answer token.
        assert!(
            links
                .presign
                .verify(&extras.actions[0].token)
                .await
                .is_err()
        );
        let file_token = links
            .presign
            .sign_scoped_token("user:x", "a.jpg", &user.id, 60)
            .await
            .unwrap();
        assert!(links.presign.verify_push_action(&file_token).await.is_err());
    }

    #[tokio::test]
    async fn a_trigger_chat_lends_its_snapshot_to_the_push() {
        let (links, user) = links_with_user().await;
        let own = links
            .chats
            .create(&chat(&user.id, &format!("user:{}", user.id)))
            .await
            .unwrap();
        let extras = links
            .extras(&user.id, &agent_notification(&user.id, &own.id), None)
            .await;
        let image = extras.image.expect("the chat's image is attached");
        assert!(image.starts_with("/api/files/"), "{image}");
        assert!(image.contains("presign="), "{image}");
        assert!(extras.actions.is_empty());

        // Never someone else's file, and never another user's chat.
        let foreign = links
            .chats
            .create(&chat(&user.id, "user:someone-else"))
            .await
            .unwrap();
        assert!(
            links
                .extras(&user.id, &agent_notification(&user.id, &foreign.id), None)
                .await
                .image
                .is_none()
        );
        assert!(
            links
                .extras(
                    "another-user",
                    &agent_notification("another-user", &own.id),
                    None
                )
                .await
                .image
                .is_none()
        );
    }
}
