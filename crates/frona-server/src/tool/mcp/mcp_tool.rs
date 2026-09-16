use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::Value;

use crate::core::error::AppError;
use crate::tool::{AgentTool, InferenceContext, ToolDefinition, ToolOutput};

use super::manager::McpManager;
use super::models::CachedMcpTool;

pub struct McpTool {
    manager: Arc<McpManager>,
    owner_name: String,
    slug: String,
    /// The live tool list shared with `McpClient`, not a copy. A gated MCP server
    /// advertises a subset at handshake and announces the rest later (`tools/list_changed`)
    /// once unlocked - e.g. a server whose entry tool hands out a
    /// "read the guide first" acknowledgement key. Holding the startup snapshot meant
    /// the client learned about those tools and the agent never did, and no restart
    /// fixed it because the handshake happens while the server is still locked.
    tool_cache: Arc<RwLock<Vec<CachedMcpTool>>>,
}

impl McpTool {
    pub fn new(
        manager: Arc<McpManager>,
        slug: &str,
        tool_cache: Arc<RwLock<Vec<CachedMcpTool>>>,
    ) -> Self {
        Self {
            manager,
            owner_name: format!("mcp__{slug}"),
            slug: slug.to_string(),
            tool_cache,
        }
    }
}

#[async_trait]
impl AgentTool for McpTool {
    fn name(&self) -> &str {
        &self.owner_name
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        self.tool_cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|c| ToolDefinition {
                id: format!("mcp__{}__{}", self.slug, c.name),
                provider_id: format!("mcp:{}", self.slug),
                description: c.description.clone(),
                parameters: c.input_schema.clone(),
            })
            .collect()
    }

    async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        _ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let server_id = self
            .manager
            .server_for_tool(tool_name)
            .await
            .ok_or_else(|| {
                AppError::Tool(format!("no running MCP server exposes tool {tool_name}"))
            })?;

        let bare_name = tool_name.split("__").nth(2).unwrap_or(tool_name);

        let result = self.manager.call(&server_id, bare_name, arguments).await?;

        let is_error = result.is_error.unwrap_or(false);
        let text = result
            .content
            .iter()
            .filter_map(|c| match c {
                rmcp::model::ContentBlock::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        if is_error {
            Ok(ToolOutput::error(text))
        } else {
            Ok(ToolOutput::text(text))
        }
    }
}
