pub mod await_signal;
pub mod browser;
pub mod manager;
pub mod cli;
pub mod create_agent;
pub mod manage_policy;
pub mod heartbeat;
pub mod manage_service;
pub mod notify_human;
pub mod produce_file;
pub mod registry;
pub mod memory;
pub mod report_signal;
pub mod request_credentials;
pub mod task;
pub mod send_message;
pub mod annotate;
pub mod task_control;
pub mod update_identity;
pub mod voice;
pub mod web_fetch;
pub mod web_search;
pub mod mcp;
pub mod provider;
pub mod sandbox;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::error::AppError;

pub use crate::inference::request::InferenceContext;

use crate::agent::prompt::PromptLoader;

/// Parse a `run_at` argument that can be a unix timestamp (number or string) or ISO 8601 datetime.
/// Returns an error if the value is in the past.
pub fn parse_run_at(value: &Value) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let dt = match value {
        Value::Number(n) => {
            let ts = n.as_i64()
                .ok_or_else(|| AppError::Validation("Invalid run_at timestamp".into()))?;
            Some(chrono::DateTime::from_timestamp(ts, 0)
                .ok_or_else(|| AppError::Validation("Invalid run_at timestamp".into()))?)
        }
        Value::String(s) => {
            if let Ok(ts) = s.parse::<i64>() {
                Some(chrono::DateTime::from_timestamp(ts, 0)
                    .ok_or_else(|| AppError::Validation("Invalid run_at timestamp".into()))?)
            } else {
                Some(s.parse::<chrono::DateTime<chrono::Utc>>()
                    .map_err(|e| AppError::Validation(format!("Invalid run_at datetime: {}", e)))?)
            }
        }
        _ => None,
    };

    if let Some(at) = dt
        && at <= chrono::Utc::now()
    {
        return Err(AppError::Validation("run_at must be in the future".into()));
    }

    Ok(dt)
}

/// Resolve a `run_at` datetime from arguments, supporting both `run_at` and `delay_minutes`.
/// `delay_minutes` takes precedence over `run_at` if both are provided.
pub fn resolve_run_at(arguments: &Value) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    if let Some(delay) = arguments.get("delay_minutes").and_then(|v| v.as_u64()) {
        if delay == 0 {
            return Err(AppError::Validation("delay_minutes must be greater than 0".into()));
        }
        return Ok(Some(chrono::Utc::now() + chrono::Duration::minutes(delay as i64)));
    }

    match arguments.get("run_at") {
        Some(v) => parse_run_at(v),
        None => Ok(None),
    }
}

pub fn is_tool_available(state: &crate::core::state::AppState, tool_name: &str) -> bool {
    match tool_name {
        "voice_call" => state.voice_provider.is_some(),
        _ => true,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub id: String,
    pub provider_id: String,
    pub description: String,
    pub parameters: Value,
}

pub struct ImageData {
    pub bytes: Vec<u8>,
    pub media_type: String,
}

pub struct ToolOutput {
    text: String,
    images: Vec<ImageData>,
    attachments: Vec<crate::storage::Attachment>,
    tool_data: Option<crate::inference::tool_call::MessageTool>,
    system_prompt: Option<String>,
    pending_external: bool,
    success: bool,
}

impl ToolOutput {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            images: Vec::new(),
            attachments: Vec::new(),
            tool_data: None,
            system_prompt: None,
            pending_external: false,
            success: true,
        }
    }

    pub fn error(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            images: Vec::new(),
            attachments: Vec::new(),
            tool_data: None,
            system_prompt: None,
            pending_external: false,
            success: false,
        }
    }

    pub fn mixed(text: impl Into<String>, images: Vec<ImageData>) -> Self {
        Self {
            text: text.into(),
            images,
            attachments: Vec::new(),
            tool_data: None,
            system_prompt: None,
            pending_external: false,
            success: true,
        }
    }

    pub fn with_attachment(mut self, a: crate::storage::Attachment) -> Self {
        self.attachments.push(a);
        self
    }

    pub fn with_tool_data(mut self, td: crate::inference::tool_call::MessageTool) -> Self {
        self.tool_data = Some(td);
        self
    }

    pub fn with_system_prompt(mut self, s: impl Into<String>) -> Self {
        self.system_prompt = Some(s.into());
        self
    }

    pub fn text_content(&self) -> &str {
        &self.text
    }

    pub fn images(&self) -> &[ImageData] {
        &self.images
    }

    pub fn attachments(&self) -> &[crate::storage::Attachment] {
        &self.attachments
    }

    pub fn tool_data(&self) -> Option<&crate::inference::tool_call::MessageTool> {
        self.tool_data.as_ref()
    }

    pub fn as_pending_external(mut self) -> Self {
        self.pending_external = true;
        self
    }

    pub fn is_pending_external(&self) -> bool {
        self.pending_external
    }

    pub fn is_success(&self) -> bool {
        self.success
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }
}

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn definitions(&self) -> Vec<ToolDefinition>;
    fn definition_vars(&self) -> Vec<(&str, &str)> {
        vec![]
    }
    async fn execute(&self, tool_name: &str, arguments: Value, ctx: &InferenceContext) -> Result<ToolOutput, AppError>;
    async fn cleanup(&self) -> Result<(), AppError> {
        Ok(())
    }
}

fn parse_frontmatter(raw: &str) -> Option<(Value, String)> {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_first = &trimmed[3..];
    let end = after_first.find("---")?;
    let yaml_str = &after_first[..end];
    let body = after_first[end + 3..].trim().to_string();
    let yaml: Value = serde_yaml::from_str(yaml_str).ok()?;
    Some((yaml, body))
}

fn build_parameters_json(yaml: &Value) -> Value {
    let params = yaml.get("parameters").cloned().unwrap_or(Value::Null);
    let required = yaml.get("required").cloned().unwrap_or(Value::Null);

    let properties: Value = if let Value::Object(map) = &params {
        let mut props = serde_json::Map::new();
        for (key, schema) in map {
            props.insert(key.clone(), serde_json::to_value(schema).unwrap_or(Value::Null));
        }
        Value::Object(props)
    } else {
        Value::Object(serde_json::Map::new())
    };

    let mut result = serde_json::json!({
        "type": "object",
        "properties": properties,
    });

    if let Value::Array(arr) = &required {
        let req: Vec<Value> = arr.iter().map(|v| {
            if let Value::String(s) = v {
                Value::String(s.clone())
            } else {
                v.clone()
            }
        }).collect();
        result["required"] = Value::Array(req);
    }

    result
}

pub fn load_tool_definition(prompts: &PromptLoader, path: &str) -> Option<ToolDefinition> {
    load_tool_definition_with_vars(prompts, path, &[])
}

pub fn load_tool_definition_with_vars(prompts: &PromptLoader, path: &str, vars: &[(&str, &str)]) -> Option<ToolDefinition> {
    let raw = prompts.read_with_vars(path, vars)?;
    let (yaml, body) = parse_frontmatter(&raw)?;
    let id = yaml.get("id")?.as_str()?.to_string();
    let provider_id = yaml
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parameters = build_parameters_json(&yaml);
    Some(ToolDefinition {
        id,
        provider_id,
        description: body,
        parameters,
    })
}
