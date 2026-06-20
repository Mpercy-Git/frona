use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use metrics_exporter_prometheus::PrometheusHandle;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::agent::signal::SignalService;
use crate::agent::task::executor::TaskExecutor;

use crate::agent::service::AgentService;
use crate::app::manager::AppManager;
use crate::app::service::AppService;
use crate::agent::skill::registry::SkillRegistryClient;
use crate::agent::skill::resolver::SkillResolver;
use crate::agent::skill::service::SkillService;
use crate::storage::StorageService;
use crate::auth::AuthService;
use crate::auth::jwt::JwtService;
use crate::auth::lockout::LoginAttemptTracker;
use crate::auth::oauth::service::OAuthService;
use crate::auth::token::service::TokenService;
use crate::call::CallService;
use crate::chat::broadcast::BroadcastService;
use crate::chat::service::ChatService;
use crate::contact::ContactService;
use crate::credential::keypair::service::KeyPairService;
use crate::credential::presign::PresignService;
use crate::credential::vault::service::VaultService;
use crate::inference::ModelProviderRegistry;
use crate::inference::config::ModelRegistryConfig;
use crate::memory::service::MemoryService;
use crate::notification::service::NotificationService;
use crate::policy::service::PolicyService;
use crate::tool::manager::ToolManager;
use crate::agent::prompt::PromptLoader;
use crate::space::service::SpaceService;
use crate::agent::task::service::TaskService;
use crate::tool::browser::session::BrowserSessionManager;
use crate::tool::cli::{CliToolConfig, load_cli_tool_configs};
use crate::tool::voice::{VoiceProvider, create_voice_provider};
use crate::tool::web_search::{SearchProvider, create_search_provider};
use crate::tool::sandbox::{SandboxFactory, SandboxManager};
use crate::tool::sandbox::driver::resource_monitor::SystemResourceManager;
use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use super::config::Config;
use crate::auth::UserService;
use crate::db::repo::generic::SurrealRepo;

