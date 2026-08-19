use crate::agent::task::models::Task;
use crate::agent::task::repository::TaskRepository;
use crate::core::error::AppError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

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
        let query =
            format!("{SELECT_CLAUSE} FROM task WHERE user_id = $user_id ORDER BY created_at DESC");
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
        let query = format!("{SELECT_CLAUSE} FROM task WHERE chat_id = $chat_id LIMIT 1");
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
            "{SELECT_CLAUSE} FROM task WHERE \
                kind.Direct.source_chat_id = $source_chat_id \
                OR kind.Delegation.source_chat_id = $source_chat_id \
                OR kind.Cron.source_chat_id = $source_chat_id \
                OR kind.CronRun.source_chat_id = $source_chat_id \
                OR kind.Signal.source_chat_id = $source_chat_id \
                ORDER BY created_at ASC"
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
        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
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
        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(tasks)
    }

    /// Crash-recovery query: any CronRun still in Pending/InProgress on startup -
    /// these were interrupted mid-flight and should be marked Failed (or restarted).
    async fn find_orphaned_cron_runs(&self) -> Result<Vec<Task>, AppError> {
        // InProgress only - Pending CronRuns haven't started yet and should
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
        let tasks: Vec<Task> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::task::models::{CronConcurrency, CronMode, SignalMode, TaskKind, TaskStatus};
    use crate::core::repository::Repository;
    use surrealdb::Surreal;
    use surrealdb::engine::local::Mem;

    fn task(id: &str, kind: TaskKind) -> Task {
        let now = Utc::now();
        Task {
            id: id.into(),
            user_id: "user".into(),
            agent_id: "agent".into(),
            space_id: None,
            chat_id: Some(format!("chat-{id}")),
            title: id.into(),
            description: String::new(),
            status: TaskStatus::Completed,
            kind,
            run_at: None,
            result_summary: None,
            error_message: None,
            quarantined: false,
            result_schema: None,
            result_description: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn source_chat_query_finds_every_child_task_kind() {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        let repo = SurrealRepo::<Task>::new(db);
        let parent = "parent-chat";
        let kinds = vec![
            TaskKind::Direct {
                source_chat_id: Some(parent.into()),
            },
            TaskKind::Delegation {
                source_agent_id: "agent".into(),
                source_chat_id: parent.into(),
                resume_parent: false,
            },
            TaskKind::Cron {
                cron_expression: "0 0 * * *".into(),
                timezone: None,
                next_run_at: None,
                source_agent_id: None,
                source_chat_id: Some(parent.into()),
                mode: CronMode::Singleton,
                concurrency: CronConcurrency::Replace,
                process_result: true,
            },
            TaskKind::CronRun {
                source_cron_id: "cron".into(),
                source_chat_id: Some(parent.into()),
                source_agent_id: Some("agent".into()),
                fire_at: Utc::now(),
                sequence_num: 1,
            },
            TaskKind::Signal {
                source_chat_id: parent.into(),
                resume_parent: false,
                mode: SignalMode::Once,
                expected_categories: Vec::new(),
                expected_channels: Vec::new(),
                expected_contacts: Vec::new(),
                expires_at: None,
                max_evaluations: 1,
                evaluation_count: 0,
            },
        ];
        for (index, kind) in kinds.into_iter().enumerate() {
            repo.create(&task(&format!("task-{index}"), kind))
                .await
                .unwrap();
        }
        repo.create(&task(
            "unrelated",
            TaskKind::Direct {
                source_chat_id: Some("other-chat".into()),
            },
        ))
        .await
        .unwrap();

        let found = repo.find_by_source_chat_id(parent).await.unwrap();

        assert_eq!(found.len(), 5);
        assert!(
            found
                .iter()
                .all(|task| task.kind.source_chat_id() == Some(parent))
        );
    }
}
