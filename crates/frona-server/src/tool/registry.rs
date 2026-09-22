use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::agent::harness::Harness;
use crate::core::error::AppError;

use super::{AgentTool, InferenceContext, ToolDefinition, ToolOutput};

/// The shell tool's id, as declared by `resources/prompts/tools/shell.md`. It is
/// the sandbox command runner, and so the only way to invoke `mcpctl`.
pub const SHELL_TOOL_ID: &str = "shell";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolFilter {
    /// Lock the agent to a small set of tools. Used by signal-mode and
    /// quarantined-task execution.
    AllowList(&'static [&'static str]),
    /// Hide specific tools while keeping everything else. Used to gate scheduling
    /// tools (e.g. `create_recurring_task`) inside a running task.
    DenyList(&'static [&'static str]),
}

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

    /// Whether MCP calls should actually be routed through the `mcpctl` bridge.
    ///
    /// Bridge mode trades every `mcp__*` tool definition for a single `mcpctl`
    /// command line, and `mcpctl` is a binary in the sandbox - the shell tool is
    /// the only thing that can run it. An agent whose tool list has no shell -
    /// restricted by `tools:` frontmatter, denied by Cedar, or filtered down by
    /// `apply_filter` - cannot reach it, so bridging there would strip the
    /// `mcp__*` definitions and leave *nothing* able to call the server while
    /// the prompt still advertises it. Fall back to plain tool definitions
    /// instead. Read at call time, not at construction, because the filters that
    /// remove the shell run after `new`.
    pub fn mcp_bridge_active(&self) -> bool {
        self.mcp_bridge_mode && self.has_tool(SHELL_TOOL_ID)
    }

    pub fn has_tool(&self, tool_id: &str) -> bool {
        self.definitions.iter().any(|d| d.id == tool_id)
    }

    pub fn apply_filter(&mut self, filter: &ToolFilter) {
        match filter {
            ToolFilter::AllowList(allowed) => self.restrict_to(allowed),
            ToolFilter::DenyList(denied) => self.deny(denied),
        }
    }

    /// Restrict beyond Cedar: even if a tool is permitted generally, hide it here.
    pub fn restrict_to(&mut self, allowed: &[&str]) {
        let allow_set: std::collections::HashSet<&str> = allowed.iter().copied().collect();
        self.definitions
            .retain(|d| allow_set.contains(d.id.as_str()));
        let surviving_owners: std::collections::HashSet<String> = self
            .definitions
            .iter()
            .filter_map(|d| self.tool_name_to_owner.get(&d.id).cloned())
            .collect();
        self.tool_name_to_owner
            .retain(|tool_id, _| allow_set.contains(tool_id.as_str()));
        self.tools
            .retain(|owner, _| surviving_owners.contains(owner));
    }

    pub fn deny(&mut self, denied: &[&str]) {
        let deny_set: std::collections::HashSet<&str> = denied.iter().copied().collect();
        self.definitions
            .retain(|d| !deny_set.contains(d.id.as_str()));
        let surviving_owners: std::collections::HashSet<String> = self
            .definitions
            .iter()
            .filter_map(|d| self.tool_name_to_owner.get(&d.id).cloned())
            .collect();
        self.tool_name_to_owner
            .retain(|tool_id, _| !deny_set.contains(tool_id.as_str()));
        self.tools
            .retain(|owner, _| surviving_owners.contains(owner));
    }

    pub fn register(&mut self, tool: Arc<dyn AgentTool>) {
        let definitions = tool.definitions();
        self.register_with_definitions(tool, definitions);
    }

    pub fn register_required(&mut self, tool: Arc<dyn AgentTool>) -> Result<(), AppError> {
        let definitions = tool.definitions();
        if definitions.is_empty() {
            return Err(AppError::Internal(format!(
                "required tool `{}` has no model definitions",
                tool.name(),
            )));
        }
        self.register_with_definitions(tool, definitions);
        Ok(())
    }

    fn register_with_definitions(
        &mut self,
        tool: Arc<dyn AgentTool>,
        definitions: Vec<ToolDefinition>,
    ) {
        let owner_name = tool.name().to_string();
        for def in definitions {
            self.tool_name_to_owner
                .insert(def.id.clone(), owner_name.clone());
            self.definitions.push(def);
        }
        self.tools.insert(owner_name, tool);
    }

    pub async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
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

