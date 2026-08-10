use async_trait::async_trait;

use crate::chat::models::ChatSummary;
use crate::chat::repository::ChatSummaryRepository;
use crate::core::error::AppError;

use super::generic::SurrealRepo;

pub type SurrealChatSummaryRepo = SurrealRepo<ChatSummary>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

/// Wrap a SurrealDB query/take error as `AppError::Database`.
fn db_err(e: impl std::fmt::Display) -> AppError {
    AppError::Database(e.to_string())
}

#[async_trait]
impl ChatSummaryRepository for SurrealRepo<ChatSummary> {
    async fn find_by_chat_id(&self, chat_id: &str) -> Result<Option<ChatSummary>, AppError> {
        let query =
            format!("{SELECT_CLAUSE} FROM chat_summary WHERE chat_id = $chat_id LIMIT 1");
        let mut result = self
            .db()
            .query(&query)
            .bind(("chat_id", chat_id.to_string()))
            .await
            .map_err(db_err)?;

        let row: Option<ChatSummary> = result
            .take(0)
            .map_err(db_err)?;

        Ok(row)
    }
}
