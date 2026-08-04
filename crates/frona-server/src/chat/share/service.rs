use chrono::Utc;

use crate::auth::UserService;
use crate::core::Handle;
use crate::core::error::AppError;
use crate::core::repository::Repository;
use crate::db::repo::chat_shares::SurrealChatShareRepo;

use super::models::ChatShare;
use super::repository::ChatShareRepository;

/// Grants that let one user view another user's chat, read-only.
///
/// Ownership of the chat is enforced by the caller (routes verify via
/// `ChatService::get_chat`, which returns `Forbidden` for non-owners); this
/// service owns the share rows themselves and recipient resolution.
#[derive(Clone)]
pub struct ChatShareService {
    repo: SurrealChatShareRepo,
    user_service: UserService,
}

impl ChatShareService {
    pub fn new(repo: SurrealChatShareRepo, user_service: UserService) -> Self {
        Self { repo, user_service }
    }

    /// Resolve a recipient reference (username/handle first, then email) to a
    /// user id. Returns `NotFound` when no user matches.
    async fn resolve_recipient(&self, recipient_ref: &str) -> Result<String, AppError> {
        let trimmed = recipient_ref.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation("Recipient is required".into()));
        }

        if let Ok(handle) = Handle::try_new(trimmed)
            && let Some(user) = self.user_service.find_by_handle(&handle).await?
        {
            return Ok(user.id);
        }

        if trimmed.contains('@')
            && let Some(user) = self.user_service.find_by_email(trimmed).await?
        {
            return Ok(user.id);
        }

        Err(AppError::NotFound(format!("No user matching '{trimmed}'")))
    }

    /// Share `chat_id` (owned by `owner_id`) with the user identified by
    /// `recipient_ref`. Idempotent: re-sharing the same chat with the same
    /// recipient returns the existing grant. The caller MUST have already
    /// verified that `owner_id` owns `chat_id`.
    pub async fn share(
        &self,
        owner_id: &str,
        chat_id: &str,
        recipient_ref: &str,
    ) -> Result<ChatShare, AppError> {
        let recipient_id = self.resolve_recipient(recipient_ref).await?;
        if recipient_id == owner_id {
            return Err(AppError::Validation(
                "You can't share a chat with yourself".into(),
            ));
        }

        if let Some(existing) = self.repo.find_one(chat_id, &recipient_id).await? {
            return Ok(existing);
        }

        let share = ChatShare {
            id: crate::core::repository::new_id(),
            chat_id: chat_id.to_string(),
            owner_id: owner_id.to_string(),
            recipient_id,
            created_at: Utc::now(),
        };
        self.repo.create(&share).await
    }

    /// Revoke a recipient's access. No-op if the share doesn't exist. The
    /// caller MUST have verified ownership.
    pub async fn unshare(&self, chat_id: &str, recipient_id: &str) -> Result<(), AppError> {
        self.repo.delete_one(chat_id, recipient_id).await
    }

    /// Who a chat is shared with (owner-facing).
    pub async fn list_for_chat(&self, chat_id: &str) -> Result<Vec<ChatShare>, AppError> {
        self.repo.find_by_chat(chat_id).await
    }

    /// Chats shared with a recipient (recipient-facing).
    pub async fn list_shared_with(&self, recipient_id: &str) -> Result<Vec<ChatShare>, AppError> {
        self.repo.find_by_recipient(recipient_id).await
    }

    /// Whether `recipient_id` has been granted read access to `chat_id`. Hot
    /// path for access-control checks.
    pub async fn is_shared_with(
        &self,
        chat_id: &str,
        recipient_id: &str,
    ) -> Result<bool, AppError> {
        Ok(self.repo.find_one(chat_id, recipient_id).await?.is_some())
    }
}
