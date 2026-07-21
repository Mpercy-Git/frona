use async_trait::async_trait;

use super::models::AgentShare;
use crate::core::error::AppError;
use crate::core::repository::Repository;

#[async_trait]
pub trait AgentShareRepository: Repository<AgentShare> {
    /// The single share row for `(agent_id, recipient_id)`, if any.
    async fn find_one(
        &self,
        agent_id: &str,
        recipient_id: &str,
    ) -> Result<Option<AgentShare>, AppError>;

    /// Every share of a given agent (who it's shared with).
    async fn find_by_agent(&self, agent_id: &str) -> Result<Vec<AgentShare>, AppError>;

    /// Every share granted to a recipient (agents shared with me).
    async fn find_by_recipient(&self, recipient_id: &str)
    -> Result<Vec<AgentShare>, AppError>;

    /// Remove the share for `(agent_id, recipient_id)`. No-op if absent.
    async fn delete_one(&self, agent_id: &str, recipient_id: &str) -> Result<(), AppError>;
}
