use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use crate::agent::execution::{self, AgentLoopOutcome};
use crate::inference::conversation::DefaultConversationBuilder;
use crate::agent::models::Agent;
use crate::agent::task::models::TaskKind;
use crate::db::repo::generic::SurrealRepo;
use crate::db::repo::memory_entries::SurrealMemoryEntryRepo;
use crate::core::state::AppState;
use crate::chat::models::CreateChatRequest;
use crate::chat::repository::ChatRepository;
use crate::core::error::AppError;
use crate::inference::config::ModelGroup;
use crate::inference::InferenceResponse;
use crate::memory::repository::MemoryEntryRepository;
use crate::memory::models::MemorySourceType;
use crate::space::repository::SpaceRepository;
use crate::tool::task::next_cron_occurrence;

pub struct Scheduler {
    app_state: AppState,
    compaction_model_group: ModelGroup,
}

macro_rules! spawn_periodic {
    ($scheduler:expr, $interval:expr, $name:expr, $method:ident, $shutdown:expr) => {{
        let s = $scheduler.clone();
        let shutdown = $shutdown.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = tokio::time::sleep($interval) => {
                        if let Err(e) = s.$method().await {
                            tracing::warn!(error = %e, task = $name, "Scheduled task failed");
                        }
                    }
                    () = shutdown.cancelled() => {
                        tracing::info!(task = $name, "Scheduler loop stopping for shutdown");
                        break;
                    }
                }
            }
        });
    }};
}

impl Scheduler {
    pub fn new(app_state: AppState, compaction_model_group: ModelGroup) -> Self {
        Self {
            app_state,
            compaction_model_group,
        }
    }

    pub fn start(self: Arc<Self>) {
        let cfg = &self.app_state.config;
        let space = Duration::from_secs(cfg.scheduler.space_compaction_secs);
        let memory = Duration::from_secs(cfg.scheduler.memory_compaction_secs);
        let poll = Duration::from_secs(cfg.scheduler.poll_secs);

        let shutdown = self.app_state.shutdown_token.clone();
        spawn_periodic!(self, space, "space_compaction", run_space_compaction, shutdown);
        spawn_periodic!(self, memory, "memory_compaction", run_memory_compaction, shutdown);
        spawn_periodic!(self, memory, "user_memory_compaction", run_user_memory_compaction, shutdown);
        spawn_periodic!(self, poll, "poll_tasks", run_poll_tasks, shutdown);
        spawn_periodic!(self, space, "token_cleanup", run_token_cleanup, shutdown);
    }

    async fn run_poll_tasks(&self) -> Result<(), AppError> {
        self.run_cron_tasks().await?;
        self.run_deferred_tasks().await?;
        self.run_signal_timeouts().await?;
        self.run_delivery_retries().await?;
        self.run_pair_expiry_sweep().await?;
        self.run_heartbeats().await
    }

    async fn run_pair_expiry_sweep(&self) -> Result<(), AppError> {
        if self.app_state.is_shutting_down() {
            return Ok(());
        }
        self.app_state
            .channel_service
            .revert_expired_pairings()
            .await
            .map(|_count| ())
    }

    async fn run_delivery_retries(&self) -> Result<(), AppError> {
        if self.app_state.is_shutting_down() {
            return Ok(());
        }
        self.app_state
            .channel_manager
            .retry_due_deliveries()
            .await
            .map(|_count| ())
    }

    async fn run_signal_timeouts(&self) -> Result<(), AppError> {
        if self.app_state.is_shutting_down() {
            return Ok(());
        }
        let expired = self.app_state.task_service.find_expired_signal_tasks().await?;
        if expired.is_empty() {
            return Ok(());
        }
        let signal_service = self.app_state.signal_service();
        for task in expired {
            let is_continuous = matches!(
                task.kind,
                TaskKind::Signal {
                    mode: crate::agent::task::models::SignalMode::Continuous,
                    ..
                }
            );
            let outcome = if is_continuous {
                self.app_state
                    .task_service
                    .mark_completed(&task.id, Some("monitoring window ended".into()))
                    .await
                    .map(|_| ())
            } else {
                self.app_state
                    .task_service
                    .mark_failed(&task.id, "timed out: no matching signal".into())
                    .await
                    .map(|_| ())
            };
            if let Err(e) = outcome {
                tracing::warn!(
                    task_id = %task.id,
                    error = %e,
                    "Failed to transition expired signal task"
                );
                continue;
            }
            if let Some(ref svc) = signal_service {
                svc.unregister(&task.user_id, &task.id).await;
            }
        }
        Ok(())
    }

