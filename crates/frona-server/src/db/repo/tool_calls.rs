use async_trait::async_trait;
use crate::core::error::AppError;
use crate::core::repository::Repository;
use crate::inference::tool_call::ToolCall;

use super::generic::SurrealRepo;

pub type SurrealToolCallRepo = SurrealRepo<ToolCall>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
pub trait ToolCallRepository: Repository<ToolCall> {
    async fn find_by_chat_id(&self, chat_id: &str) -> Result<Vec<ToolCall>, AppError>;
    async fn find_by_message_id(&self, message_id: &str) -> Result<Vec<ToolCall>, AppError>;
    async fn find_by_message_ids(&self, message_ids: &[String]) -> Result<Vec<ToolCall>, AppError>;
    async fn find_pending_by_chat_id(&self, chat_id: &str) -> Result<Option<ToolCall>, AppError>;
}

#[async_trait]
impl ToolCallRepository for SurrealRepo<ToolCall> {
    async fn find_by_chat_id(&self, chat_id: &str) -> Result<Vec<ToolCall>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM tool_call WHERE chat_id = $chat_id ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("chat_id", chat_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let executions: Vec<ToolCall> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(executions)
    }

    async fn find_by_message_id(&self, message_id: &str) -> Result<Vec<ToolCall>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM tool_call WHERE message_id = $message_id ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("message_id", message_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let executions: Vec<ToolCall> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(executions)
    }

    async fn find_by_message_ids(&self, message_ids: &[String]) -> Result<Vec<ToolCall>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM tool_call WHERE message_id IN $message_ids ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("message_ids", message_ids.to_vec()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let executions: Vec<ToolCall> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(executions)
    }

    async fn find_pending_by_chat_id(&self, chat_id: &str) -> Result<Option<ToolCall>, AppError> {
        let all = self.find_by_chat_id(chat_id).await?;
        Ok(all
            .into_iter()
            .rev()
            .find(|te| {
                te.hitl
                    .as_ref()
                    .is_some_and(|h| h.status == crate::inference::tool_call::ToolStatus::Pending)
            }))
    }
}
