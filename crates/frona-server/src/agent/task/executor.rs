use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::agent::execution;
use crate::agent::task::models::{Task, TaskKind, TaskStatus};
use crate::chat::broadcast::BroadcastEventKind;
use crate::chat::message::models::{MessageEvent, MessageRole};
use crate::inference::tool_call::MessageTool;
use crate::chat::models::CreateChatRequest;
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::inference::InferenceResponse;
use crate::storage::Attachment;

const MAX_TASK_RETRIES: usize = 10;

pub struct TaskExecutor {
    app_state: AppState,
    active_tasks: Mutex<HashMap<String, CancellationToken>>,
}

impl TaskExecutor {
    pub fn new(app_state: AppState) -> Self {
        Self {
            app_state,
            active_tasks: Mutex::new(HashMap::new()),
        }
    }

    pub async fn resume_all(self: &Arc<Self>) {
        let tasks = match self.app_state.task_service.find_resumable().await {
            Ok(tasks) => tasks,
            Err(e) => {
                tracing::error!(error = %e, "Failed to query resumable tasks");
                return;
            }
        };

        if tasks.is_empty() {
            return;
        }

        tracing::info!(count = tasks.len(), "Resuming tasks from previous run");

        for task in tasks {
            if task.status == TaskStatus::Cancelled {
                continue;
            }

            if let Err(e) = self.spawn_execution(task).await {
                tracing::warn!(error = %e, "Failed to spawn task during resume");
            }
        }
    }

    pub async fn spawn_execution(self: &Arc<Self>, task: Task) -> Result<(), AppError> {
        if self.app_state.is_shutting_down() {
            tracing::info!(task_id = %task.id, "Rejecting task spawn during shutdown");
            return Ok(());
        }

        let active = self.active_tasks.lock().await;
        if active.len() >= self.app_state.max_concurrent_tasks {
            tracing::info!(
                task_id = %task.id,
                active = active.len(),
                limit = self.app_state.max_concurrent_tasks,
                "Global concurrency limit reached, task stays Pending"
            );
            return Ok(());
        }

        let agent_max = self.get_agent_concurrent_limit(&task.agent_id).await;
        let agent_active_count = active
            .keys()
            .filter(|k| k.starts_with(&format!("{}:", task.agent_id)))
            .count();

        if agent_active_count >= agent_max {
            tracing::info!(
                task_id = %task.id,
                agent_id = %task.agent_id,
                active = agent_active_count,
                limit = agent_max,
                "Per-agent concurrency limit reached, task stays Pending"
            );
            return Ok(());
        }
        drop(active);

        let cancel_token = CancellationToken::new();
        let key = format!("{}:{}", task.agent_id, task.id);
        self.active_tasks
            .lock()
            .await
            .insert(key.clone(), cancel_token.clone());

        let executor = Arc::clone(self);
        let task_id = task.id.clone();

        tokio::spawn(async move {
            let result = executor.execute_task(task, cancel_token).await;
            executor.active_tasks.lock().await.remove(&key);

            if let Err(e) = result {
                tracing::error!(error = %e, task_id = %task_id, "Task execution failed");
            }
        });

        Ok(())
    }

    pub async fn cancel_task(&self, task_id: &str) -> bool {
        let active = self.active_tasks.lock().await;
        for (key, token) in active.iter() {
            if key.ends_with(&format!(":{}", task_id)) {
                token.cancel();
                return true;
            }
        }
        false
    }

    async fn get_agent_concurrent_limit(&self, agent_id: &str) -> usize {
        if let Ok(Some(agent)) = self.app_state.agent_service.find_by_id(agent_id).await {
            return agent.max_concurrent_tasks.unwrap_or(3) as usize;
        }
        3
    }

