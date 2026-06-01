use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::core::error::AppError;
use crate::credential::vault::service::VaultService;

use crate::inference::tool_call::{MessageTool, ToolStatus};
use frona_derive::agent_tool;

use super::{InferenceContext, ToolOutput};

pub struct NotifyHumanTool {
    vault_service: VaultService,
    prompts: PromptLoader,
}

impl NotifyHumanTool {
    pub fn new(vault_service: VaultService, prompts: PromptLoader) -> Self {
        Self { vault_service, prompts }
    }
}

#[agent_tool(files("ask_user_question", "request_user_takeover"))]
impl NotifyHumanTool {
    async fn execute(&self, tool_name: &str, arguments: Value, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        match tool_name {
            "request_user_takeover" => {
                let reason = arguments
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("User intervention needed")
                    .to_string();
                let debugger_url = self
                    .vault_service
                    .list_credentials(&ctx.user.id)
                    .await
                    .ok()
                    .and_then(|creds| creds.into_iter().next())
                    .map(|c| format!("/api/browser/debugger/{}", c.id))
                    .unwrap_or_default();

                let json = serde_json::json!({
                    "tool_type": "HumanInTheLoop",
                    "reason": reason,
                    "debugger_url": debugger_url,
                });

                Ok(ToolOutput::text(json.to_string())
                    .with_tool_data(MessageTool::HumanInTheLoop {
                        reason,
                        debugger_url,
                        status: ToolStatus::Pending,
                        response: None,
                    })
                    .as_pending_external())
            }
            "ask_user_question" => {
                let question = arguments
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let options: Vec<String> = arguments
                    .get("options")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                let json = serde_json::json!({
                    "tool_type": "Question",
                    "question": question,
                    "options": options,
                });

                Ok(ToolOutput::text(json.to_string())
                    .with_tool_data(MessageTool::Question {
                        question,
                        options,
                        status: ToolStatus::Pending,
                        response: None,
                    })
                    .as_pending_external())
            }
            _ => Err(AppError::Tool(format!(
                "Unknown notify_human sub-tool: {tool_name}"
            ))),
        }
    }
}
