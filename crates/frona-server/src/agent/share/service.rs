use chrono::Utc;

use crate::auth::UserService;
use crate::core::Handle;
use crate::core::error::AppError;
use crate::core::repository::Repository;
use crate::db::repo::agent_shares::SurrealAgentShareRepo;

use super::models::{AgentShare, ShareLevel};
use super::repository::AgentShareRepository;

/// Grants that let one user use another user's agent (use-only).
///
/// Ownership of the agent is enforced by the caller (routes verify via
/// `AgentService::get`, which returns `Forbidden` for non-owners); this service
/// owns the share rows themselves and recipient resolution.
#[derive(Clone)]
pub struct AgentShareService {
    repo: SurrealAgentShareRepo,
    user_service: UserService,
}

impl AgentShareService {
    pub fn new(repo: SurrealAgentShareRepo, user_service: UserService) -> Self {
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

    /// Share `agent_id` (owned by `owner_id`) with the user identified by
    /// `recipient_ref`. Idempotent: re-sharing the same agent with the same
    /// recipient returns the existing grant. The caller MUST have already
    /// verified that `owner_id` owns `agent_id`.
    ///
    /// Returns the share plus the resolved recipient id.
    pub async fn share(
        &self,
        owner_id: &str,
        agent_id: &str,
        recipient_ref: &str,
    ) -> Result<AgentShare, AppError> {
        let recipient_id = self.resolve_recipient(recipient_ref).await?;
        if recipient_id == owner_id {
            return Err(AppError::Validation(
                "You can't share an agent with yourself".into(),
            ));
        }

        if let Some(existing) = self.repo.find_one(agent_id, &recipient_id).await? {
            return Ok(existing);
        }

        let now = Utc::now();
        let share = AgentShare {
            id: crate::core::repository::new_id(),
            agent_id: agent_id.to_string(),
            owner_id: owner_id.to_string(),
            recipient_id,
            level: ShareLevel::Use,
            delegate_credentials: false,
            created_at: now,
            updated_at: now,
        };
        self.repo.create(&share).await
    }

    /// Toggle credential delegation for an existing share. Returns `NotFound`
    /// if the recipient isn't currently shared with. The caller MUST have
    /// verified ownership.
    pub async fn set_delegation(
        &self,
        agent_id: &str,
        recipient_id: &str,
        delegate_credentials: bool,
    ) -> Result<AgentShare, AppError> {
        let mut share = self
            .repo
            .find_one(agent_id, recipient_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Share not found".into()))?;
        share.delegate_credentials = delegate_credentials;
        share.updated_at = Utc::now();
        self.repo.update(&share).await
    }

    /// The full share row for `(agent_id, recipient_id)`, if any. Used at
    /// runtime to read the delegation flag.
    pub async fn find_share(
        &self,
        agent_id: &str,
        recipient_id: &str,
    ) -> Result<Option<AgentShare>, AppError> {
        self.repo.find_one(agent_id, recipient_id).await
    }

    /// Revoke a recipient's access. No-op if the share doesn't exist. The
    /// caller MUST have verified ownership.
    pub async fn unshare(
        &self,
        agent_id: &str,
        recipient_id: &str,
    ) -> Result<(), AppError> {
        self.repo.delete_one(agent_id, recipient_id).await
    }

    /// Who an agent is shared with (owner-facing).
    pub async fn list_for_agent(&self, agent_id: &str) -> Result<Vec<AgentShare>, AppError> {
        self.repo.find_by_agent(agent_id).await
    }

    /// Agents shared with a recipient (recipient-facing).
    pub async fn list_shared_with(
        &self,
        recipient_id: &str,
    ) -> Result<Vec<AgentShare>, AppError> {
        self.repo.find_by_recipient(recipient_id).await
    }

    /// The access level `recipient_id` has on `agent_id`, if any. Hot path for
    /// access-control checks.
    pub async fn find_level(
        &self,
        agent_id: &str,
        recipient_id: &str,
    ) -> Result<Option<ShareLevel>, AppError> {
        Ok(self
            .repo
            .find_one(agent_id, recipient_id)
            .await?
            .map(|s| s.level))
    }
}