    async fn execute_task(
        self: &Arc<Self>,
        mut task: Task,
        cancel_token: CancellationToken,
    ) -> Result<(), AppError> {
        let task_id = task.id.clone();

        let current_status = self
            .app_state
            .task_service
            .find_by_id(&task_id)
            .await?
            .map(|t| t.status);

        if matches!(current_status, Some(TaskStatus::Cancelled)) {
            tracing::info!(task_id = %task_id, "Task already cancelled, skipping");
            return Ok(());
        }

        let chat_id = self.ensure_task_chat(&mut task).await?;
        let event_sender = self.app_state.broadcast_service.create_event_sender(&task.user_id, &chat_id);

        self.app_state
            .task_service
            .mark_in_progress(&task_id, Some(&chat_id))
            .await?;

        self.broadcast_task_status(&task, "inprogress", None);
        self.save_initial_message_if_needed(&task, &chat_id).await?;

        for turn in 0..MAX_TASK_RETRIES {
            // Check for unprocessed lifecycle events (crash recovery)
            if let Some(action) = self.find_lifecycle_event(&chat_id).await {
                self.handle_lifecycle_action(&task, &chat_id, action)
                    .await?;
                return Ok(());
            }

            let continuation_prompt = if turn > 0 {
                self.app_state.prompts.read("TASK_CONTINUATION.md")
            } else {
                None
            };

            let agent_msg_id = match self.app_state.chat_service
                .find_executing_message_for_chat(&chat_id)
                .await
            {
                Ok(Some(msg)) => msg.id,
                _ => {
                    let msg = self.app_state.chat_service
                        .create_executing_agent_message(&chat_id, &task.agent_id)
                        .await?;
                    msg.id
                }
            };

            let session_token = self.app_state.active_sessions.register(&chat_id).await;
            let result = execution::run_agent_loop(
                &self.app_state,
                &task.user_id,
                &chat_id,
                &agent_msg_id,
                cancel_token.clone(),
                true,
                continuation_prompt.as_deref(),
            )
            .await;
            drop(session_token);
            self.app_state.active_sessions.remove(&chat_id).await;

            match result {
                Ok(execution::AgentLoopOutcome { response }) => match response {
                    InferenceResponse::Completed { text, attachments, lifecycle_event, reasoning, .. } => {
                        if let Ok(mut msg) = self.app_state.chat_service
                            .complete_agent_message(&agent_msg_id, text.clone(), attachments.clone(), reasoning)
                            .await
                        {
                            // Populate tool_calls so the SSE has complete_task/TaskCompletion data
                            if let Ok(tes) = self.app_state.chat_service
                                .get_tool_calls_by_message(&agent_msg_id).await
                            {
                                msg.tool_calls = tes.into_iter().map(Into::into).collect();
                            }
                            crate::credential::presign::presign_response_by_user_id(
                                &self.app_state.presign_service, &mut msg, &task.user_id,
                            ).await;
                            event_sender.send_kind(BroadcastEventKind::InferenceDone { message: msg });
                        }

                        if let Some(event) = lifecycle_event {
                            let action = self.lifecycle_action_from_event(event);
                            self.handle_lifecycle_action(&task, &chat_id, action)
                                .await?;
                            return Ok(());
                        }

                        if let Some(action) = self.find_lifecycle_event(&chat_id).await {
                            self.handle_lifecycle_action(&task, &chat_id, action)
                                .await?;
                            return Ok(());
                        }
                        continue;
                    }
                    InferenceResponse::ExternalToolPending { tool_calls, .. } => {
                        for te in tool_calls {
                            event_sender.send_kind(BroadcastEventKind::ToolCallCreated { tool_call: te });
                        }
                        self.wait_for_resolution(&task.id, &cancel_token).await?;
                        continue;
                    }
                    InferenceResponse::Cancelled(text) => {
                        let _ = self.app_state.chat_service
                            .complete_agent_message(&agent_msg_id, text, vec![], None)
                            .await;
                        event_sender.send_kind(BroadcastEventKind::InferenceCancelled {
                            reason: "Task cancelled".to_string(),
                        });
                        self.handle_cancelled(&task).await?;
                        return Ok(());
                    }
                },
                Err(e) => {
                    let _ = self.app_state.chat_service
                        .fail_agent_message(&agent_msg_id).await;
                    event_sender.send_kind(BroadcastEventKind::InferenceError {
                        error: e.to_string(),
                    });
                    self.handle_error(&task, &e).await?;
                    return Ok(());
                }
            }
        }

        tracing::warn!(
            task_id = %task.id,
            turns = MAX_TASK_RETRIES,
            "Task reached max retries, auto-completing"
        );
        self.app_state
            .task_service
            .mark_completed(&task.id, Some("Task auto-completed after max retries".into()))
            .await?;
        self.deliver_to_source(
            &task,
            TaskStatus::Completed,
            Some("Task auto-completed after max retries".into()),
            vec![],
        )
        .await;
        self.broadcast_task_status(&task, "completed", Some("Task auto-completed after max retries"));

        Ok(())
    }