    async fn run_cron_tasks(&self) -> Result<(), AppError> {
        if self.app_state.is_shutting_down() {
            return Ok(());
        }
        let templates = self.app_state.task_service.find_due_cron_templates().await?;
        if templates.is_empty() {
            return Ok(());
        }

        let server_tz = self.app_state.config.server.timezone.as_str();

        for template in templates {
            let (cron_expression, timezone) = match &template.kind {
                TaskKind::Cron { cron_expression, timezone, .. } => (
                    cron_expression.clone(),
                    crate::auth::models::resolve_timezone(timezone.as_deref(), server_tz),
                ),
                _ => continue,
            };

            tracing::info!(
                task_id = %template.id,
                title = %template.title,
                timezone = %timezone,
                "Firing cron task"
            );

            let app_state = self.app_state.clone();
            let task_service = self.app_state.task_service.clone();
            let cron_expr = cron_expression.clone();
            let task_clone = template.clone();
            let tz_clone = timezone.clone();

            tokio::spawn(async move {
                if let Err(e) = execute_cron(&app_state, &task_clone).await {
                    tracing::error!(
                        error = %e,
                        task_id = %task_clone.id,
                        "Cron execution failed"
                    );
                }
                match next_cron_occurrence(&cron_expr, &tz_clone) {
                    Ok(next) => {
                        if let Err(e) = task_service
                            .advance_cron_template(&task_clone.id, next)
                            .await
                        {
                            tracing::warn!(error = %e, task_id = %task_clone.id, "Failed to advance cron template");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, task_id = %task_clone.id, "Failed to compute next cron occurrence");
                    }
                }
            });
        }

        Ok(())
    }

    async fn run_deferred_tasks(&self) -> Result<(), AppError> {
        if self.app_state.is_shutting_down() {
            return Ok(());
        }
        let tasks = self.app_state.task_service.find_deferred_due().await?;
        if tasks.is_empty() {
            return Ok(());
        }

        let executor = match self.app_state.task_executor() {
            Some(e) => e,
            None => {
                tracing::warn!("Task executor not available, skipping deferred tasks");
                return Ok(());
            }
        };

        for task in tasks {
            tracing::info!(
                task_id = %task.id,
                title = %task.title,
                "Firing deferred task"
            );

            if let Err(e) = executor.spawn_execution(task).await {
                tracing::warn!(error = %e, "Failed to spawn deferred task");
            }
        }

        Ok(())
    }

    async fn run_heartbeats(&self) -> Result<(), AppError> {
        if self.app_state.is_shutting_down() {
            return Ok(());
        }
        let now = Utc::now();
        let agents = self.app_state.agent_service.find_due_heartbeats(now).await?;
        if agents.is_empty() {
            return Ok(());
        }

        for agent in agents {
            let interval = match agent.heartbeat_interval {
                Some(mins) if mins > 0 => mins,
                _ => continue,
            };

            let user_id = agent.user_id.clone();
            let Some(user) = self.app_state.user_service.find_by_id(&user_id).await? else {
                tracing::warn!(agent_id = %agent.id, user_id = %user_id, "Heartbeat agent's user not found, skipping");
                continue;
            };
            let ws = self.app_state.storage_service.agent_workspace(&user.handle, &agent.handle);
            let heartbeat_content = match ws.read("HEARTBEAT.md") {
                Some(content) if !content.trim().is_empty() => content,
                _ => {
                    let next = now + chrono::Duration::minutes(interval as i64);
                    let _ = self.app_state.agent_service.update_next_heartbeat(&agent.id, Some(next)).await;
                    continue;
                }
            };

            tracing::info!(
                agent_id = %agent.id,
                "Firing heartbeat"
            );

            let app_state = self.app_state.clone();
            let agent_clone = agent.clone();

            tokio::spawn(async move {
                if let Err(e) = execute_heartbeat(&app_state, &agent_clone, &user_id, &heartbeat_content).await {
                    tracing::error!(
                        error = %e,
                        agent_id = %agent_clone.id,
                        "Heartbeat execution failed"
                    );
                }
                let next = Utc::now() + chrono::Duration::minutes(interval as i64);
                if let Err(e) = app_state.agent_service.update_next_heartbeat(&agent_clone.id, Some(next)).await {
                    tracing::error!(
                        error = %e,
                        agent_id = %agent_clone.id,
                        "Failed to advance heartbeat"
                    );
                }
            });
        }

        Ok(())
    }

