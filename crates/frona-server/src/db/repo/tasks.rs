use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crate::core::error::AppError;
use crate::agent::task::models::Task;
use crate::agent::task::repository::TaskRepository;

use super::generic::SurrealRepo;

pub type SurrealTaskRepo = SurrealRepo<Task>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl TaskRepository for SurrealRepo<Task> {
    async fn find_active_by_user_id(&self, user_id: &str) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE user_id = $user_id AND (status.Pending IS NOT NONE OR status.InProgress IS NOT NONE) ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tasks)
    }

    async fn find_all_by_user_id(&self, user_id: &str) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE user_id = $user_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tasks)
    }

    async fn find_resumable(&self, now: DateTime<Utc>) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE (status.Pending IS NOT NONE OR status.InProgress IS NOT NONE) AND kind.Cron IS NONE AND (run_at IS NONE OR run_at <= $now) ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("now", now))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tasks)
    }

    async fn find_by_chat_id(&self, chat_id: &str) -> Result<Option<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE chat_id = $chat_id LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("chat_id", chat_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let task: Option<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(task)
    }

    async fn find_by_source_chat_id(&self, source_chat_id: &str) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE kind.Delegation.source_chat_id = $source_chat_id ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("source_chat_id", source_chat_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tasks)
    }

    async fn find_due_cron_templates(&self, now: DateTime<Utc>) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE kind.Cron IS NOT NONE AND kind.Cron.next_run_at <= $now AND status.Pending IS NOT NONE ORDER BY kind.Cron.next_run_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("now", now))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tasks)
    }

    async fn find_runs_by_cron(&self, cron_id: &str) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE kind.CronRun.source_cron_id = $cron_id ORDER BY kind.CronRun.sequence_num DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("cron_id", cron_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let tasks: Vec<Task> = result.take(0).map_err(|e| AppError::Database(e.to_string()))?;
        Ok(tasks)
    }

    async fn find_active_runs_by_cron(&self, cron_id: &str) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE kind.CronRun.source_cron_id = $cron_id \
             AND (status.Pending IS NOT NONE OR status.InProgress IS NOT NONE) \
             ORDER BY kind.CronRun.sequence_num ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("cron_id", cron_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let tasks: Vec<Task> = result.take(0).map_err(|e| AppError::Database(e.to_string()))?;
        Ok(tasks)
    }

    /// Crash-recovery query: any CronRun still in Pending/InProgress on startup —
    /// these were interrupted mid-flight and should be marked Failed (or restarted).
    async fn find_orphaned_cron_runs(&self) -> Result<Vec<Task>, AppError> {
        // InProgress only — Pending CronRuns haven't started yet and should
        // be picked up by `find_resumable`, not marked Failed.
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE kind.CronRun IS NOT NONE \
             AND status.InProgress IS NOT NONE"
        );
        let mut result = self
            .db()
            .query(&query)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let tasks: Vec<Task> = result.take(0).map_err(|e| AppError::Database(e.to_string()))?;
        Ok(tasks)
    }

    async fn find_deferred_due(&self, now: DateTime<Utc>) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE run_at IS NOT NONE AND run_at <= $now AND status.Pending IS NOT NONE AND kind.Cron IS NONE ORDER BY run_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("now", now))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tasks)
    }

    async fn find_pending_signal_tasks(&self) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE kind.Signal IS NOT NONE AND status.Pending IS NOT NONE ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tasks)
    }

    async fn find_expired_signal_tasks(&self, now: DateTime<Utc>) -> Result<Vec<Task>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM task WHERE kind.Signal IS NOT NONE AND status.Pending IS NOT NONE AND kind.Signal.expires_at IS NOT NONE AND kind.Signal.expires_at <= $now ORDER BY kind.Signal.expires_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("now", now))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tasks)
    }
}