    async fn find_lifecycle_event(&self, chat_id: &str) -> Option<LifecycleAction> {
        let messages = self.app_state.chat_service.get_stored_messages(chat_id).await;

        let deliverables = {
            let tool_calls = self.app_state.chat_service
                .get_tool_calls(chat_id).await.unwrap_or_default();
            tool_calls.into_iter().rev().find_map(|te| {
                match te.tool_data {
                    Some(MessageTool::TaskCompletion { deliverables, .. }) if !deliverables.is_empty() => {
                        Some(deliverables)
                    }
                    _ => None,
                }
            }).unwrap_or_default()
        };

        for msg in messages.iter().rev() {
            if msg.role != MessageRole::System {
                continue;
            }
            match &msg.event {
                Some(MessageEvent::TaskCompletion {
                    status, summary, ..
                }) => {
                    return Some(LifecycleAction::Complete {
                        status: status.clone(),
                        summary: summary.clone(),
                        attachments: deliverables,
                    });
                }
                Some(MessageEvent::TaskDeferred {
                    delay_minutes,
                    reason,
                    ..
                }) => {
                    return Some(LifecycleAction::Defer {
                        delay_minutes: *delay_minutes,
                        reason: reason.clone(),
                    });
                }
                _ => continue,
            }
        }
        None
    }

    fn lifecycle_action_from_event(
        &self,
        event: MessageTool,
    ) -> LifecycleAction {
        match event {
            MessageTool::TaskCompletion { status, summary, deliverables, .. } => {
                LifecycleAction::Complete { status, summary, attachments: deliverables }
            }
            MessageTool::TaskDeferred { delay_minutes, reason, .. } => {
                LifecycleAction::Defer { delay_minutes, reason }
            }
            _ => LifecycleAction::Complete {
                status: TaskStatus::Completed,
                summary: None,
                attachments: vec![],
            },
        }
    }

    async fn handle_lifecycle_action(
        &self,
        task: &Task,
        _chat_id: &str,
        action: LifecycleAction,
    ) -> Result<(), AppError> {
        match action {
            LifecycleAction::Complete {
                status: TaskStatus::Completed,
                summary,
                attachments,
            } => {
                self.app_state
                    .task_service
                    .mark_completed(&task.id, summary.clone())
                    .await?;
                self.deliver_to_source(task, TaskStatus::Completed, summary.clone(), attachments)
                    .await;
                self.broadcast_task_status(task, "completed", summary.as_deref());
            }
            LifecycleAction::Complete {
                status: TaskStatus::Failed,
                summary,
                attachments,
            } => {
                let error_msg = summary.clone().unwrap_or_default();
                self.app_state
                    .task_service
                    .mark_failed(&task.id, error_msg)
                    .await?;
                self.deliver_to_source(task, TaskStatus::Failed, summary, attachments)
                    .await;
                self.broadcast_task_status(task, "failed", None);
            }
            LifecycleAction::Complete { .. } => {}
            LifecycleAction::Defer {
                delay_minutes,
                reason,
            } => {
                let run_at = Utc::now() + chrono::Duration::minutes(delay_minutes as i64);
                self.app_state
                    .task_service
                    .mark_deferred(&task.id, run_at, &reason)
                    .await?;
                self.broadcast_task_status(task, "deferred", Some(&reason));
            }
        }
        Ok(())
    }

    async fn wait_for_resolution(
        &self,
        task_id: &str,
        cancel_token: &CancellationToken,
    ) -> Result<(), AppError> {
        let notify = Arc::new(tokio::sync::Notify::new());
        {
            let mut notifiers = self.app_state.task_resolution_notifiers.lock().await;
            notifiers.insert(task_id.to_string(), notify.clone());
        }

        tokio::select! {
            () = notify.notified() => {}
            () = cancel_token.cancelled() => {}
        }

        {
            let mut notifiers = self.app_state.task_resolution_notifiers.lock().await;
            notifiers.remove(task_id);
        }

        Ok(())
    }

