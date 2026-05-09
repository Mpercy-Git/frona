use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::agent::service::AgentService;
use crate::agent::task::executor::TaskExecutor;
use crate::agent::task::models::CreateTaskRequest;
use crate::agent::task::service::TaskService;
use crate::chat::broadcast::BroadcastService;
use crate::core::error::AppError;
use crate::policy::models::PolicyAction;
use crate::policy::service::PolicyService;
use frona_derive::agent_tool;

use super::{InferenceContext, ToolOutput};

pub fn parse_cron(expression: &str) -> Result<cron::Schedule, AppError> {
    let seven_field = format!("0 {} *", expression);
    cron::Schedule::from_str(&seven_field)
        .map_err(|e| AppError::Validation(format!("Invalid cron expression '{}': {}", expression, e)))
}

pub fn next_cron_occurrence(expression: &str) -> Result<DateTime<Utc>, AppError> {
    let schedule = parse_cron(expression)?;
    schedule
        .upcoming(Utc)
        .next()
        .ok_or_else(|| AppError::Validation("Cron expression has no future occurrences".into()))
}

pub struct TaskTool {
    task_service: TaskService,
    agent_service: AgentService,
    task_executor: Arc<TaskExecutor>,
    broadcast_service: BroadcastService,
    policy_service: PolicyService,
    prompts: PromptLoader,
}

impl TaskTool {
    pub fn new(
        task_service: TaskService,
        agent_service: AgentService,
        task_executor: Arc<TaskExecutor>,
        broadcast_service: BroadcastService,
        policy_service: PolicyService,
        prompts: PromptLoader,
    ) -> Self {
        Self {
            task_service,
            agent_service,
            task_executor,
            broadcast_service,
            policy_service,
            prompts,
        }
    }

