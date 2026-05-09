use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::core::error::AppError;
use crate::core::state::AppState;

use super::{AgentTool, InferenceContext, ToolDefinition, ToolOutput};

pub struct AgentToolRegistry {
    tools: HashMap<String, Arc<dyn AgentTool>>,
    tool_name_to_owner: HashMap<String, String>,
    definitions: Vec<ToolDefinition>,
    mcp_bridge_mode: bool,
}

impl Default for AgentToolRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

impl AgentToolRegistry {
    pub fn new(
        tools: HashMap<String, Arc<dyn AgentTool>>,
        tool_name_to_owner: HashMap<String, String>,
        definitions: Vec<ToolDefinition>,
        mcp_bridge_mode: bool,
    ) -> Self {
        Self {
            tools,
            tool_name_to_owner,
            definitions,
            mcp_bridge_mode,
        }
    }

    pub fn empty() -> Self {
        Self {
            tools: HashMap::new(),
            tool_name_to_owner: HashMap::new(),
            definitions: Vec::new(),
            mcp_bridge_mode: false,
        }
    }

    pub fn mcp_bridge_mode(&self) -> bool {
        self.mcp_bridge_mode
    }

    /// Used by signal-mode inference to restrict the agent's blast radius —
    /// even if Cedar permits a tool generally, the in-mode registry hides it.
    pub fn restrict_to(&mut self, allowed: &[&str]) {
        let allow_set: std::collections::HashSet<&str> = allowed.iter().copied().collect();
        self.definitions.retain(|d| allow_set.contains(d.id.as_str()));
        let surviving_owners: std::collections::HashSet<String> =
            self.definitions.iter().filter_map(|d| {
                self.tool_name_to_owner.get(&d.id).cloned()
            }).collect();
        self.tool_name_to_owner.retain(|tool_id, _| allow_set.contains(tool_id.as_str()));
        self.tools.retain(|owner, _| surviving_owners.contains(owner));
    }

    pub fn register(&mut self, tool: Arc<dyn AgentTool>) {
        let owner_name = tool.name().to_string();
        for def in tool.definitions() {
            self.tool_name_to_owner
                .insert(def.id.clone(), owner_name.clone());
            self.definitions.push(def);
        }
        self.tools.insert(owner_name, tool);
    }

    pub async fn execute(&self, tool_name: &str, arguments: Value, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let owner = self
            .tool_name_to_owner
            .get(tool_name)
            .ok_or_else(|| AppError::Tool(format!("Unknown tool: {tool_name}")))?;

        let tool = self
            .tools
            .get(owner)
            .ok_or_else(|| AppError::Tool(format!("Tool owner not found: {owner}")))?;

        tool.execute(tool_name, arguments, ctx).await
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    pub fn tool_groups(&self) -> Vec<String> {
        let mut groups: std::collections::HashSet<String> = std::collections::HashSet::new();
        for def in &self.definitions {
            if !def.provider_id.is_empty() {
                groups.insert(def.provider_id.clone());
            }
        }
        let mut sorted: Vec<String> = groups.into_iter().collect();
        sorted.sort();
        sorted
    }
}

pub async fn build_agent_summaries(
    state: &AppState,
    user_id: &str,
    current_agent_id: &str,
) -> Vec<(String, String)> {
    let current_agent = match state.agent_service.find_by_id(current_agent_id).await {
        Ok(Some(agent)) => agent,
        _ => return Vec::new(),
    };

    let agents = match state.agent_service.list(user_id).await {
        Ok(agents) => agents,
        Err(_) => return Vec::new(),
    };

    let mut summaries = Vec::new();
    for target in &agents {
        if target.id == current_agent_id || !target.enabled {
            continue;
        }
        let decision = state
            .policy_service
            .authorize(
                user_id,
                &current_agent,
                crate::policy::models::PolicyAction::DelegateTask {
                    target_agent_id: target.id.clone(),
                },
            )
            .await;
        if decision.is_ok_and(|d| d.allowed) {
            summaries.push((target.name.clone(), target.description.clone()));
        }
    }

    summaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockTool;

    #[async_trait]
    impl AgentTool for MockTool {
        fn name(&self) -> &str {
            "mock"
        }

        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                id: "mock_action".to_string(),
                provider_id: String::new(),
                description: "A mock action".to_string(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            }]
        }

        async fn execute(&self, tool_name: &str, _arguments: Value, _ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
            Ok(ToolOutput::text(format!("executed {tool_name}")))
        }
    }

    fn mock_context() -> InferenceContext {
        let broadcast = crate::chat::broadcast::BroadcastService::new();
        let event_sender = broadcast.create_event_sender("test-user", "test-chat", None);
        InferenceContext::new(
            crate::auth::User {
                id: "test-user".into(),
                username: "testuser".into(),
                email: "test@test.com".into(),
                name: "Test".into(),
                password_hash: String::new(),
                timezone: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            crate::agent::models::Agent {
                id: "test-agent".into(),
                user_id: Some("test-user".into()),
                name: "Test Agent".into(),
                description: String::new(),
                model_group: "primary".into(),
                enabled: true,
                skills: None,
                sandbox_limits: None,
                max_concurrent_tasks: None,
                avatar: None,
                identity: Default::default(),
                prompt: None,
                heartbeat_interval: None,
                next_heartbeat_at: None,
                heartbeat_chat_id: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            crate::chat::models::Chat {
                id: "test-chat".into(),
                user_id: "test-user".into(),
                space_id: None,
                task_id: None,
                agent_id: "test-agent".into(),
                title: None,
                archived_at: None,
                channel_id: None,
                channel_external_id: None,
                metadata: Default::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            event_sender,
            tokio_util::sync::CancellationToken::new(),
            tokio_util::sync::CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn test_registry_dispatch() {
        let mut registry = AgentToolRegistry::empty();
        registry.register(Arc::new(MockTool));

        let defs = registry.definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].id, "mock_action");

        let ctx = mock_context();
        let output = registry
            .execute("mock_action", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        assert_eq!(output.text_content(), "executed mock_action");
    }

    #[tokio::test]
    async fn test_registry_unknown_tool() {
        let registry = AgentToolRegistry::empty();
        let ctx = mock_context();
        let result = registry.execute("nonexistent", serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
    }

    struct OtherTool;

    #[async_trait]
    impl AgentTool for OtherTool {
        fn name(&self) -> &str {
            "other"
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                id: "other_action".into(),
                provider_id: String::new(),
                description: String::new(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            }]
        }
        async fn execute(&self, _: &str, _: Value, _: &InferenceContext) -> Result<ToolOutput, AppError> {
            Ok(ToolOutput::text("other"))
        }
    }

    #[tokio::test]
    async fn restrict_to_keeps_only_listed_tools() {
        let mut registry = AgentToolRegistry::empty();
        registry.register(Arc::new(MockTool));
        registry.register(Arc::new(OtherTool));
        assert_eq!(registry.definitions().len(), 2);

        registry.restrict_to(&["mock_action"]);

        assert_eq!(registry.definitions().len(), 1);
        assert_eq!(registry.definitions()[0].id, "mock_action");

        let ctx = mock_context();
        assert!(registry.execute("other_action", serde_json::json!({}), &ctx).await.is_err());
        assert!(registry.execute("mock_action", serde_json::json!({}), &ctx).await.is_ok());
    }

    #[tokio::test]
    async fn restrict_to_empty_yields_empty_registry() {
        let mut registry = AgentToolRegistry::empty();
        registry.register(Arc::new(MockTool));
        registry.restrict_to(&[]);
        assert!(registry.is_empty());
    }
}