    /// Inject a system message into the task's chat and run one agent turn.
    /// If the agent emits a `complete_task` / `fail_task` lifecycle event,
    /// it's processed and the parent resumes via `deliver_to_source`.
    pub async fn run_with_injected_message(
        self: &Arc<Self>,
        task: &Task,
        system_message: String,
    ) -> Result<(), AppError> {
        let mut task_for_chat = task.clone();
        let was_unset = task_for_chat.chat_id.is_none();
        let chat_id = self.ensure_task_chat(&mut task_for_chat).await?;
        if was_unset {
            // ensure_task_chat populated chat_id on the local clone — persist
            // it so subsequent calls reuse C₂ instead of creating a new chat.
            self.app_state
                .task_service
                .save(&task_for_chat)
                .await?;
        }

        self.app_state
            .chat_service
            .save_system_message(&chat_id, system_message)
            .await?;

        let agent_msg = self
            .app_state
            .chat_service
            .create_executing_agent_message(&chat_id, &task.agent_id)
            .await?;
        let agent_msg_id = agent_msg.id.clone();
        let cancel_token = CancellationToken::new();

        let outcome = execution::run_agent_loop(
            &self.app_state,
            &task.user_id,
            &chat_id,
            &agent_msg_id,
            cancel_token,
            true,
            None,
        )
        .await;

        match outcome {
            Ok(execution::AgentLoopOutcome { response }) => {
                if let InferenceResponse::Completed {
                    text,
                    attachments,
                    reasoning,
                    ..
                } = response
                {
                    let _ = self
                        .app_state
                        .chat_service
                        .complete_agent_message(&agent_msg_id, text, attachments, reasoning)
                        .await;
                }
            }
            Err(e) => {
                let _ = self
                    .app_state
                    .chat_service
                    .fail_agent_message(&agent_msg_id)
                    .await;
                tracing::warn!(
                    task_id = %task.id,
                    error = %e,
                    "Agent failed during injected-message run"
                );
                return Err(e);
            }
        }

        if let Some(action) = self.find_lifecycle_event(&chat_id).await
            && let Err(e) = self.handle_lifecycle_action(task, &chat_id, action).await
        {
            tracing::warn!(
                task_id = %task.id,
                error = %e,
                "Failed to apply lifecycle action after injected-message run"
            );
        }

        Ok(())
    }

    pub async fn ensure_task_chat(&self, task: &mut Task) -> Result<String, AppError> {
        if let Some(ref cid) = task.chat_id {
            return Ok(cid.clone());
        }

        let chat = self
            .app_state
            .chat_service
            .create_chat(
                &task.user_id,
                CreateChatRequest {
                    space_id: task.space_id.clone(),
                    task_id: Some(task.id.clone()),
                    agent_id: task.agent_id.clone(),
                    title: Some(format!("Task: {}", task.title)),
                },
            )
            .await?;
        task.chat_id = Some(chat.id.clone());
        Ok(chat.id)
    }

    pub async fn save_initial_message_if_needed(
        &self,
        task: &Task,
        chat_id: &str,
    ) -> Result<(), AppError> {
        let stored_messages = self.app_state.chat_service.get_stored_messages(chat_id).await;
        if !stored_messages.is_empty() {
            return Ok(());
        }

        let source_agent_id = match &task.kind {
            TaskKind::Delegation {
                source_agent_id, ..
            } => source_agent_id.as_str(),
            _ => &task.agent_id,
        };
        let msg = self
            .app_state
            .chat_service
            .save_agent_message(chat_id, source_agent_id, task.description.clone())
            .await?;
        self.app_state
            .broadcast_service
            .broadcast_chat_message(&task.user_id, chat_id, msg);
        Ok(())
    }

    pub async fn handle_cancelled(
        &self,
        task: &Task,
    ) -> Result<(), AppError> {
        self.app_state
            .task_service
            .mark_cancelled(&task.id)
            .await?;
        self.broadcast_task_status(task, "cancelled", None);
        Ok(())
    }

    pub async fn handle_error(&self, task: &Task, error: &AppError) -> Result<(), AppError> {
        let error_msg = format!("Task execution error: {}", error);
        tracing::error!(error = %error, task_id = %task.id, "Task execution failed");
        self.app_state
            .task_service
            .mark_failed(&task.id, error_msg)
            .await?;
        self.deliver_to_source(task, TaskStatus::Failed, Some(error.to_string()), vec![])
            .await;
        self.broadcast_task_status(task, "failed", None);
        Ok(())
    }