/// What `<available_agents>` has to say for one agent: who it may delegate to,
/// and who it may not.
///
/// The denied list is the point of the type. Delegation is permitted only to
/// agents whose tools are a subset of the caller's (the `delegation` policy in
/// `resources/policy/frona.cedar`), so narrowing one agent's tools silently
/// empties its view of every colleague that kept a tool it lost. With only the
/// delegable list the section vanished entirely, leaving an agent that had been
/// told to "check `<available_agents>`" to conclude the user has no other
/// agents - and say so. Carrying the denied names keeps the prompt honest: they
/// exist, they are simply not reachable from here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentSummaries {
    /// `(name, description)` for each agent this one may delegate to.
    pub delegable: Vec<(String, String)>,
    /// Names of the enabled agents the delegation policy turned down.
    pub denied: Vec<String>,
}

impl AgentSummaries {
    /// True when there is genuinely nothing to say - no colleagues at all,
    /// reachable or otherwise.
    pub fn is_empty(&self) -> bool {
        self.delegable.is_empty() && self.denied.is_empty()
    }
}

pub async fn build_agent_summaries(
    harness: &Harness,
    user_id: &str,
    current_agent_id: &str,
) -> AgentSummaries {
    let current_agent = match harness.agent_service.find_by_id(current_agent_id).await {
        Ok(Some(agent)) => agent,
        _ => return AgentSummaries::default(),
    };

    let agents = match harness.agent_service.list(user_id).await {
        Ok(agents) => agents,
        Err(_) => return AgentSummaries::default(),
    };

    let mut summaries = AgentSummaries::default();
    for target in &agents {
        // A disabled agent is off on purpose: it is not a permission the user
        // has to go and fix, so it stays out of both lists.
        if target.id == current_agent_id || !target.enabled {
            continue;
        }
        let decision = harness
            .policy_service
            .authorize(
                user_id,
                &current_agent,
                crate::policy::models::PolicyAction::DelegateTask {
                    target_agent_id: target.id.clone(),
                    target_handle: target.handle.clone(),
                },
            )
            .await;
        if decision.is_ok_and(|d| d.allowed) {
            summaries
                .delegable
                .push((target.name.clone(), target.description.clone()));
        } else {
            summaries.denied.push(target.name.clone());
        }
    }

    if !summaries.denied.is_empty() {
        // The operator-facing half of the same fact. Whichever agent is
        // complaining that it cannot see its colleagues, this line names them
        // and the principal the policy turned down.
        tracing::debug!(
            agent = %current_agent.handle,
            denied = ?summaries.denied,
            "delegation policy hid agents from <available_agents>"
        );
    }

    summaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockTool;

    struct EmptyTool;

    #[async_trait]
    impl AgentTool for EmptyTool {
        fn name(&self) -> &str {
            "empty"
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            Vec::new()
        }
        async fn execute(
            &self,
            _: &str,
            _: Value,
            _: &InferenceContext,
        ) -> Result<ToolOutput, AppError> {
            Ok(ToolOutput::text("empty"))
        }
    }

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

        async fn execute(
            &self,
            tool_name: &str,
            _arguments: Value,
            _ctx: &InferenceContext,
        ) -> Result<ToolOutput, AppError> {
            Ok(ToolOutput::text(format!("executed {tool_name}")))
        }
    }

    fn mock_context() -> InferenceContext {
        let broadcast = crate::chat::broadcast::BroadcastService::new();
        let event_sender = broadcast.create_event_sender("test-user", "test-chat", None);
        InferenceContext::new(
            crate::auth::User {
                id: "test-user".into(),
                handle: crate::handle!("testuser"),
                email: "test@test.com".into(),
                name: "Test".into(),
                password_hash: String::new(),
                timezone: None,
                phone: None,
                groups: Vec::new(),
                deactivated_at: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            crate::agent::models::Agent {
                id: "test-agent".into(),
                user_id: "test-user".into(),
                handle: crate::handle!("test-agent"),
                name: "Test Agent".into(),
                description: String::new(),
                model_group: "primary".into(),
                enabled: true,
                skills: None,
                sandbox_limits: None,
                max_concurrent_tasks: None,
                avatar: None,
                voice_id: None,
                private_memory: false,
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

    #[test]
    fn required_tool_registration_rejects_an_empty_definition_set() {
        let mut registry = AgentToolRegistry::empty();

        let error = registry.register_required(Arc::new(EmptyTool)).unwrap_err();

        assert!(error.to_string().contains("empty"));
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn test_registry_unknown_tool() {
        let registry = AgentToolRegistry::empty();
        let ctx = mock_context();
        let result = registry
            .execute("nonexistent", serde_json::json!({}), &ctx)
            .await;
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
        async fn execute(
            &self,
            _: &str,
            _: Value,
            _: &InferenceContext,
        ) -> Result<ToolOutput, AppError> {
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
        assert!(
            registry
                .execute("other_action", serde_json::json!({}), &ctx)
                .await
                .is_err()
        );
        assert!(
            registry
                .execute("mock_action", serde_json::json!({}), &ctx)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn restrict_to_empty_yields_empty_registry() {
        let mut registry = AgentToolRegistry::empty();
        registry.register(Arc::new(MockTool));
        registry.restrict_to(&[]);
        assert!(registry.is_empty());
    }

    fn def(id: &str, provider_id: &str) -> ToolDefinition {
        ToolDefinition {
            id: id.to_string(),
            provider_id: provider_id.to_string(),
            description: String::new(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }
    }

    fn bridged(defs: Vec<ToolDefinition>) -> AgentToolRegistry {
        AgentToolRegistry::new(HashMap::new(), HashMap::new(), defs, true)
    }

    #[test]
    fn bridge_is_active_only_when_the_agent_can_run_mcpctl() {
        let with_shell = bridged(vec![
            def(SHELL_TOOL_ID, "shell"),
            def("mcp__ha__get_state", "mcp:ha"),
        ]);
        assert!(with_shell.mcp_bridge_active());

        // The failure this guards: bridge mode hides every `mcp__*` tool in
        // favour of `mcpctl`, so an agent with no shell is left holding neither.
        let without_shell = bridged(vec![
            def("read", "files"),
            def("memory_search", "memory"),
            def("mcp__ha__get_state", "mcp:ha"),
        ]);
        assert!(!without_shell.mcp_bridge_active());
        assert!(without_shell.mcp_bridge_mode());
    }

    #[test]
    fn bridge_stays_off_when_the_config_flag_is_off() {
        let registry = AgentToolRegistry::new(
            HashMap::new(),
            HashMap::new(),
            vec![def(SHELL_TOOL_ID, "shell")],
            false,
        );
        assert!(!registry.mcp_bridge_active());
    }

    #[test]
    fn losing_the_shell_to_a_filter_deactivates_the_bridge() {
        let mut registry = bridged(vec![
            def(SHELL_TOOL_ID, "shell"),
            def("mcp__ha__get_state", "mcp:ha"),
        ]);
        assert!(registry.mcp_bridge_active());

        // Signal-mode and quarantined tasks restrict after construction, which is
        // why the predicate reads the live definitions rather than a stored bool.
        registry.apply_filter(&ToolFilter::DenyList(&[SHELL_TOOL_ID]));
        assert!(!registry.mcp_bridge_active());
    }
}
