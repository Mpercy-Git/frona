use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::error::AppError;
use crate::memory::models::MemoryEntry;
use crate::memory::repository::MemoryEntryRepository;

use super::generic::SurrealRepo;

pub type SurrealMemoryEntryRepo = SurrealRepo<MemoryEntry>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl MemoryEntryRepository for SurrealRepo<MemoryEntry> {
    async fn find_by_agent_id(&self, agent_id: &str) -> Result<Vec<MemoryEntry>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM memory_entry WHERE agent_id = $agent_id ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("agent_id", agent_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let entries: Vec<MemoryEntry> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(entries)
    }

    async fn find_by_agent_id_after(
        &self,
        agent_id: &str,
        after: DateTime<Utc>,
    ) -> Result<Vec<MemoryEntry>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM memory_entry WHERE agent_id = $agent_id AND created_at > $after ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("agent_id", agent_id.to_string()))
            .bind(("after", after))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let entries: Vec<MemoryEntry> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(entries)
    }

    async fn delete_by_agent_id_before(
        &self,
        agent_id: &str,
        before: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM memory_entry WHERE agent_id = $agent_id AND created_at <= $before")
            .bind(("agent_id", agent_id.to_string()))
            .bind(("before", before))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn find_distinct_agent_ids(&self) -> Result<Vec<String>, AppError> {
        let mut result = self
            .db()
            .query("SELECT agent_id FROM memory_entry WHERE agent_id != '' AND (user_id IS NULL OR user_id IS NONE) GROUP BY agent_id")
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows: Vec<serde_json::Value> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        let ids = rows
            .into_iter()
            .filter_map(|v| v.get("agent_id").and_then(|id| id.as_str().map(String::from)))
            .collect();

        Ok(ids)
    }

    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<MemoryEntry>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM memory_entry WHERE user_id = $user_id ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let entries: Vec<MemoryEntry> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(entries)
    }

    async fn find_by_user_id_after(
        &self,
        user_id: &str,
        after: DateTime<Utc>,
    ) -> Result<Vec<MemoryEntry>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM memory_entry WHERE user_id = $user_id AND created_at > $after ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .bind(("after", after))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let entries: Vec<MemoryEntry> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(entries)
    }

    async fn delete_by_user_id_before(
        &self,
        user_id: &str,
        before: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM memory_entry WHERE user_id = $user_id AND created_at <= $before")
            .bind(("user_id", user_id.to_string()))
            .bind(("before", before))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn find_distinct_user_ids(&self) -> Result<Vec<String>, AppError> {
        let mut result = self
            .db()
            .query("SELECT user_id FROM memory_entry WHERE user_id IS NOT NULL GROUP BY user_id")
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows: Vec<serde_json::Value> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        let ids = rows
            .into_iter()
            .filter_map(|v| v.get("user_id").and_then(|id| id.as_str().map(String::from)))
            .collect();

        Ok(ids)
    }
}