    pub async fn deliver_to_source(
        &self,
        task: &Task,
        status: TaskStatus,
        text: Option<String>,
        attachments: Vec<Attachment>,
    ) {
        let (source_chat_id, resume_parent) = match &task.kind {
            TaskKind::Delegation {
                source_chat_id,
                resume_parent,
                ..
            } => (source_chat_id.as_str(), *resume_parent),
            TaskKind::Direct {
                source_chat_id: Some(source_chat_id),
            } => (source_chat_id.as_str(), false),
            TaskKind::Signal {
                source_chat_id,
                resume_parent,
                ..
            } => (source_chat_id.as_str(), *resume_parent),
            _ => return,
        };

        let content = text.unwrap_or_default();

        let event = MessageEvent::TaskCompletion {
            task_id: task.id.clone(),
            chat_id: task.chat_id.clone(),
            status,
            summary: if content.is_empty() { None } else { Some(content.clone()) },
        };

        match self
            .app_state
            .chat_service
            .save_task_completion_message(source_chat_id, &task.agent_id, content, event, attachments)
            .await
        {
            Ok(mut msg) => {
                crate::credential::presign::presign_response_by_user_id(
                    &self.app_state.presign_service, &mut msg, &task.user_id,
                ).await;
                self.app_state.broadcast_service.broadcast_chat_message(
                    &task.user_id,
                    source_chat_id,
                    msg,
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, task_id = %task.id, "Failed to deliver task result to source chat");
            }
        }

        if resume_parent {
            self.check_and_resume_parent(source_chat_id, &task.user_id)
                .await;
        }
    }

    async fn check_and_resume_parent(&self, source_chat_id: &str, user_id: &str) {
        // Only resume the parent if the source chat belongs to a task.
        // For user chats, just deliver the message — don't trigger the agent.
        let is_task_chat = matches!(
            self.app_state.chat_service.find_chat(source_chat_id).await,
            Ok(Some(chat)) if chat.task_id.is_some()
        );
        if !is_task_chat {
            return;
        }

        let siblings = match self
            .app_state
            .task_service
            .find_by_source_chat_id(source_chat_id)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to query sibling tasks");
                return;
            }
        };

        let all_done = siblings.iter().all(|t| {
            matches!(
                t.status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
            )
        });

        if !all_done {
            tracing::debug!(
                source_chat_id = %source_chat_id,
                remaining = siblings.iter().filter(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress)).count(),
                "Not all sibling tasks done yet"
            );
            return;
        }

        tracing::info!(
            source_chat_id = %source_chat_id,
            "All child tasks complete, resuming parent"
        );

        let state = self.app_state.clone();
        let user_id = user_id.to_string();
        let chat_id = source_chat_id.to_string();
        tokio::spawn(async move {
            let message_id = match state.chat_service
                .find_executing_message_for_chat(&chat_id)
                .await
            {
                Ok(Some(msg)) => msg.id,
                Ok(None) => {
                    tracing::warn!(chat_id = %chat_id, "No executing message found for child task resume");
                    return;
                }
                Err(e) => {
                    tracing::error!(error = %e, chat_id = %chat_id, "Failed to find executing message");
                    return;
                }
            };
            resume_or_notify(&state, &user_id, &chat_id, &message_id).await;
        });
    }

    pub fn broadcast_task_status(&self, task: &Task, status: &str, summary: Option<&str>) {
        self.app_state.broadcast_service.broadcast_task_update(
            &task.user_id,
            &task.id,
            status,
            &task.title,
            task.chat_id.as_deref(),
            task.kind.source_chat_id(),
            summary,
        );
    }

}

enum LifecycleAction {
    Complete {
        status: TaskStatus,
        summary: Option<String>,
        attachments: Vec<Attachment>,
    },
    Defer {
        delay_minutes: u32,
        reason: String,
    },
}

/// Check if a task executor is waiting for resolution on this chat's task.
/// If so, notify it. Otherwise, fall back to `resume_agent_loop`.
pub async fn resume_or_notify(state: &AppState, user_id: &str, chat_id: &str, message_id: &str) {
    if let Ok(Some(chat)) = state.chat_service.find_chat(chat_id).await
        && let Some(ref task_id) = chat.task_id
    {
        let notifiers = state.task_resolution_notifiers.lock().await;
        if let Some(notify) = notifiers.get(task_id) {
            notify.notify_one();
            return;
        }
    }

    if let Err(e) = crate::agent::execution::resume_agent_loop(state, user_id, chat_id, message_id).await {
        tracing::error!(error = %e, chat_id = %chat_id, "Failed to resume chat");
    }
}
