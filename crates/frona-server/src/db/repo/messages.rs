use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crate::storage::Attachment;
use crate::core::error::AppError;
use crate::chat::message::models::Message;
use crate::chat::message::repository::MessageRepository;

use super::generic::SurrealRepo;

pub type SurrealMessageRepo = SurrealRepo<Message>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl MessageRepository for SurrealRepo<Message> {
    async fn find_by_chat_id(&self, chat_id: &str) -> Result<Vec<Message>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM message WHERE chat_id = $chat_id ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("chat_id", chat_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let messages: Vec<Message> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(messages)
    }

    async fn find_by_chat_id_after(
        &self,
        chat_id: &str,
        after: DateTime<Utc>,
    ) -> Result<Vec<Message>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM message WHERE chat_id = $chat_id AND created_at > $after ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("chat_id", chat_id.to_string()))
            .bind(("after", after))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let messages: Vec<Message> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(messages)
    }

    async fn delete_by_chat_id_before(
        &self,
        chat_id: &str,
        before: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM message WHERE chat_id = $chat_id AND created_at <= $before")
            .bind(("chat_id", chat_id.to_string()))
            .bind(("before", before))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn delete_by_chat_id(&self, chat_id: &str) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM message WHERE chat_id = $chat_id")
            .bind(("chat_id", chat_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn find_by_chat_id_page(
        &self,
        chat_id: &str,
        before: Option<DateTime<Utc>>,
        after: Option<DateTime<Utc>>,
        limit: u32,
    ) -> Result<Vec<Message>, AppError> {
        let mut query = format!("{SELECT_CLAUSE} FROM message WHERE chat_id = $chat_id");
        let order_desc = after.is_none();

        if before.is_some() {
            query.push_str(" AND created_at < $before");
        }
        if after.is_some() {
            query.push_str(" AND created_at > $after");
        }

        if order_desc {
            query.push_str(" ORDER BY created_at DESC LIMIT $limit");
        } else {
            query.push_str(" ORDER BY created_at ASC LIMIT $limit");
        }

        let mut builder = self
            .db()
            .query(&query)
            .bind(("chat_id", chat_id.to_string()))
            .bind(("limit", limit));

        if let Some(before) = before {
            builder = builder.bind(("before", before));
        }
        if let Some(after) = after {
            builder = builder.bind(("after", after));
        }

        let mut result = builder
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut messages: Vec<Message> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        if order_desc {
            messages.reverse();
        }

        Ok(messages)
    }

    async fn find_attachments_by_chat_id(
        &self,
        chat_id: &str,
    ) -> Result<Vec<Attachment>, AppError> {
        let messages: Vec<Message> = self.find_by_chat_id(chat_id).await?;
        Ok(messages
            .into_iter()
            .flat_map(|m| m.attachments)
            .collect())
    }

    async fn find_due_deliveries(
        &self,
        now: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<Message>, AppError> {
        // SurrealDB stores Rust enums as tagged objects (e.g. `Pending: {}`)
        // when the SurrealValue derive is on a unit variant — direct string
        // comparisons in WHERE wouldn't match. Bind the typed enum values
        // and use `!= sent` to cover both Pending and Failed rows that
        // still have work to do. Status filter excludes Executing — those
        // resume via `resume_all_chats`, not this retry queue.
        use crate::chat::message::models::{DeliveryState, MessageStatus};
        let query = format!(
            "{SELECT_CLAUSE} FROM message
             WHERE delivery.state IS NOT NONE
               AND delivery.state != $sent
               AND delivery.next_attempt_at IS NOT NONE
               AND delivery.next_attempt_at <= $now
               AND status = $completed
             LIMIT $limit"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("sent", DeliveryState::Sent))
            .bind(("completed", MessageStatus::Completed))
            .bind(("now", now))
            .bind(("limit", limit as i64))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let messages: Vec<Message> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(messages)
    }

    async fn resume_deliveries_for_channel(
        &self,
        channel_id: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, AppError> {
        // See find_due_deliveries for why we filter via `!= $sent` instead of `IN [...]`.
        // RETURN meta::id(id) projects to String so we can count via Vec length
        // without deserializing the full Message (whose id is a record, not a string).
        // Status filter excludes Executing — `resume_all_chats` handles those.
        use crate::chat::message::models::{DeliveryState, MessageStatus};
        let query = "UPDATE message
            SET delivery.next_attempt_at = $now
            WHERE chat_id IN (SELECT VALUE meta::id(id) FROM chat WHERE channel_id = $channel_id)
              AND delivery.state IS NOT NONE
              AND delivery.state != $sent
              AND delivery.next_attempt_at IS NOT NONE
              AND status = $completed
            RETURN meta::id(id) AS id";
        let mut result = self
            .db()
            .query(query)
            .bind(("channel_id", channel_id.to_string()))
            .bind(("now", now))
            .bind(("sent", DeliveryState::Sent))
            .bind(("completed", MessageStatus::Completed))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows: Vec<serde_json::Value> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(rows.len() as u64)
    }
}
