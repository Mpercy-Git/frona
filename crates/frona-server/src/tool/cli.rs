use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::agent::prompt::PromptLoader;
use crate::agent::skill::service::SkillService;
use crate::auth::ephemeral_token::EphemeralTokenGuard;
use crate::auth::token::service::TokenService;
use crate::core::Principal;
use crate::core::error::AppError;
use crate::credential::keypair::service::KeyPairService;
use crate::policy::service::PolicyService;

use super::sandbox::SandboxManager;
use super::{AgentTool, InferenceContext, ToolDefinition, ToolOutput, parse_frontmatter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliToolConfig {
    pub name: String,
    pub description: String,
    pub program: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub stdin: Option<String>,
    pub parameters: HashMap<String, Value>,
    #[serde(default)]
    pub required: Vec<String>,
    pub timeout_secs: Option<u64>,
    /// CLI tools sharing this id are grouped under one provider in the UI.
    #[serde(default)]
    pub provider: Option<String>,
}

pub struct CliTool {
    config: CliToolConfig,
    sandbox_manager: Arc<SandboxManager>,
    storage_service: crate::storage::service::StorageService,
    skill_service: SkillService,
    token_service: TokenService,
    keypair_service: KeyPairService,
    policy_service: PolicyService,
    api_base_url: String,
    ephemeral_token_expiry_secs: u64,
    server_timezone: String,
}

#[allow(clippy::too_many_arguments)]
impl CliTool {
    pub fn new(
        config: CliToolConfig,
        sandbox_manager: Arc<SandboxManager>,
        storage_service: crate::storage::service::StorageService,
        skill_service: SkillService,
        token_service: TokenService,
        keypair_service: KeyPairService,
        policy_service: PolicyService,
        api_base_url: String,
        ephemeral_token_expiry_secs: u64,
        server_timezone: String,
    ) -> Self {
        Self {
            config,
            sandbox_manager,
            storage_service,
            skill_service,
            token_service,
            keypair_service,
            policy_service,
            api_base_url,
            ephemeral_token_expiry_secs,
            server_timezone,
        }
    }

    fn substitute(template: &str, arguments: &Map<String, Value>) -> String {
        let mut result = template.to_string();
        for (key, value) in arguments {
            let placeholder = format!("${{{key}}}");
            let replacement = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            result = result.replace(&placeholder, &replacement);
        }
        result
    }
}

#[async_trait]
impl AgentTool for CliTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        let mut properties = Map::new();
        for (key, schema) in &self.config.parameters {
            properties.insert(key.clone(), schema.clone());
        }

        let parameters = serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": self.config.required,
        });

        vec![ToolDefinition {
            id: self.config.name.clone(),
            provider_id: self
                .config
                .provider
                .clone()
                .unwrap_or_else(|| self.config.name.clone()),
            description: self.config.description.clone(),
            parameters,
        }]
    }

    async fn execute(&self, _tool_name: &str, arguments: Value, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let args_map = arguments
            .as_object()
            .ok_or_else(|| AppError::Tool("Arguments must be a JSON object".to_string()))?;

        for req in &self.config.required {
            if req == "description" {
                continue;
            }
            if !args_map.contains_key(req) {
                return Err(AppError::Tool(format!("Missing required parameter: {req}")));
            }
        }

        let substituted_args: Vec<String> = self
            .config
            .args
            .iter()
            .map(|a| Self::substitute(a, args_map))
            .collect();
        let args_refs: Vec<&str> = substituted_args.iter().map(|s| s.as_str()).collect();

        let agent_id = &ctx.agent.id;

        let policy = self
            .policy_service
            .evaluate_sandbox_policy(
                crate::policy::service::SandboxPrincipalRef::agent(
                    &ctx.user.id,
                    &ctx.user.handle,
                    &ctx.agent.handle,
                ),
                true,
            )
            .await?;

        let skill_read_paths: Vec<String> = self
            .skill_service
            .list(&ctx.user.handle, &ctx.agent.handle, ctx.agent.skills.as_deref())
            .await
            .into_iter()
            .map(|s| s.path)
            .collect();

        let workspace = self
            .storage_service
            .agent_workspace_path(&ctx.user.handle, &ctx.agent.handle);
        let mut sandbox = self.sandbox_manager.get_sandbox(
            workspace,
            agent_id,
            policy.network_access,
            policy.network_destinations.clone(),
        )
        .with_read_paths(skill_read_paths)
        .with_read_paths(policy.read_paths.clone())
        .with_write_paths(policy.write_paths.clone())
        .with_denied_paths(policy.denied_paths.clone())
        .with_blocked_networks(policy.blocked_networks.clone())
        .with_bind_ports(policy.bind_ports.clone());

        if !ctx.file_paths.is_empty() {
            sandbox = sandbox.with_write_paths(ctx.file_paths.clone());
        }

        let tokens_dir = self.storage_service.user_tokens_path(&ctx.user.handle);
        let token_guard = EphemeralTokenGuard::issue(
            &self.token_service,
            &self.keypair_service,
            &ctx.user,
            Principal::agent(agent_id),
            self.ephemeral_token_expiry_secs,
            &tokens_dir,
        )
        .await?;

        sandbox = sandbox.with_read_files(vec![
            token_guard.path().to_string_lossy().into_owned(),
        ]);

        {
            let mut extra_vars = ctx.vault_env_vars.read().await.clone();
            extra_vars.push((
                "TZ".to_string(),
                ctx.user.resolved_timezone(&self.server_timezone),
            ));
            extra_vars.push((
                "FRONA_TOKEN_FILE".to_string(),
                token_guard.path().to_string_lossy().into_owned(),
            ));
            extra_vars.push((
                "FRONA_API_URL".to_string(),
                self.api_base_url.clone(),
            ));
            sandbox = sandbox.with_extra_env_vars(extra_vars);
        }

        let timeout = ctx
            .agent
            .sandbox_limits
            .as_ref()
            .map(|l| l.timeout_secs)
            .unwrap_or_else(|| self.sandbox_manager.default_timeout_secs());

        let rm = self.sandbox_manager.resource_manager();
        let (eff_cpu, eff_mem) = rm.effective_agent_limits(agent_id);

        tracing::debug!(
            agent = %agent_id,
            tool = %self.config.name,
            timeout_secs = timeout,
            max_cpu_pct = eff_cpu,
            max_memory_pct = eff_mem,
            "Executing sandboxed command"
        );

        let output = sandbox
            .execute(
                &self.config.program,
                &args_refs,
                timeout,
                None,
                None,
                Some(ctx.cancel_token.clone()),
            )
            .await?;

        let mut result = String::new();

        if output.cancelled {
            result.push_str("Process cancelled by user.\n");
        } else if output.resource_killed {
            result.push_str("Process killed: resource limit exceeded.\n");
        } else if output.timed_out {
            result.push_str(&format!(
                "Process timed out after {timeout} seconds.\n"
            ));
        }

        if let Some(code) = output.exit_code
            && code != 0
        {
            result.push_str(&format!("Exit code: {code}\n"));
        }

        if !output.stdout.is_empty() {
            result.push_str(&output.stdout);
        }

        if !output.stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("stderr:\n");
            result.push_str(&output.stderr);
        }

        if result.is_empty() {
            result.push_str("(no output)");
        }

        let failed = output.timed_out || output.exit_code.is_some_and(|c| c != 0);
        Ok(if failed { ToolOutput::error(result) } else { ToolOutput::text(result) })
    }
}

