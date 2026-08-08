//! SurrealDB repositories for the basic memory service: the `memory` blob
//! table (`SurrealMemoryRepo`) and the immutable `memory_entry` table
//! (`SurrealMemoryEntryRepo`). Trait definitions live in
//! `crate::memory::basic::repository`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::error::AppError;
use crate::memory::basic::models::{Memory, MemoryEntry, MemorySourceType};
use crate::memory::basic::repository::{MemoryEntryRepository, MemoryRepository};

use super::generic::SurrealRepo;

pub type SurrealMemoryRepo = SurrealRepo<Memory>;
pub type SurrealMemoryEntryRepo = SurrealRepo<MemoryEntry>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

/// Wrap a SurrealDB query/take error as `AppError::Database`.
fn db_err(e: impl std::fmt::Display) -> AppError {
    AppError::Database(e.to_string())
}

#[async_trait]
impl MemoryRepository for SurrealRepo<Memory> {
    async fn find_latest(
        &self,
        source_type: MemorySourceType,
        source_id: &str,
    ) -> Result<Option<Memory>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM memory WHERE source_type = $st AND source_id = $sid ORDER BY created_at DESC LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("st", source_type))
            .bind(("sid", source_id.to_string()))
            .await
            .map_err(db_err)?;

        let memory: Option<Memory> = result
            .take(0)
            .map_err(db_err)?;

        Ok(memory)
    }
}

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
            .map_err(db_err)?;

        let entries: Vec<MemoryEntry> = result
            .take(0)
            .map_err(db_err)?;

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
            .map_err(db_err)?;

        let entries: Vec<MemoryEntry> = result
            .take(0)
            .map_err(db_err)?;

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
            .map_err(db_err)?;

        Ok(())
    }

    async fn find_distinct_agent_ids(&self) -> Result<Vec<String>, AppError> {
        let mut result = self
            .db()
            .query("SELECT VALUE agent_id FROM memory_entry WHERE agent_id != '' AND (user_id IS NULL OR user_id IS NONE) GROUP BY agent_id")
            .await
            .map_err(db_err)?;

        // agent_id is nullable (an entry is agent- or user-scoped), and SurrealDB's
        // `NONE != ''` is true, so a NONE can slip through - flatten it away.
        let ids: Vec<Option<String>> = result
            .take(0)
            .map_err(db_err)?;
        Ok(ids.into_iter().flatten().collect())
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
            .map_err(db_err)?;

        let entries: Vec<MemoryEntry> = result
            .take(0)
            .map_err(db_err)?;

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
            .map_err(db_err)?;

        let entries: Vec<MemoryEntry> = result
            .take(0)
            .map_err(db_err)?;

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
            .map_err(db_err)?;

        Ok(())
    }

    async fn find_distinct_user_ids(&self) -> Result<Vec<String>, AppError> {
        let mut result = self
            .db()
            .query("SELECT VALUE user_id FROM memory_entry WHERE user_id IS NOT NULL GROUP BY user_id")
            .await
            .map_err(db_err)?;

        // user_id is nullable (an entry is agent- or user-scoped) - flatten any NONE.
        let ids: Vec<Option<String>> = result
            .take(0)
            .map_err(db_err)?;
        Ok(ids.into_iter().flatten().collect())
    }
}