#[derive(Clone, Default)]
pub struct ActiveSessions {
    inner: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl ActiveSessions {
    pub async fn register(&self, chat_id: &str) -> CancellationToken {
        let mut map = self.inner.lock().await;
        if let Some(existing) = map.get(chat_id) {
            existing.cancel();
        }
        let token = CancellationToken::new();
        map.insert(chat_id.to_string(), token.clone());
        token
    }

    pub async fn cancel(&self, chat_id: &str) -> bool {
        let map = self.inner.lock().await;
        if let Some(token) = map.get(chat_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub async fn remove(&self, chat_id: &str) {
        self.inner.lock().await.remove(chat_id);
    }

    pub async fn count(&self) -> usize {
        self.inner.lock().await.len()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: Surreal<Db>,
    pub auth_service: Arc<AuthService>,
    pub app_service: AppService,
    pub user_service: UserService,
    pub user_group_service: crate::auth::group_service::UserGroupService,
    pub agent_service: AgentService,
    pub space_service: SpaceService,
    pub call_service: CallService,
    pub chat_service: ChatService,
    pub contact_service: ContactService,
    pub task_service: TaskService,
    pub broadcast_service: BroadcastService,
    pub browser_session_manager: Arc<BrowserSessionManager>,
    pub active_sessions: ActiveSessions,
    pub memory_service: MemoryService,
    pub notification_service: NotificationService,
    pub sandbox_factory: Arc<SandboxFactory>,
    pub sandbox_manager: Arc<SandboxManager>,
    pub cli_tools_config: Arc<Vec<CliToolConfig>>,
    pub search_provider: Option<Arc<dyn SearchProvider>>,
    pub voice_provider: Option<Arc<dyn VoiceProvider>>,
    pub skill_service: SkillService,
    pub task_executor: Arc<TaskExecutor>,
    pub signal_service: Arc<OnceLock<Arc<SignalService>>>,
    pub config: Arc<Config>,
    pub storage_service: StorageService,
    pub prompts: PromptLoader,
    pub vault_service: VaultService,
    pub policy_service: PolicyService,
    pub tool_manager: Arc<ToolManager>,
    pub mcp_manager: Arc<crate::tool::mcp::McpManager>,
    pub mcp_service: Arc<crate::tool::mcp::McpServerService>,
    pub keypair_service: KeyPairService,
    pub presign_service: PresignService,
    pub share_service: crate::credential::share::service::ShareService,
    pub token_service: TokenService,
    pub oauth_service: Option<OAuthService>,
    pub login_tracker: LoginAttemptTracker,
    pub metrics_handle: PrometheusHandle,
    pub shutdown_token: CancellationToken,
    pub channel_registry: Arc<crate::chat::channel::ChannelRegistry>,
    pub channel_manager: Arc<crate::chat::channel::ChannelManager>,
    pub channel_service: Arc<crate::chat::channel::ChannelService>,
    pub http_client: reqwest::Client,
    pub harness: Arc<crate::agent::harness::Harness>,
}

impl AppState {
    pub fn new(
        db: Surreal<Db>,
        config: &Config,
        models_config: Option<ModelRegistryConfig>,
        storage: StorageService,
        metrics_handle: PrometheusHandle,
        resource_manager: Arc<SystemResourceManager>,
    ) -> Self {
        // Both `aws-lc-rs` and `ring` are active via reqwest + slack-morphism;
        // rustls 0.23 panics on first TLS use without an explicit default.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let http_client = crate::build_http_client();

        let broadcast_service = BroadcastService::with_pending_events_secs(config.server.sse_pending_events_secs);
        let llm_config = load_models_config(models_config);
        let provider_registry = ModelProviderRegistry::from_config(llm_config, broadcast_service.clone(), &config.inference)
            .expect("Failed to initialize provider registry");

        let chat_repo = SurrealRepo::new(db.clone());
        let message_repo = SurrealRepo::new(db.clone());
        let tool_call_repo = SurrealRepo::new(db.clone());

        let shared_config_dir = PathBuf::from(&config.storage.shared_config_dir);
        let shared_config_abs = std::fs::canonicalize(&shared_config_dir)
            .unwrap_or_else(|_| shared_config_dir.clone());

        let sandbox_factory = Arc::new(
            SandboxFactory::new(config.sandbox.disabled, resource_manager.clone())
                .with_default_timeout(config.sandbox.default_limits.timeout_secs)
                .with_shared_read_paths(vec![shared_config_abs.to_string_lossy().into_owned()]),
        );
        // `SandboxManager` (the orchestrator that bundles services and provides
        // `for_context`) is constructed below, after PolicyService et al. exist.
        let search_provider = create_search_provider(http_client.clone(), &config.search);
        let local_base_url = config.server.base_url.clone()
            .unwrap_or_else(|| format!("http://localhost:{}", config.server.port));
        let voice_base_url = config.server.external_base_url()
            .unwrap_or_else(|| local_base_url.clone());

        let provider_registry_arc = Arc::new(provider_registry.clone());
        let schema_path = shared_config_abs.join("schemas").join("service_manifest.json")
            .to_string_lossy().into_owned();
        let prompt_loader = PromptLoader::new(shared_config_abs.join("prompts"))
            .with_var("schema_path", &schema_path);

        let cli_tools_config = load_cli_tool_configs(&prompt_loader);
        let cli_tools_config = Arc::new(cli_tools_config);

        let memory_service = MemoryService::new(
            SurrealRepo::new(db.clone()),
            SurrealRepo::new(db.clone()),
            SurrealRepo::new(db.clone()),
            provider_registry_arc,
            prompt_loader.clone(),
            storage.clone(),
        );

        let skill_resolver = SkillResolver::new(&config.storage.shared_config_dir, storage.clone())
            .with_installed_dir(&config.storage.skills_dir);
        let skill_service = SkillService::new(
            SkillRegistryClient::new(http_client.clone(), format!("{}/skills", config.storage.cache_dir)),
            skill_resolver,
            storage.clone(),
            &config.storage.skills_dir,
            &config.cache,
        );

        let keypair_repo: SurrealRepo<crate::credential::keypair::models::KeyPair> =
            SurrealRepo::new(db.clone());
        let keypair_service = KeyPairService::new(
            &config.auth.encryption_secret,
            Arc::new(keypair_repo),
        );
        let user_service = UserService::new(SurrealRepo::new(db.clone()), &config.cache);
        let user_group_service = crate::auth::group_service::UserGroupService::new(db.clone());
        let presign_service = PresignService::new(
            keypair_service.clone(),
            user_service.clone(),
            local_base_url.clone(),
            config.auth.presign_expiry_secs,
        );

        let share_repo: Arc<dyn crate::credential::share::repository::ShareRepository> =
            Arc::new(SurrealRepo::<crate::credential::share::models::Share>::new(db.clone()));
        let share_service = crate::credential::share::service::ShareService::new(
            share_repo,
            config.share.ttl_secs,
        );

        let jwt_service = JwtService::new();
        let token_repo: SurrealRepo<crate::auth::token::models::ApiToken> =
            SurrealRepo::new(db.clone());
        let token_service = TokenService::new(
            Arc::new(token_repo),
            jwt_service,
            user_service.clone(),
            config.auth.access_token_expiry_secs,
            config.auth.refresh_token_expiry_secs,
        );

        let voice_provider = create_voice_provider(
            &config.voice,
            &voice_base_url,
            token_service.clone(),
            keypair_service.clone(),
        );
        match &voice_provider {
            Some(p) => tracing::info!(provider = %p.name(), voice_base_url = %voice_base_url, "Voice calling enabled"),
            None => tracing::info!("Voice calling disabled (no provider configured)"),
        }

        let vault_credential_repo: Arc<dyn crate::credential::vault::repository::CredentialRepository> =
            Arc::new(SurrealRepo::<crate::credential::vault::models::Credential>::new(db.clone()));
        let vault_connection_repo: Arc<dyn crate::credential::vault::repository::VaultConnectionRepository> =
            Arc::new(SurrealRepo::<crate::credential::vault::models::VaultConnection>::new(db.clone()));
        let vault_grant_repo: Arc<dyn crate::credential::vault::repository::VaultGrantRepository> =
            Arc::new(SurrealRepo::<crate::credential::vault::models::VaultGrant>::new(db.clone()));
        let vault_access_log_repo: Arc<dyn crate::credential::vault::repository::VaultAccessLogRepository> =
            Arc::new(SurrealRepo::<crate::credential::vault::models::VaultAccessLog>::new(db.clone()));
        let binding_repo: Arc<dyn crate::credential::vault::repository::PrincipalCredentialBindingRepository> =
            Arc::new(SurrealRepo::<crate::credential::vault::models::PrincipalCredentialBinding>::new(db.clone()));
        let data_dir = PathBuf::from(&config.database.path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("data"));
        let vault_service = VaultService::new(
            vault_connection_repo,
            vault_grant_repo,
            vault_credential_repo,
            vault_access_log_repo,
            binding_repo,
            &config.auth.encryption_secret,
            config.vault.clone(),
            data_dir,
            storage.clone(),
            user_service.clone(),
        );

        let oauth_service = if config.sso.enabled {
            let oauth_repo: SurrealRepo<crate::auth::oauth::models::OAuthIdentity> =
                SurrealRepo::new(db.clone());
            OAuthService::new(config, Arc::new(oauth_repo), http_client.clone()).ok()
        } else {
            None
        };

        let tool_manager = Arc::new(ToolManager::new(config.mcp.bridge_mode));
        let policy_schema = crate::policy::schema::build_schema();
        let policy_repo: Arc<dyn crate::policy::repository::PolicyRepository> =
            Arc::new(SurrealRepo::<crate::policy::models::Policy>::new(db.clone()));
        let policy_service = PolicyService::with_sandbox_disabled(
            policy_repo,
            policy_schema,
            tool_manager.clone(),
            storage.clone(),
            user_service.clone(),
            config.sandbox.disabled,
        );

        let sandbox_manager = Arc::new(SandboxManager::new(
            sandbox_factory.clone(),
            policy_service.clone(),
            skill_service.clone(),
            storage.clone(),
            token_service.clone(),
            keypair_service.clone(),
            config.server.public_base_url(),
            config.auth.ephemeral_token_expiry_secs,
            config.server.timezone.clone(),
        ));

        let agent_service = AgentService::new(
            SurrealRepo::new(db.clone()),
            &config.cache,
            resource_manager.clone(),
            policy_service.clone(),
            user_service.clone(),
        );

        let app_manager = Arc::new(AppManager::new(
            sandbox_manager.clone(),
            config.app.port_range_start,
            config.app.port_range_end,
            user_service.clone(),
            agent_service.clone(),
            http_client.clone(),
        ));

        let mcp_manager = Arc::new(crate::tool::mcp::McpManager::new(
            sandbox_manager.clone(),
            storage.clone(),
            config.mcp.port_range_start,
            config.mcp.port_range_end,
            user_service.clone(),
            http_client.clone(),
        ));
        let mcp_repo: Arc<dyn crate::tool::mcp::repository::McpServerRepository> =
            Arc::new(SurrealRepo::<crate::tool::mcp::McpServer>::new(db.clone()));
        let mcp_registry: Arc<dyn crate::tool::mcp::McpRegistryClient> =
            Arc::new(crate::tool::mcp::PrebuiltMcpRegistryClient::new(
                http_client.clone(),
                std::path::PathBuf::from(
                    config.mcp.cache_path.clone()
                        .unwrap_or_else(|| format!("{}/mcp", config.storage.cache_dir))
                ).join("registry"),
            ));
        let mcp_installer: Arc<dyn crate::tool::mcp::PackageInstaller> =
            Arc::new(crate::tool::mcp::SandboxedPackageInstaller::new(
                mcp_manager.clone(),
            ));

        let mcp_service = Arc::new(crate::tool::mcp::McpServerService::new(
            mcp_repo,
            mcp_manager.clone(),
            mcp_registry,
            Arc::new(vault_service.clone()),
            mcp_installer,
            tool_manager.clone(),
            token_service.clone(),
            keypair_service.clone(),
            user_service.clone(),
            policy_service.clone(),
            storage.clone(),
            config.server.public_base_url(),
            config.auth.ephemeral_token_expiry_secs,
        ));

        let app_service = AppService::new(
            SurrealRepo::new(db.clone()),
            app_manager,
            config.app.clone(),
            policy_service.clone(),
            user_service.clone(),
        );

        let channel_registry = {
            let reg = Arc::new(crate::chat::channel::ChannelRegistry::new());
            reg.register_factory(Arc::new(crate::chat::channel::adapter::telegram::TelegramAdapterFactory));
            reg.register_factory(Arc::new(crate::chat::channel::adapter::sms::SmsAdapterFactory));
            reg.register_factory(Arc::new(crate::chat::channel::adapter::slack::SlackAdapterFactory));
            reg.register_factory(Arc::new(crate::chat::channel::adapter::whatsapp_cloud::WhatsAppCloudAdapterFactory));
            reg.register_factory(Arc::new(crate::chat::channel::adapter::whatsapp_user::WhatsAppUserAdapterFactory));
            reg.register_factory(Arc::new(crate::chat::channel::adapter::discord::DiscordAdapterFactory));
            reg.register_factory(Arc::new(crate::chat::channel::adapter::signal::SignalAdapterFactory));
            reg
        };
        let channel_repo: Arc<dyn crate::chat::channel::repository::ChannelRepository> =
            Arc::new(SurrealRepo::<crate::chat::channel::Channel>::new(db.clone()));
        let config_arc = Arc::new(config.clone());
        let channel_service = Arc::new(crate::chat::channel::ChannelService::new(
            channel_repo,
            channel_registry.clone(),
            Arc::new(vault_service.clone()),
            broadcast_service.clone(),
            config_arc.clone(),
        ));

        let chat_service = ChatService::new(
            chat_repo,
            message_repo,
            tool_call_repo,
            agent_service.clone(),
            provider_registry,
            storage.clone(),
            user_service.clone(),
            memory_service.clone(),
            prompt_loader.clone(),
            broadcast_service.clone(),
            presign_service.clone(),
        );
        let shutdown_token = CancellationToken::new();
        let active_sessions = ActiveSessions::default();
        let harness = Arc::new(crate::agent::harness::Harness::new(
            chat_service.clone(),
            user_service.clone(),
            storage.clone(),
            agent_service.clone(),
            memory_service.clone(),
            skill_service.clone(),
            TaskService::new(SurrealRepo::new(db.clone()), broadcast_service.clone()),
            vault_service.clone(),
            mcp_service.clone(),
            tool_manager.clone(),
            policy_service.clone(),
            broadcast_service.clone(),
            active_sessions.clone(),
            shutdown_token.clone(),
            prompt_loader.clone(),
            config_arc.clone(),
        ));
        let task_executor = Arc::new(crate::agent::task::executor::TaskExecutor::new(
            harness.clone(),
        ));
        let message_repo_for_channel: Arc<dyn crate::chat::message::repository::MessageRepository> =
            Arc::new(SurrealRepo::<crate::chat::message::models::Message>::new(db.clone()));
        let channel_manager = Arc::new(crate::chat::channel::ChannelManager::new(
            message_repo_for_channel,
            chat_service.clone(),
            channel_service.clone(),
            harness.clone(),
            task_executor.clone(),
        ));
        Self {
            db: db.clone(),
            auth_service: Arc::new(AuthService::new()),
            app_service,
            user_service: user_service.clone(),
            user_group_service: user_group_service.clone(),
            agent_service: agent_service.clone(),
            space_service: SpaceService::new(SurrealRepo::new(db.clone()), broadcast_service.clone()),
            call_service: CallService::new(SurrealRepo::new(db.clone())),
            contact_service: ContactService::new(SurrealRepo::new(db.clone()), broadcast_service.clone()),
            chat_service,
            task_service: TaskService::new(SurrealRepo::new(db.clone()), broadcast_service.clone()),
            broadcast_service: broadcast_service.clone(),
            browser_session_manager: Arc::new(BrowserSessionManager::new(config.browser.clone())),
            active_sessions,
            memory_service,
            notification_service: NotificationService::new(SurrealRepo::new(db.clone())),
            policy_service: policy_service.clone(),
            tool_manager,
            sandbox_factory,
            sandbox_manager,
            cli_tools_config,
            search_provider,
            voice_provider,
            skill_service,
            task_executor,
            signal_service: Arc::new(OnceLock::new()),
            config: config_arc,
            storage_service: storage,
            prompts: prompt_loader,
            vault_service,
            mcp_manager,
            mcp_service,
            keypair_service,
            presign_service,
            share_service,
            token_service,
            oauth_service,
            login_tracker: LoginAttemptTracker::new(5, 15),
            metrics_handle,
            shutdown_token,
            channel_registry: channel_registry.clone(),
            channel_manager,
            channel_service,
            http_client,
            harness,
        }
    }

    pub async fn get_runtime_config(&self, key: &str) -> Result<Option<String>, crate::core::error::AppError> {
        let mut result = self.db
            .query("SELECT `value` FROM runtime_config WHERE `key` = $key LIMIT 1")
            .bind(("key", key.to_string()))
            .await
            .map_err(|e| crate::core::error::AppError::Internal(e.to_string()))?;
        let row: Option<serde_json::Value> = result.take(0)
            .map_err(|e| crate::core::error::AppError::Internal(e.to_string()))?;
        Ok(row.and_then(|v| v.get("value").and_then(|v| v.as_str().map(String::from))))
    }

    pub async fn set_runtime_config(&self, key: &str, value: &str) -> Result<(), crate::core::error::AppError> {
        self.db
            .query(
                "DELETE FROM runtime_config WHERE `key` = $key; \
                 CREATE runtime_config SET `key` = $key, `value` = $value, updated_at = $now"
            )
            .bind(("key", key.to_string()))
            .bind(("value", value.to_string()))
            .bind(("now", chrono::Utc::now()))
            .await
            .map_err(|e| crate::core::error::AppError::Internal(e.to_string()))?;
        Ok(())
    }

    pub async fn get_runtime_config_bool(&self, key: &str) -> bool {
        self.get_runtime_config(key)
            .await
            .ok()
            .flatten()
            .is_some_and(|v| v == "true")
    }

    pub fn init_signal_service(&self) -> Arc<SignalService> {
        let svc = Arc::new(SignalService::new(
            self.task_service.clone(),
            self.task_executor.clone(),
            self.agent_service.clone(),
            self.contact_service.clone(),
            self.policy_service.clone(),
            self.prompts.clone(),
        ));
        let _ = self.signal_service.set(svc.clone());
        svc
    }

    pub fn signal_service(&self) -> Option<Arc<SignalService>> {
        self.signal_service.get().cloned()
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutdown_token.is_cancelled()
    }

    pub fn compaction_model_group(&self) -> Option<crate::inference::config::ModelGroup> {
        let registry = self.chat_service.provider_registry();
        if let Ok(group) = registry.get_model_group("compaction") {
            return Some(group.clone());
        }
        if let Ok(group) = registry.get_model_group("primary") {
            return Some(group.clone());
        }
        None
    }
}

fn load_models_config(from_yaml: Option<ModelRegistryConfig>) -> ModelRegistryConfig {
    match from_yaml {
        Some(mut config) => {
            config.merge_with_auto_discovered();
            tracing::info!("Loaded models config from config file");
            config
        }
        None => {
            tracing::info!("No models in config, auto-discovering from environment");
            ModelRegistryConfig::auto_discover()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_count() {
        let sessions = ActiveSessions::default();
        sessions.register("chat-1").await;
        sessions.register("chat-2").await;
        assert_eq!(sessions.count().await, 2);
    }

    #[tokio::test]
    async fn test_remove_decrements_count() {
        let sessions = ActiveSessions::default();
        sessions.register("chat-1").await;
        sessions.register("chat-2").await;
        sessions.remove("chat-1").await;
        assert_eq!(sessions.count().await, 1);
    }

    #[tokio::test]
    async fn test_register_cancels_previous() {
        let sessions = ActiveSessions::default();
        let first = sessions.register("chat-1").await;
        let _second = sessions.register("chat-1").await;
        assert!(first.is_cancelled());
        assert_eq!(sessions.count().await, 1);
    }

    #[tokio::test]
    async fn test_cancel_returns_true_for_existing() {
        let sessions = ActiveSessions::default();
        sessions.register("chat-1").await;
        assert!(sessions.cancel("chat-1").await);
    }

    #[tokio::test]
    async fn test_cancel_returns_false_for_missing() {
        let sessions = ActiveSessions::default();
        assert!(!sessions.cancel("nonexistent").await);
    }

    #[test]
    fn test_is_shutting_down() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }
}