pub fn load_cli_tool_config(prompts: &PromptLoader, path: &str) -> Option<CliToolConfig> {
    let raw = prompts.read(path)?;
    let (yaml, body) = parse_frontmatter(&raw)?;

    let name = yaml.get("name")?.as_str()?.to_string();
    let program = yaml.get("program")?.as_str()?.to_string();

    let args: Vec<String> = yaml
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let stdin = yaml.get("stdin").and_then(|v| v.as_str()).map(|s| s.to_string());
    let timeout_secs = yaml.get("timeout_secs").and_then(|v| v.as_u64());
    let provider = yaml.get("provider").and_then(|v| v.as_str()).map(|s| s.to_string());

    let parameters: HashMap<String, Value> = yaml
        .get("parameters")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(Value::Null)))
                .collect()
        })
        .unwrap_or_default();

    let required: Vec<String> = yaml
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Some(CliToolConfig {
        name,
        description: body,
        program,
        args,
        stdin,
        parameters,
        required,
        timeout_secs,
        provider,
    })
}

pub fn load_cli_tool_configs(prompts: &PromptLoader) -> Vec<CliToolConfig> {
    let files = prompts.list_dir("tools");
    let mut configs = Vec::new();

    for path in &files {
        if let Some(config) = load_cli_tool_config(prompts, path) {
            configs.push(config);
            tracing::info!(path = %path, "Loaded CLI tool config");
        }
    }

    tracing::info!(count = configs.len(), "Loaded CLI tool configs");
    configs
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mock_token_services() -> (TokenService, KeyPairService) {
        use crate::auth::jwt::JwtService;
        use crate::db::repo::generic::SurrealRepo;

        let db: surrealdb::Surreal<surrealdb::engine::local::Db> =
            surrealdb::Surreal::new::<surrealdb::engine::local::Mem>(()).await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();

        let keypair = KeyPairService::new(
            "test-secret",
            Arc::new(SurrealRepo::new(db.clone())),
        );
        let user_service = crate::auth::user_service::UserService::new(
            SurrealRepo::new(db.clone()),
            &crate::core::config::CacheConfig::default(),
        );
        let tokens = TokenService::new(
            Arc::new(SurrealRepo::new(db.clone())),
            JwtService::new(),
            user_service,
            900,
            604_800,
        );
        (tokens, keypair)
    }

    #[test]
    fn test_substitute_placeholders() {
        let mut args = Map::new();
        args.insert("command".to_string(), Value::String("echo hello".to_string()));

        let result = CliTool::substitute("${command}", &args);
        assert_eq!(result, "echo hello");
    }

    #[test]
    fn test_substitute_no_match() {
        let args = Map::new();
        let result = CliTool::substitute("no placeholders here", &args);
        assert_eq!(result, "no placeholders here");
    }

    #[test]
    fn test_load_embedded_config() {
        let shared_prompts = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("resources")
            .join("prompts");
        let prompts = PromptLoader::new(shared_prompts);
        let configs = load_cli_tool_configs(&prompts);
        assert_eq!(configs.len(), 3);
        assert!(configs.iter().any(|c| c.name == "shell"));
        assert!(configs.iter().any(|c| c.name == "python"));
        assert!(configs.iter().any(|c| c.name == "node"));
    }

    fn mock_skill_service() -> SkillService {
        use crate::agent::skill::registry::SkillRegistryClient;
        use crate::agent::skill::resolver::SkillResolver;
        use crate::core::config::{Config, CacheConfig};

        let config = Config::default();
        let storage = crate::storage::StorageService::new(&config);
        let resolver = SkillResolver::new("/tmp/frona_test_config", storage.clone());
        SkillService::new(
            SkillRegistryClient::default(),
            resolver,
            storage,
            "/tmp/frona_test_skills",
            &CacheConfig::default(),
        )
    }

    async fn mock_policy_service() -> PolicyService {
        use crate::db::repo::generic::SurrealRepo;
        use crate::policy::repository::PolicyRepository;
        let db = surrealdb::Surreal::new::<surrealdb::engine::local::Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        let schema = crate::policy::schema::build_schema();
        let repo: std::sync::Arc<dyn PolicyRepository> =
            std::sync::Arc::new(SurrealRepo::<crate::policy::models::Policy>::new(db.clone()));
        let tool_manager = std::sync::Arc::new(crate::tool::manager::ToolManager::new(false));
        let storage = crate::storage::StorageService::new(&crate::core::config::Config::default());
        let user_service = crate::auth::UserService::new(
            SurrealRepo::new(db),
            &crate::core::config::CacheConfig::default(),
        );
        let _ = user_service
            .create(&crate::auth::User {
                id: "test-user".into(),
                handle: crate::handle!("testuser"),
                email: "t@example.com".into(),
                name: "Test".into(),
                password_hash: String::new(),
                timezone: None,
                groups: Vec::new(),
                deactivated_at: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await;
        PolicyService::new(repo, schema, tool_manager, storage, user_service)
    }

    #[tokio::test]
    async fn test_cli_tool_definitions() {
        let config = CliToolConfig {
            name: "shell".to_string(),
            description: "Execute a shell command".to_string(),
            program: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "${command}".to_string()],
            stdin: None,
            parameters: {
                let mut m = HashMap::new();
                m.insert(
                    "command".to_string(),
                    serde_json::json!({"type": "string", "description": "The command"}),
                );
                m
            },
            required: vec!["command".to_string()],
            timeout_secs: Some(30),
            provider: None,
        };

        let wm = Arc::new(SandboxManager::new(false, Arc::new(crate::tool::sandbox::driver::resource_monitor::SystemResourceManager::new(60.0, 60.0, 60.0, 60.0))));
        let storage = crate::storage::StorageService::new(&crate::core::config::Config::default());
        let service = mock_skill_service();
        let (tokens, keypair) = mock_token_services().await;
        let tool = CliTool::new(
            config,
            wm,
            storage,
            service,
            tokens,
            keypair,
            mock_policy_service().await,
            "http://localhost".into(),
            300,
            "UTC".to_string(),
        );
        let defs = tool.definitions();

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].id, "shell");
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
    async fn test_cli_tool_execute_echo() {
        let config = CliToolConfig {
            name: "shell".to_string(),
            description: "Execute a shell command".to_string(),
            program: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "${command}".to_string()],
            stdin: None,
            parameters: {
                let mut m = HashMap::new();
                m.insert(
                    "command".to_string(),
                    serde_json::json!({"type": "string", "description": "The command"}),
                );
                m
            },
            required: vec!["command".to_string()],
            timeout_secs: Some(5),
            provider: None,
        };

        let tmp = std::env::temp_dir().join("frona_test_cli_tool");
        let _ = std::fs::create_dir_all(&tmp);

        let wm = Arc::new(SandboxManager::new(false, Arc::new(crate::tool::sandbox::driver::resource_monitor::SystemResourceManager::new(60.0, 60.0, 60.0, 60.0))));
        let mut config_obj = crate::core::config::Config::default();
        config_obj.storage.data_dir = tmp.to_string_lossy().into_owned();
        let storage = crate::storage::StorageService::new(&config_obj);
        let service = mock_skill_service();
        let (tokens, keypair) = mock_token_services().await;
        let tool = CliTool::new(
            config,
            wm,
            storage,
            service,
            tokens,
            keypair,
            mock_policy_service().await,
            "http://localhost".into(),
            300,
            "UTC".to_string(),
        );
        let ctx = mock_context();

        let result = tool
            .execute(
                "shell",
                serde_json::json!({"command": "echo hello world"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.text_content().contains("hello world"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
