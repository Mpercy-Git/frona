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

/// Accepts unix timestamp or naive ISO 8601 (interpreted in `tz`). Rejects
/// offset-bearing strings — the agent must use naive + `timezone` parameter.
pub fn parse_run_at(value: &Value, tz: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
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
                Some(parse_naive_run_at(s, tz)?)
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

fn parse_naive_run_at(s: &str, tz: &str) -> Result<chrono::DateTime<chrono::Utc>, AppError> {
    // RFC 3339 parse succeeds = offset-bearing. Reject — bypasses per-task TZ.
    if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
        return Err(AppError::Validation(format!(
            "run_at '{}' includes an explicit UTC offset. Use a naive ISO 8601 form like '2026-05-20T22:00:00' (interpreted in the user's local timezone) and set the optional `timezone` parameter only if the user names a different zone.",
            s
        )));
    }

    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M"))
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .map_err(|e| {
            AppError::Validation(format!(
                "Invalid run_at datetime '{}': {}. Use naive ISO 8601 form like '2026-05-20T22:00:00' (no 'Z', no offset).",
                s, e
            ))
        })?;

    let parsed_tz: chrono_tz::Tz = tz.parse().map_err(|e| {
        AppError::Validation(format!(
            "Invalid timezone '{}': {}. Use an IANA name like 'America/Los_Angeles', 'Asia/Tokyo', or 'UTC'.",
            tz, e
        ))
    })?;

    use chrono::TimeZone;
    let resolved = parsed_tz
        .from_local_datetime(&naive)
        .single()
        .ok_or_else(|| {
            AppError::Validation(format!(
                "run_at '{}' is ambiguous or invalid in timezone '{}' (likely a DST transition). Pick a different time.",
                s, tz
            ))
        })?;

    Ok(resolved.with_timezone(&chrono::Utc))
}

/// Resolve a `run_at` datetime from arguments, supporting both `run_at` and `delay_minutes`.
/// `delay_minutes` takes precedence over `run_at` if both are provided.
pub fn resolve_run_at(arguments: &Value, tz: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    if let Some(delay) = arguments.get("delay_minutes").and_then(|v| v.as_u64()) {
        if delay == 0 {
            return Err(AppError::Validation("delay_minutes must be greater than 0".into()));
        }
        return Ok(Some(chrono::Utc::now() + chrono::Duration::minutes(delay as i64)));
    }

    match arguments.get("run_at") {
        Some(v) => parse_run_at(v, tz),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_run_at_unix_timestamp_number() {
        let dt = parse_run_at(&json!(4_000_000_000_i64), "UTC")
            .unwrap()
            .unwrap();
        assert_eq!(dt.timestamp(), 4_000_000_000);
    }

    #[test]
    fn parse_run_at_unix_timestamp_string() {
        let dt = parse_run_at(&json!("4000000000"), "UTC")
            .unwrap()
            .unwrap();
        assert_eq!(dt.timestamp(), 4_000_000_000);
    }

    #[test]
    fn parse_run_at_naive_in_la_winter() {
        // 22:00 on 2030-01-15 in LA (PST = UTC-8) → 06:00 UTC on 01-16
        let dt = parse_run_at(&json!("2030-01-15T22:00:00"), "America/Los_Angeles")
            .unwrap()
            .unwrap();
        assert_eq!(dt.to_rfc3339(), "2030-01-16T06:00:00+00:00");
    }

    #[test]
    fn parse_run_at_naive_in_la_summer() {
        // 22:00 on 2030-07-15 in LA (PDT = UTC-7) → 05:00 UTC on 07-16
        let dt = parse_run_at(&json!("2030-07-15T22:00:00"), "America/Los_Angeles")
            .unwrap()
            .unwrap();
        assert_eq!(dt.to_rfc3339(), "2030-07-16T05:00:00+00:00");
    }

    #[test]
    fn parse_run_at_naive_in_tokyo() {
        // 06:00 on 2030-05-20 in Tokyo (JST = UTC+9) → 21:00 UTC on 05-19
        let dt = parse_run_at(&json!("2030-05-20T06:00:00"), "Asia/Tokyo")
            .unwrap()
            .unwrap();
        assert_eq!(dt.to_rfc3339(), "2030-05-19T21:00:00+00:00");
    }

    #[test]
    fn parse_run_at_rejects_explicit_offset() {
        let err = parse_run_at(&json!("2030-05-20T22:00:00-04:00"), "UTC").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("explicit UTC offset"),
            "expected hint about explicit offset, got: {msg}"
        );
    }

    #[test]
    fn parse_run_at_rejects_z_suffix() {
        let err = parse_run_at(&json!("2030-05-20T22:00:00Z"), "UTC").unwrap_err();
        assert!(err.to_string().contains("explicit UTC offset"));
    }

    #[test]
    fn parse_run_at_rejects_past_time() {
        let err = parse_run_at(&json!("2000-01-01T00:00:00"), "UTC").unwrap_err();
        assert!(err.to_string().contains("future"));
    }

    #[test]
    fn parse_run_at_invalid_tz_rejected() {
        let err = parse_run_at(&json!("2030-05-20T22:00:00"), "Mars/Olympus").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Invalid timezone"), "got: {msg}");
    }

    #[test]
    fn parse_run_at_accepts_space_separator() {
        let dt = parse_run_at(&json!("2030-05-20 22:00:00"), "UTC")
            .unwrap()
            .unwrap();
        assert_eq!(dt.to_rfc3339(), "2030-05-20T22:00:00+00:00");
    }

    #[test]
    fn resolve_run_at_delay_minutes_takes_precedence() {
        let args = json!({"delay_minutes": 5, "run_at": "2030-05-20T22:00:00"});
        let dt = resolve_run_at(&args, "UTC").unwrap().unwrap();
        // Should be ~5min from now, not 2030 — delay_minutes wins.
        let delta = (dt - chrono::Utc::now()).num_minutes();
        assert!((4..=6).contains(&delta), "expected ~5 min from now, got {delta}");
    }

    #[test]
    fn resolve_run_at_delay_minutes_zero_rejected() {
        let args = json!({"delay_minutes": 0});
        assert!(resolve_run_at(&args, "UTC").is_err());
    }
}