    async fn run_token_cleanup(&self) -> Result<(), AppError> {
        let deleted = self.app_state.token_service.cleanup_expired().await?;
        if deleted > 0 {
            tracing::info!(count = deleted, "Cleaned up expired tokens");
        }
        Ok(())
    }

    async fn run_space_compaction(&self) -> Result<(), AppError> {
        let space_repo: SurrealRepo<crate::space::models::Space> =
            SurrealRepo::new(self.app_state.db.clone());
        let chat_repo: SurrealRepo<crate::chat::models::Chat> =
            SurrealRepo::new(self.app_state.db.clone());

        let spaces = space_repo.find_all().await?;

        for space in spaces {
            let chats = chat_repo.find_by_space_id(&space.id).await?;
            if chats.is_empty() {
                continue;
            }

            let mut summaries = Vec::new();
            for chat in &chats {
                let title = chat.title.clone().unwrap_or_else(|| "Untitled".to_string());

                let memory = self
                    .app_state
                    .memory_service
                    .get_memory(MemorySourceType::Chat, &chat.id)
                    .await?;

                let summary = if let Some(mem) = memory {
                    mem.content
                } else {
                    format!("(No summary available for chat: {title})")
                };

                summaries.push((title, summary));
            }

            if let Err(e) = self
                .app_state
                .memory_service
                .compact_space(&space.id, summaries, &self.compaction_model_group)
                .await
            {
                tracing::warn!(
                    space_id = %space.id,
                    error = %e,
                    "Failed to compact space"
                );
            }
        }

        Ok(())
    }

    async fn run_memory_compaction(&self) -> Result<(), AppError> {
        let repo: SurrealMemoryEntryRepo = SurrealRepo::new(self.app_state.db.clone());
        let ids = repo.find_distinct_agent_ids().await?;
        self.run_memory_compaction_for("agent", ids).await
    }

    async fn run_user_memory_compaction(&self) -> Result<(), AppError> {
        let repo: SurrealMemoryEntryRepo = SurrealRepo::new(self.app_state.db.clone());
        let ids = repo.find_distinct_user_ids().await?;
        self.run_memory_compaction_for("user", ids).await
    }

    async fn run_memory_compaction_for(
        &self,
        kind: &str,
        ids: Vec<String>,
    ) -> Result<(), AppError> {
        tracing::info!(count = ids.len(), kind = kind, "Starting scheduled memory compaction");
        for id in &ids {
            tracing::info!(%id, kind = kind, "Running scheduled memory compaction");
            let result = match kind {
                "agent" => {
                    self.app_state
                        .memory_service
                        .compact_entries_if_needed(id, &self.compaction_model_group)
                        .await
                }
                "user" => {
                    self.app_state
                        .memory_service
                        .compact_user_entries_if_needed(id, &self.compaction_model_group)
                        .await
                }
                _ => Ok(()),
            };
            if let Err(e) = result {
                tracing::warn!(%id, kind = kind, error = %e, "Failed to compact memory entries");
            }
        }
        Ok(())
    }
}

