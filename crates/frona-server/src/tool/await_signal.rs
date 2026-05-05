use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::agent::signal::{SignalService, Watch};
use crate::agent::task::service::TaskService;
use crate::core::error::AppError;
use crate::tool::{
    AgentTool, InferenceContext, ToolDefinition, ToolOutput, load_tool_definition,
};

pub struct AwaitSignalTool {
    pub task_service: TaskService,
    pub signal_service: Arc<SignalService>,
    pub prompts: PromptLoader,
    pub default_max_evaluations: u32,
}

impl AwaitSignalTool {
    pub fn new(
        task_service: TaskService,
        signal_service: Arc<SignalService>,
        prompts: PromptLoader,
        default_max_evaluations: u32,
    ) -> Self {
        Self {
            task_service,
            signal_service,
            prompts,
            default_max_evaluations,
        }
    }
}

#[async_trait]
impl AgentTool for AwaitSignalTool {
    fn name(&self) -> &str {
        "await_signal"
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        load_tool_definition(&self.prompts, "tools/await_signal.md")
            .map(|d| vec![d])
            .unwrap_or_default()
    }

    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: title".into()))?
            .trim()
            .to_string();
        if title.is_empty() {
            return Err(AppError::Validation("title must not be empty".into()));
        }
        if title.len() > 256 {
            return Err(AppError::Validation(
                "title must be at most 256 bytes".into(),
            ));
        }
        let description = arguments
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: description".into()))?
            .to_string();
        let tags = parse_string_array(&arguments, "tags")?;
        let expected_channels = parse_string_array(&arguments, "expected_channels")?;
        let expected_contacts = parse_string_array(&arguments, "expected_contacts")?;
        let expires_at = resolve_expires_at(&arguments)?;
        let resume_parent = arguments
            .get("resume_parent")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let max_evaluations = arguments
            .get("max_evaluations")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(MAX_EVALUATIONS_CAP) as u32)
            .unwrap_or(self.default_max_evaluations);

        if tags.is_empty() && expected_channels.is_empty() && expected_contacts.is_empty() {
            return Err(AppError::Validation(
                "await_signal requires at least one of: tags, expected_channels, expected_contacts"
                    .into(),
            ));
        }

        let task = self
            .task_service
            .create_signal(
                &ctx.user.id,
                ctx.agent.id.clone(),
                ctx.chat.id.clone(),
                title,
                description,
                resume_parent,
                tags,
                expected_channels,
                expected_contacts,
                expires_at,
                max_evaluations,
            )
            .await?;

        if let Some(watch) = Watch::from_task(&task) {
            self.signal_service.register(watch).await;
        }

        let body = format!(
            "Signal task created (id: {}). The current chat will resume when a matching candidate arrives and you confirm it via complete_task in the signal task's chat.",
            task.id
        );
        Ok(ToolOutput::text(body))
    }
}

const MAX_EVALUATIONS_CAP: u64 = 1_000;

fn resolve_expires_at(arguments: &Value) -> Result<Option<DateTime<Utc>>, AppError> {
    let has_minutes = arguments.get("expires_in_minutes").is_some();
    let has_absolute = arguments.get("expires_at").is_some();

    if has_minutes && has_absolute {
        return Err(AppError::Validation(
            "Cannot use both expires_in_minutes and expires_at.".into(),
        ));
    }

    if has_minutes {
        let mins = arguments
            .get("expires_in_minutes")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                AppError::Validation("expires_in_minutes must be a non-negative integer".into())
            })?;
        if mins == 0 {
            return Err(AppError::Validation(
                "expires_in_minutes must be greater than 0".into(),
            ));
        }
        return Ok(Some(Utc::now() + Duration::minutes(mins as i64)));
    }

    let Some(value) = arguments.get("expires_at") else {
        return Ok(None);
    };
    super::parse_run_at(value)
        .map_err(|e| AppError::Validation(e.to_string().replace("run_at", "expires_at")))
}

fn parse_string_array(args: &Value, key: &str) -> Result<Vec<String>, AppError> {
    let Some(arr) = args.get(key) else {
        return Ok(Vec::new());
    };
    let arr = arr
        .as_array()
        .ok_or_else(|| AppError::Validation(format!("{key} must be an array of strings")))?;
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        let s = v
            .as_str()
            .ok_or_else(|| AppError::Validation(format!("{key} entries must be strings")))?;
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation(format!(
                "{key} entries must be non-empty"
            )));
        }
        out.push(trimmed.to_string());
    }
    Ok(out)
}

