use async_trait::async_trait;

use crate::chat::share::models::ChatShare;
use crate::chat::share::repository::ChatShareRepository;
use crate::core::error::AppError;

use super::generic::SurrealRepo;

pub type SurrealChatShareRepo = SurrealRepo<ChatShare>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl ChatShareRepository for SurrealRepo<ChatShare> {
    async fn find_one(
        &self,
        chat_id: &str,
        recipient_id: &str,
    ) -> Result<Option<ChatShare>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM chat_share \
             WHERE chat_id = $chat_id AND recipient_id = $recipient_id LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("chat_id", chat_id.to_string()))
            .bind(("recipient_id", recipient_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let row: Option<ChatShare> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(row)
    }

    async fn find_by_chat(&self, chat_id: &str) -> Result<Vec<ChatShare>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM chat_share WHERE chat_id = $chat_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("chat_id", chat_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows: Vec<ChatShare> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(rows)
    }

    async fn find_by_recipient(&self, recipient_id: &str) -> Result<Vec<ChatShare>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM chat_share WHERE recipient_id = $recipient_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("recipient_id", recipient_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows: Vec<ChatShare> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(rows)
    }

    async fn delete_one(&self, chat_id: &str, recipient_id: &str) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM chat_share WHERE chat_id = $chat_id AND recipient_id = $recipient_id")
            .bind(("chat_id", chat_id.to_string()))
            .bind(("recipient_id", recipient_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