pub async fn execute_cron(
    state: &AppState,
    template: &crate::agent::task::models::Task,
) -> Result<(), AppError> {
    use crate::agent::task::models::CronConcurrency;

    let concurrency = match &template.kind {
        TaskKind::Cron { concurrency, .. } => *concurrency,
        _ => return Ok(()),
    };

    let active = state
        .task_service
        .find_active_runs_by_cron(&template.id)
        .await
        .unwrap_or_default();

    if !active.is_empty() {
        match concurrency {
            CronConcurrency::Allow => {}
            CronConcurrency::Forbid => {
                tracing::info!(
                    template_id = %template.id,
                    active = active.len(),
                    "Cron concurrency=Forbid: skipping fire while previous still active"
                );
                return Ok(());
            }
            CronConcurrency::Replace => {
                if let Some(executor) = state.task_executor() {
                    for run in &active {
                        executor.cancel_task(&run.id).await;
                    }
                }
            }
        }
    }

    let next_sequence = state
        .task_service
        .find_runs_by_cron(&template.id)
        .await
        .map(|runs| runs.len() as u64 + 1)
        .unwrap_or(1);

    let run = state
        .task_service
        .spawn_cron_run(template, Utc::now(), next_sequence)
        .await?;

    if let Some(executor) = state.task_executor() {
        if let Err(e) = executor.spawn_execution(run).await {
            tracing::warn!(error = %e, template_id = %template.id, "Failed to spawn CronRun execution");
        }
    } else {
        tracing::warn!(template_id = %template.id, "No task executor available to spawn CronRun");
    }

    Ok(())
}

async fn execute_heartbeat(
    state: &AppState,
    agent: &Agent,
    user_id: &str,
    heartbeat_content: &str,
) -> Result<(), AppError> {
    let agent_id = &agent.id;

    let chat_id = if let Some(ref cid) = agent.heartbeat_chat_id {
        cid.clone()
    } else {
        let chat = state
            .chat_service
            .create_chat(
                user_id,
                CreateChatRequest {
                    space_id: None,
                    task_id: None,
                    agent_id: agent_id.clone(),
                    title: Some("Heartbeat".to_string()),
                    metadata: None,
                },
            )
            .await?;

        state
            .agent_service
            .update_heartbeat_chat(&agent.id, &chat.id)
            .await?;

        chat.id
    };

    let message = format!(
        "Heartbeat: review and act on your checklist.\n\n{}",
        heartbeat_content
    );
    let cancel_token = state.active_sessions.register(&chat_id).await;
    execute_background_agent(state, user_id, &chat_id, &message, cancel_token).await
}

async fn execute_background_agent(
    state: &AppState,
    user_id: &str,
    chat_id: &str,
    message_content: &str,
    cancel_token: tokio_util::sync::CancellationToken,
) -> Result<(), AppError> {
    state
        .chat_service
        .create_stream_user_message(user_id, chat_id, message_content, vec![])
        .await?;

    let chat = state.chat_service.find_chat(chat_id).await?
        .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;
    let agent_msg = state.chat_service
        .create_executing_agent_message(chat_id, &chat.agent_id)
        .await?;
    let agent_msg_id = agent_msg.id.clone();

    let builder = Box::new(DefaultConversationBuilder {
        user_service: state.user_service.clone(),
        storage_service: state.storage_service.clone(),
    });
    let result = execution::run_agent_loop(
        state, user_id, chat_id, &agent_msg_id, cancel_token, builder, &[],
    )
    .await;

    match result {
        Ok(AgentLoopOutcome { response }) => match response {
            InferenceResponse::Completed { text, attachments, reasoning, .. } => {
                let _ = state.chat_service
                    .complete_agent_message(&agent_msg_id, text, attachments, reasoning)
                    .await;
            }
            InferenceResponse::Cancelled(text) => {
                let _ = state.chat_service
                    .complete_agent_message(&agent_msg_id, text, vec![], None)
                    .await;
            }
            InferenceResponse::ExternalToolPending { .. } => {
                tracing::warn!(chat_id = %chat_id, "Background agent hit external tool pending — not supported");
            }
        },
        Err(e) => {
            let _ = state.chat_service.fail_agent_message(&agent_msg_id).await;
            tracing::error!(error = %e, chat_id = %chat_id, "Background agent tool loop failed");
        }
    }

    state.active_sessions.remove(chat_id).await;
    Ok(())
}
