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
    server_id: String,
    /// Whether the server declared `resources` at initialize. Gates the two
    /// synthetic tools below, so a tools-only server grows nothing.
    supports_resources: bool,
}

/// Bare names of the synthetic tools that expose MCP resources. A server's own
/// tool of the same name always wins - these fill a gap, they never shadow.
pub const LIST_RESOURCES: &str = "list_resources";
pub const READ_RESOURCE: &str = "read_resource";

impl McpTool {
    pub fn new(
        manager: Arc<McpManager>,
        slug: &str,
        tool_cache: Arc<RwLock<Vec<CachedMcpTool>>>,
        server_id: String,
        supports_resources: bool,
    ) -> Self {
        Self {
            manager,
            owner_name: format!("mcp__{slug}"),
            slug: slug.to_string(),
            tool_cache,
            server_id,
            supports_resources,
        }
    }

    fn has_real_tool(&self, name: &str) -> bool {
        self.tool_cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|c| c.name == name)
    }

    async fn run_list_resources(&self) -> Result<ToolOutput, AppError> {
        let resources = self.manager.list_resources(&self.server_id).await?;
        if resources.is_empty() {
            return Ok(ToolOutput::text("This server exposes no resources."));
        }
        let mut out = String::new();
        for r in &resources {
            out.push_str(&r.uri);
            let label = r.title.as_deref().unwrap_or(r.name.as_str());
            if !label.is_empty() {
                out.push_str(&format!("  {label}"));
            }
            if let Some(mime) = &r.mime_type {
                out.push_str(&format!("  [{mime}]"));
            }
            if let Some(desc) = &r.description {
                out.push_str(&format!("\n    {desc}"));
            }
            out.push('\n');
        }
        Ok(ToolOutput::text(out))
    }

    async fn run_read_resource(&self, arguments: Value) -> Result<ToolOutput, AppError> {
        let uri = arguments
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("read_resource needs a `uri`".to_string()))?;

        let result = self.manager.read_resource(&self.server_id, uri).await?;
        let mut out = String::new();
        for content in &result.contents {
            match content {
                rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
                    out.push_str(text);
                    out.push('\n');
                }
                // Binary payloads are base64 in the wire format. Pasting that into a
                // transcript burns context and tells the agent nothing, so say what it
                // is instead.
                rmcp::model::ResourceContents::BlobResourceContents {
                    uri,
                    mime_type,
                    blob,
                    ..
                } => {
                    let kind = mime_type.as_deref().unwrap_or("application/octet-stream");
                    out.push_str(&format!(
                        "[binary resource {uri} ({kind}), {} base64 chars - not shown]\n",
                        blob.len()
                    ));
                }
                // `ResourceContents` is #[non_exhaustive]: a future rmcp may carry a
                // shape this build has no name for. Say so rather than drop it
                // silently, which would read as an empty resource.
                other => {
                    out.push_str(&format!(
                        "[resource content of a kind this build does not understand: {other:?}]\n"
                    ));
                }
            }
        }
        if out.is_empty() {
            out.push_str("(resource is empty)");
        }
        Ok(ToolOutput::text(out))
    }

    fn synthetic(&self, name: &str, description: &str, parameters: Value) -> ToolDefinition {
        ToolDefinition {
            id: format!("mcp__{}__{}", self.slug, name),
            provider_id: format!("mcp:{}", self.slug),
            description: description.to_string(),
            parameters,
        }
    }
}

#[async_trait]
impl AgentTool for McpTool {
    fn name(&self) -> &str {
        &self.owner_name
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        let cached = self.tool_cache.read().unwrap_or_else(|e| e.into_inner());
        let mut defs: Vec<ToolDefinition> = cached
            .iter()
            .map(|c| ToolDefinition {
                id: format!("mcp__{}__{}", self.slug, c.name),
                provider_id: format!("mcp:{}", self.slug),
                description: c.description.clone(),
                parameters: c.input_schema.clone(),
            })
            .collect();

        // Resources are a first-class half of MCP that has no tool of its own, so a
        // server answering with a `uri` hands the agent something it otherwise cannot
        // dereference. Expose the two calls as tools on the same server namespace, so
        // policy, the registry and the bridge all treat them like any other MCP tool.
        if self.supports_resources {
            let taken = |name: &str| cached.iter().any(|c| c.name == name);
            if !taken(LIST_RESOURCES) {
                defs.push(self.synthetic(
                    LIST_RESOURCES,
                    "List the resources this MCP server exposes, with each one's URI, name                      and description. Resources hold content the server publishes for reading                      (guides, schemas, files) rather than actions to call.",
                    serde_json::json!({"type": "object", "properties": {}}),
                ));
            }
            if !taken(READ_RESOURCE) {
                defs.push(self.synthetic(
                    READ_RESOURCE,
                    "Read one resource from this MCP server by URI and return its contents.                      Use it on any URI the server hands back (for example a `skill://` or                      `file://` link in another tool's result).",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "uri": {
                                "type": "string",
                                "description": "Resource URI, exactly as the server gave it."
                            }
                        },
                        "required": ["uri"]
                    }),
                ));
            }
        }
        defs
    }

    async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        _ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let bare_name = tool_name.split("__").nth(2).unwrap_or(tool_name);

        // Mirror `definitions`: only handle these when the server did not supply a
        // real tool of the same name, or we would shadow it.
        if self.supports_resources && !self.has_real_tool(bare_name) {
            match bare_name {
                LIST_RESOURCES => return self.run_list_resources().await,
                READ_RESOURCE => return self.run_read_resource(arguments).await,
                _ => {}
            }
        }

        let server_id = self
            .manager
            .server_for_tool(tool_name)
            .await
            .ok_or_else(|| {
                AppError::Tool(format!("no running MCP server exposes tool {tool_name}"))
            })?;

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
