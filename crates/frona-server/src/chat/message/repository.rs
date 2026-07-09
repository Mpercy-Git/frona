use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crate::storage::Attachment;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Message;

#[async_trait]
pub trait MessageRepository: Repository<Message> {
    async fn find_by_chat_id(&self, chat_id: &str) -> Result<Vec<Message>, AppError>;
    async fn find_by_chat_id_after(
        &self,
        chat_id: &str,
        after: DateTime<Utc>,
    ) -> Result<Vec<Message>, AppError>;
    async fn delete_by_chat_id_before(
        &self,
        chat_id: &str,
        before: DateTime<Utc>,
    ) -> Result<(), AppError>;
    async fn delete_by_chat_id(&self, chat_id: &str) -> Result<(), AppError>;
    async fn find_attachments_by_chat_id(
        &self,
        chat_id: &str,
    ) -> Result<Vec<Attachment>, AppError>;
    async fn find_by_chat_id_page(
        &self,
        chat_id: &str,
        before: Option<DateTime<Utc>>,
        after: Option<DateTime<Utc>>,
        limit: u32,
    ) -> Result<Vec<Message>, AppError>;
    async fn find_due_deliveries(
        &self,
        now: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<Message>, AppError>;
    async fn resume_deliveries_for_channel(
        &self,
        channel_id: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, AppError>;

    /// Orphans from a crash between `status = Completed` and the watcher
    /// stamping delivery - swept on channel start.
    async fn find_undelivered_completed_for_channel(
        &self,
        channel_id: &str,
    ) -> Result<Vec<Message>, AppError>;
}
