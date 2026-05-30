use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::core::error::AppError;
use frona_derive::agent_tool;

use super::browser::session::BrowserSessionManager;
use super::{InferenceContext, ToolOutput};

pub struct WebFetchTool {
    session_manager: Arc<BrowserSessionManager>,
    prompts: PromptLoader,
}

impl WebFetchTool {
    pub fn new(session_manager: Arc<BrowserSessionManager>, prompts: PromptLoader) -> Self {
        Self {
            session_manager,
            prompts,
        }
    }

    fn provider(&self) -> &str {
        "default"
    }
}

#[agent_tool]
impl WebFetchTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let url = arguments
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: url".into()))?;

        let conn = self
            .session_manager
            .connection(&ctx.user.handle, self.provider())
            .await?;

        conn.navigate(url, false)
            .await
            .map_err(|e| AppError::Browser(format!("navigate: {e}")))?;

        let _ = conn
            .wait_for_selector("body", Duration::from_secs(15))
            .await;

        let markdown = conn
            .get_markdown(1, 100_000)
            .await
            .map_err(|e| AppError::Browser(format!("get_markdown: {e}")))?;

        Ok(ToolOutput::text(markdown.content))
    }

    async fn cleanup(&self) -> Result<(), AppError> {
        Ok(())
    }
}
