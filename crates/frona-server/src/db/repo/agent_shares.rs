use async_trait::async_trait;

use crate::agent::share::models::AgentShare;
use crate::agent::share::repository::AgentShareRepository;
use crate::core::error::AppError;

use super::generic::SurrealRepo;

pub type SurrealAgentShareRepo = SurrealRepo<AgentShare>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl AgentShareRepository for SurrealRepo<AgentShare> {
    async fn find_one(
        &self,
        agent_id: &str,
        recipient_id: &str,
    ) -> Result<Option<AgentShare>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM agent_share \
             WHERE agent_id = $agent_id AND recipient_id = $recipient_id LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("agent_id", agent_id.to_string()))
            .bind(("recipient_id", recipient_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let row: Option<AgentShare> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(row)
    }

    async fn find_by_agent(&self, agent_id: &str) -> Result<Vec<AgentShare>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM agent_share WHERE agent_id = $agent_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("agent_id", agent_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows: Vec<AgentShare> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(rows)
    }

    async fn find_by_recipient(
        &self,
        recipient_id: &str,
    ) -> Result<Vec<AgentShare>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM agent_share WHERE recipient_id = $recipient_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("recipient_id", recipient_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows: Vec<AgentShare> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(rows)
    }

    async fn delete_one(&self, agent_id: &str, recipient_id: &str) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM agent_share WHERE agent_id = $agent_id AND recipient_id = $recipient_id")
            .bind(("agent_id", agent_id.to_string()))
            .bind(("recipient_id", recipient_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
