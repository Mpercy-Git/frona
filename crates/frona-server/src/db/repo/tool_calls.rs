use async_trait::async_trait;
use surrealdb::types::SurrealValue;

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
    /// Total number of tool invocations for the chat. Uses
    /// `idx_tool_call_chat` for an O(matching) lookup.
    async fn count_by_chat_id(&self, chat_id: &str) -> Result<u64, AppError>;
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

    async fn count_by_chat_id(&self, chat_id: &str) -> Result<u64, AppError> {
        // `count()` without GROUP returns a single Number row; SurrealDB's
        // SurrealValue derive doesn't deserialize that into a bare u64, so we
        // wrap in a Row struct (same pattern as inference_usage::last_chat_input_tokens).
        #[derive(serde::Deserialize, SurrealValue)]
        #[surreal(crate = "surrealdb::types")]
        struct Row {
            count: u64,
        }
        let mut result = self
            .db()
            .query("SELECT count() AS count FROM tool_call WHERE chat_id = $chat_id GROUP ALL")
            .bind(("chat_id", chat_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let row: Option<Row> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(row.map(|r| r.count).unwrap_or(0))
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
