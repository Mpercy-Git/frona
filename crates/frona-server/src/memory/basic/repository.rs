use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::{Memory, MemoryEntry, MemorySourceType};

#[async_trait]
pub trait MemoryRepository: Repository<Memory> {
    async fn find_latest(
        &self,
        source_type: MemorySourceType,
        source_id: &str,
    ) -> Result<Option<Memory>, AppError>;
}

#[async_trait]
pub trait MemoryEntryRepository: Repository<MemoryEntry> {
    async fn find_by_agent_id(&self, agent_id: &str) -> Result<Vec<MemoryEntry>, AppError>;
    async fn find_by_agent_id_after(
        &self,
        agent_id: &str,
        after: DateTime<Utc>,
    ) -> Result<Vec<MemoryEntry>, AppError>;
    async fn delete_by_agent_id_before(
        &self,
        agent_id: &str,
        before: DateTime<Utc>,
    ) -> Result<(), AppError>;
    async fn find_distinct_agent_ids(&self) -> Result<Vec<String>, AppError>;
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<MemoryEntry>, AppError>;
    async fn find_by_user_id_after(
        &self,
        user_id: &str,
        after: DateTime<Utc>,
    ) -> Result<Vec<MemoryEntry>, AppError>;
    async fn delete_by_user_id_before(
        &self,
        user_id: &str,
        before: DateTime<Utc>,
    ) -> Result<(), AppError>;
    async fn find_distinct_user_ids(&self) -> Result<Vec<String>, AppError>;
}