    async fn handle_create(&self, arguments: Value, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let user_id = &ctx.user.id;
        let agent_id = &ctx.agent.id;
        let chat_id = &ctx.chat.id;
        let space_id = ctx.chat.space_id.clone();

        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing 'title' parameter".into()))?;

        let instruction = arguments
            .get("instruction")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing 'instruction' parameter".into()))?;

        let target_agent_name = arguments.get("target_agent").and_then(|v| v.as_str());
        let process_result = arguments
            .get("process_result")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cron_expression = arguments.get("cron_expression").and_then(|v| v.as_str());
        let has_delay_minutes = arguments.get("delay_minutes").and_then(|v| v.as_u64()).is_some();
        let has_run_at = arguments.get("run_at").is_some();

        let (target_agent, is_self) = match target_agent_name {
            Some(name) => {
                let agent = self
                    .agent_service
                    .find_by_name(user_id, name)
                    .await?
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "Agent '{}' not found. Check <available_agents> for valid agent names.",
                            name
                        ))
                    })?;
                if !agent.enabled {
                    return Err(AppError::Validation(format!(
                        "Agent '{}' is disabled and cannot accept tasks.",
                        agent.name
                    )));
                }
                let is_self = agent.id == *agent_id;
                (agent, is_self)
            }
            None => (ctx.agent.clone(), true),
        };

        if !is_self {
            let decision = self
                .policy_service
                .authorize(
                    user_id,
                    &ctx.agent,
                    PolicyAction::DelegateTask {
                        target_agent_id: target_agent.id.clone(),
                    },
                )
                .await?;
            if decision.is_denied() {
                return Ok(ToolOutput::error(format!(
                    "Authorization denied: agent '{}' is not permitted to delegate tasks to '{}'.",
                    ctx.agent.name, target_agent.name
                )));
            }
        }

        if is_self && process_result {
            return Err(AppError::Validation(
                "Cannot use process_result on a task targeting yourself.".into(),
            ));
        }
        if cron_expression.is_some() && process_result {
            return Err(AppError::Validation(
                "Cannot use process_result on a recurring task.".into(),
            ));
        }
        if cron_expression.is_some() && has_delay_minutes {
            return Err(AppError::Validation(
                "Cannot use delay_minutes with cron_expression. Use run_at to set the first cron run time.".into(),
            ));
        }
        if has_delay_minutes && has_run_at {
            return Err(AppError::Validation(
                "Cannot use both delay_minutes and run_at.".into(),
            ));
        }

        if let Some(cron_expr) = cron_expression {
            self.handle_create_cron(
                user_id, agent_id, chat_id, &target_agent, is_self, title, instruction, cron_expr, &arguments,
            )
            .await
        } else {
            self.handle_create_oneoff(
                user_id, agent_id, chat_id, space_id, &target_agent, is_self, process_result, title, instruction,
                &arguments,
            )
            .await
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_create_cron(
        &self,
        user_id: &str,
        agent_id: &str,
        chat_id: &str,
        target_agent: &crate::agent::models::Agent,
        is_self: bool,
        title: &str,
        instruction: &str,
        cron_expression: &str,
        arguments: &Value,
    ) -> Result<ToolOutput, AppError> {
        let run_at = super::resolve_run_at(arguments)?;

        parse_cron(cron_expression)?;
        let next_run_at = match run_at {
            Some(dt) => dt,
            None => next_cron_occurrence(cron_expression)?,
        };

        let source_agent_id = if is_self {
            None
        } else {
            Some(agent_id.to_string())
        };

        let task = self
            .task_service
            .create_cron_template(
                user_id,
                &target_agent.id,
                title,
                instruction,
                cron_expression,
                next_run_at,
                source_agent_id,
                Some(chat_id.to_string()),
                run_at,
            )
            .await?;

        Ok(ToolOutput::text(
            serde_json::json!({
                "task_id": task.id,
                "cron_expression": cron_expression,
                "next_run_at": next_run_at.to_rfc3339(),
                "message": format!(
                    "Cron job '{}' created for {}. Next run at {}.",
                    title, target_agent.name, next_run_at.format("%Y-%m-%d %H:%M UTC")
                )
            })
            .to_string(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_create_oneoff(
        &self,
        user_id: &str,
        agent_id: &str,
        chat_id: &str,
        space_id: Option<String>,
        target_agent: &crate::agent::models::Agent,
        is_self: bool,
        process_result: bool,
        title: &str,
        instruction: &str,
        arguments: &Value,
    ) -> Result<ToolOutput, AppError> {
        let run_at = super::resolve_run_at(arguments)?;

        let source_agent_id = if is_self {
            None
        } else {
            Some(agent_id.to_string())
        };

        let req = CreateTaskRequest {
            agent_id: target_agent.id.clone(),
            space_id,
            chat_id: None,
            title: title.to_string(),
            description: Some(instruction.to_string()),
            source_agent_id,
            source_chat_id: Some(chat_id.to_string()),
            resume_parent: Some(process_result),
            run_at,
            quarantined: false,
            result_schema: None,
        };

        let task_response = self.task_service.create(user_id, req).await?;
        let task_id = task_response.id.clone();

        self.broadcast_service.broadcast_task_update(
            user_id,
            &task_id,
            "pending",
            &task_response.title,
            task_response.chat_id.as_deref(),
            Some(chat_id),
            None,
        );

        if run_at.is_none() {
            let task = self
                .task_service
                .find_by_id(&task_id)
                .await?
                .ok_or_else(|| AppError::Internal("Task just created but not found".into()))?;

            if let Err(e) = self.task_executor.spawn_execution(task).await {
                tracing::warn!(error = %e, task_id = %task_id, "Failed to spawn task execution immediately");
            }
        }

        let message = if is_self {
            match run_at {
                Some(at) => format!(
                    "Task '{}' created, deferred until {}.",
                    title,
                    at.format("%Y-%m-%d %H:%M UTC")
                ),
                None => format!("Task '{}' created and running.", title),
            }
        } else {
            match (run_at, process_result) {
                (Some(at), _) => format!(
                    "Task '{}' assigned to {}, deferred until {}.",
                    title,
                    target_agent.name,
                    at.format("%Y-%m-%d %H:%M UTC")
                ),
                (None, false) => format!(
                    "Task '{}' assigned to {}. Results will be posted to this chat when complete.",
                    title, target_agent.name
                ),
                (None, true) => format!(
                    "Task '{}' assigned to {}. You will be resumed with the result.",
                    title, target_agent.name
                ),
            }
        };

        Ok(ToolOutput::text(
            serde_json::json!({
                "task_id": task_id,
                "target_agent": target_agent.name,
                "run_at": run_at.map(|t| t.to_rfc3339()),
                "message": message,
            })
            .to_string(),
        ))
    }

    async fn handle_list(&self, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let all_tasks = self
            .task_service
            .list_active(&ctx.user.id)
            .await
            .unwrap_or_default();

        let tasks: Vec<_> = all_tasks
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.id,
                    "title": t.title,
                    "description": t.description,
                    "agent_id": t.agent_id,
                    "kind": t.kind,
                    "status": t.status,
                })
            })
            .collect();

        Ok(ToolOutput::text(
            serde_json::json!({
                "tasks": tasks,
                "count": tasks.len(),
            })
            .to_string(),
        ))
    }

    async fn handle_delete(&self, arguments: Value, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let task_id = arguments
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing 'task_id' parameter".into()))?;

        let task = self
            .task_service
            .find_by_id(task_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Task not found".into()))?;

        if task.user_id != ctx.user.id {
            return Err(AppError::Forbidden("Not your task".into()));
        }

        self.task_service.mark_cancelled(task_id).await?;

        Ok(ToolOutput::text(
            serde_json::json!({
                "message": format!("Task '{}' cancelled.", task.title)
            })
            .to_string(),
        ))
    }
}

#[agent_tool(name = "task", files("create_task", "list_tasks", "delete_task"))]
impl TaskTool {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        match tool_name {
            "create_task" => self.handle_create(arguments, ctx).await,
            "list_tasks" => self.handle_list(ctx).await,
            "delete_task" => self.handle_delete(arguments, ctx).await,
            _ => Err(AppError::Validation(format!("Unknown task tool: {}", tool_name))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn parse_cron_valid_every_minute() {
        let schedule = parse_cron("* * * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());
    }

    #[test]
    fn parse_cron_valid_daily_9am() {
        let schedule = parse_cron("0 9 * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert_eq!(next.minute(), 0);
        assert_eq!(next.hour(), 9);
    }

    #[test]
    fn parse_cron_valid_weekdays_at_noon() {
        let schedule = parse_cron("0 12 * * MON-FRI").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert_eq!(next.hour(), 12);
        assert_eq!(next.minute(), 0);
        let weekday = next.weekday().num_days_from_monday();
        assert!(weekday < 5, "Should be a weekday (Mon=0 .. Fri=4), got {weekday}");
    }

    #[test]
    fn parse_cron_valid_every_30_mins() {
        let schedule = parse_cron("*/30 * * * *").unwrap();
        let occurrences: Vec<_> = schedule.upcoming(Utc).take(4).collect();
        assert_eq!(occurrences.len(), 4);
        for occ in &occurrences {
            assert!(occ.minute() == 0 || occ.minute() == 30);
        }
    }

    #[test]
    fn parse_cron_invalid_expression() {
        let result = parse_cron("not a cron");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid cron expression"), "Error: {err}");
    }

    #[test]
    fn parse_cron_rejects_empty() {
        assert!(parse_cron("").is_err());
    }

    #[test]
    fn parse_cron_rejects_6_fields() {
        let result = parse_cron("0 0 9 * * MON");
        assert!(result.is_err());
    }

    #[test]
    fn parse_cron_rejects_3_fields() {
        let result = parse_cron("0 9 *");
        assert!(result.is_err());
    }

    #[test]
    fn next_cron_occurrence_returns_future() {
        let next = next_cron_occurrence("* * * * *").unwrap();
        assert!(next > Utc::now());
    }

    #[test]
    fn next_cron_occurrence_daily_has_correct_time() {
        let next = next_cron_occurrence("30 14 * * *").unwrap();
        assert_eq!(next.hour(), 14);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn next_cron_occurrence_multiple_calls_are_consistent() {
        let a = next_cron_occurrence("0 0 * * *").unwrap();
        let b = next_cron_occurrence("0 0 * * *").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn next_cron_occurrence_invalid_returns_error() {
        assert!(next_cron_occurrence("invalid").is_err());
    }
}
