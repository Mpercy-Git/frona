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
use crate::auth::password_reset::service::PasswordResetService;
use crate::mail::MailService;
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
use crate::notification::push_sender::PushSender;
use crate::notification::push_repository::PushSubscriptionRepository;
use crate::db::repo::push_subscriptions::SurrealPushSubscriptionRepo;
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

/// A named entry in the per-user voice inbound allowlist.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AllowlistEntry {
    pub phone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Default)]
pub struct ActiveSessions {
    /// Per-chat active run: a monotonic generation id plus its cancel token.
    /// The id lets a finishing run clean up only its *own* entry — critical
    /// when a new turn supersedes a running one (Stop, or an interrupting
    /// message): the superseded task must not delete the successor's token.
    inner: Arc<Mutex<HashMap<String, (u64, CancellationToken)>>>,
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

impl ActiveSessions {
    /// Register a new run for `chat_id`, cancelling any run already active for
    /// it. Returns the generation id (pass it to [`remove`](Self::remove)) and
    /// the cancel token for the new run.
    pub async fn register(&self, chat_id: &str) -> (u64, CancellationToken) {
        let mut map = self.inner.lock().await;
        if let Some((_, existing)) = map.get(chat_id) {
            existing.cancel();
        }
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let token = CancellationToken::new();
        map.insert(chat_id.to_string(), (id, token.clone()));
        (id, token)
    }

    /// Register a run for `chat_id` using a caller-supplied cancel token,
    /// instead of minting a fresh one. This is for callers (e.g. the task
    /// executor) whose run is already driven by their own token: registering
    /// that same token here means the generic chat-level [`cancel`](Self::cancel)
    /// fires the token the run is actually listening on. Cancels any run already
    /// active for the chat, and returns the generation id for [`remove`](Self::remove).
    pub async fn register_token(&self, chat_id: &str, token: CancellationToken) -> u64 {
        let mut map = self.inner.lock().await;
        if let Some((_, existing)) = map.get(chat_id) {
            existing.cancel();
        }
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        map.insert(chat_id.to_string(), (id, token));
        id
    }

    pub async fn cancel(&self, chat_id: &str) -> bool {
        let map = self.inner.lock().await;
        if let Some((_, token)) = map.get(chat_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Remove the entry for `chat_id`, but only if it still holds generation
    /// `id`. A superseded run passes its own (older) id here and is correctly
    /// a no-op, leaving the current run's token in place.
    pub async fn remove(&self, chat_id: &str, id: u64) {
        let mut map = self.inner.lock().await;
        if map.get(chat_id).is_some_and(|(cur, _)| *cur == id) {
            map.remove(chat_id);
        }
    }

    /// True if `id` is still the current generation registered for
    /// `chat_id` — i.e. this run has not been superseded by a newer
    /// `register`/`register_token` call. A cancelled run uses this to tell
    /// a genuine Stop (broadcast the cancellation) apart from losing a race
    /// against an interrupting message (stay quiet — the superseding run's
    /// own lifecycle events already carry the UI forward, and a stray
    /// "cancelled" broadcast for the old generation would otherwise reset
    /// the client's running/streaming state out from under the new turn).
    pub async fn is_current(&self, chat_id: &str, id: u64) -> bool {
        let map = self.inner.lock().await;
        map.get(chat_id).is_some_and(|(cur, _)| *cur == id)
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
    pub agent_share_service: crate::agent::share::service::AgentShareService,
    pub space_service: SpaceService,
    pub call_service: CallService,
    pub usage_service: crate::inference::usage::UsageService,
    pub model_catalog: crate::inference::metadata::ModelCatalogStore,
    pub chat_service: ChatService,
    pub chat_share_service: crate::chat::share::service::ChatShareService,
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
    pub password_reset_service: PasswordResetService,
    pub mail_service: Option<MailService>,
    pub login_tracker: LoginAttemptTracker,
    pub metrics_handle: PrometheusHandle,
    pub shutdown_token: CancellationToken,
    pub channel_registry: Arc<crate::chat::channel::ChannelRegistry>,
    pub channel_supervisor: Arc<crate::chat::channel::ChannelSupervisor>,
    pub channel_service: crate::chat::channel::ChannelService,
    pub http_client: reqwest::Client,
    pub harness: Arc<crate::agent::harness::Harness>,
    pub push_subscription_repo: Arc<dyn PushSubscriptionRepository>,
    pub push_sender: Option<Arc<PushSender>>,
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

        // Load the catalog before the provider registry — `parse_model_groups`
        // consults it to bake `context_window` into each `ModelGroup` at
        // resolve time.
        let model_catalog = crate::inference::metadata::ModelCatalogStore::new(
            crate::inference::metadata::loader::load_cache_or_defaults(
                std::path::Path::new(&config.storage.cache_dir),
            ),
        );

        let llm_config = load_models_config(models_config);
        let provider_registry = ModelProviderRegistry::from_config(
            llm_config,
            broadcast_service.clone(),
            &config.inference,
            &model_catalog.current(),
        )
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

        let usage_service = crate::inference::usage::UsageService::new(
            model_catalog.clone(),
            SurrealRepo::new(db.clone()),
            broadcast_service.clone(),
        );
        let memory_service = MemoryService::new(
            SurrealRepo::new(db.clone()),
            SurrealRepo::new(db.clone()),
            SurrealRepo::new(db.clone()),
            provider_registry_arc,
            prompt_loader.clone(),
            storage.clone(),
            usage_service.clone(),
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

        let password_reset_repo: SurrealRepo<
            crate::auth::password_reset::models::PasswordResetToken,
        > = SurrealRepo::new(db.clone());
        let password_reset_service = PasswordResetService::new(
            Arc::new(password_reset_repo),
            config.auth.password_reset_expiry_minutes,
        );

        // A bad mail config disables email rather than taking the server down —
        // everything except password reset works fine without it.
        let mail_service = match MailService::from_config(&config.mail) {
            Ok(Some(svc)) => {
                tracing::info!(host = %config.mail.smtp_host, "Outbound email enabled");
                Some(svc)
            }
            Ok(None) => {
                tracing::info!("Outbound email disabled (no mail.smtp_host configured)");
                None
            }
            Err(e) => {
                tracing::error!(error = %e, "Outbound email disabled: invalid mail configuration");
                None
            }
        };

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

        let agent_share_service = crate::agent::share::service::AgentShareService::new(
            SurrealRepo::new(db.clone()),
            user_service.clone(),
        );

        let mut agent_service = AgentService::new(
            SurrealRepo::new(db.clone()),
            &config.cache,
            resource_manager.clone(),
            policy_service.clone(),
            user_service.clone(),
        );
        agent_service.set_share_service(agent_share_service.clone());

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
        let channel_service = crate::chat::channel::ChannelService::new(
            channel_repo,
            channel_registry.clone(),
            Arc::new(vault_service.clone()),
            broadcast_service.clone(),
            config_arc.clone(),
        );

        let push_subscription_repo: Arc<dyn PushSubscriptionRepository> =
            Arc::new(SurrealPushSubscriptionRepo::new(db.clone()));
        let push_sender = PushSender::new(&config.push, push_subscription_repo.clone())
            .ok()
            .flatten()
            .map(Arc::new);
        let notification_service = NotificationService::with_broadcast(
            SurrealRepo::new(db.clone()),
            broadcast_service.clone(),
            push_sender.clone(),
        );
        match &push_sender {
            Some(_) => tracing::info!("Push notifications enabled (VAPID configured)"),
            None => tracing::info!("Push notifications disabled (no VAPID keys configured)"),
        }

        let chat_share_service = crate::chat::share::service::ChatShareService::new(
            SurrealRepo::new(db.clone()),
            user_service.clone(),
        );

        let mut chat_service = ChatService::new(
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
            notification_service.clone(),
            usage_service.clone(),
        );
        chat_service.set_share_service(chat_share_service.clone());
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
            notification_service.clone(),
            vault_service.clone(),
            mcp_service.clone(),
            tool_manager.clone(),
            policy_service.clone(),
            broadcast_service.clone(),
            active_sessions.clone(),
            shutdown_token.clone(),
            prompt_loader.clone(),
            config_arc.clone(),
            usage_service.clone(),
        ));
        let task_executor = Arc::new(crate::agent::task::executor::TaskExecutor::new(
            harness.clone(),
        ));
        let message_repo_for_channel: Arc<dyn crate::chat::message::repository::MessageRepository> =
            Arc::new(SurrealRepo::<crate::chat::message::models::Message>::new(db.clone()));
        let space_service = SpaceService::new(SurrealRepo::new(db.clone()), broadcast_service.clone());
        let contact_service = ContactService::new(SurrealRepo::new(db.clone()), broadcast_service.clone());
        let channel_supervisor = Arc::new(crate::chat::channel::ChannelSupervisor::new(
            config_arc.clone(),
            shutdown_token.clone(),
            channel_service.clone(),
            channel_registry.clone(),
            space_service.clone(),
            user_service.clone(),
            storage.clone(),
            chat_service.clone(),
            share_service.clone(),
            broadcast_service.clone(),
            message_repo_for_channel,
            agent_service.clone(),
            contact_service.clone(),
            policy_service.clone(),
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
            agent_share_service: agent_share_service.clone(),
            space_service,
            call_service: CallService::new(SurrealRepo::new(db.clone())),
            usage_service,
            model_catalog,
            contact_service,
            chat_service,
            chat_share_service: chat_share_service.clone(),
            task_service: TaskService::new(SurrealRepo::new(db.clone()), broadcast_service.clone()),
            broadcast_service: broadcast_service.clone(),
            browser_session_manager: Arc::new(BrowserSessionManager::new(config.browser.clone())),
            active_sessions,
            memory_service,
            notification_service,
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
            password_reset_service,
            mail_service,
            login_tracker: LoginAttemptTracker::new(
                config.auth.max_login_attempts,
                config.auth.lockout_minutes,
            ),
            metrics_handle,
            shutdown_token,
            channel_registry: channel_registry.clone(),
            channel_supervisor,
            channel_service,
            http_client,
            harness,
            push_subscription_repo,
            push_sender,
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
            self.usage_service.clone(),
        ));
        let _ = self.signal_service.set(svc.clone());
        svc
    }

    pub fn signal_service(&self) -> Option<Arc<SignalService>> {
        self.signal_service.get().cloned()
    }

    // -----------------------------------------------------------------------
    // Voice inbound allowlist (DB-stored per user)
    // -----------------------------------------------------------------------

    fn allowlist_key(user_id: &str) -> String {
        format!("voice.inbound_allowlist.{user_id}")
    }

    fn inbound_agent_key(user_id: &str) -> String {
        format!("voice.inbound_agent.{user_id}")
    }

    /// Return the user's preferred inbound answering agent (ID, handle, or
    /// name), if they have set one. Blank values are treated as unset.
    pub async fn get_inbound_agent(&self, user_id: &str) -> Option<String> {
        self.get_runtime_config(&Self::inbound_agent_key(user_id))
            .await
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Set the user's preferred inbound answering agent. A blank value clears
    /// the override, so inbound calls fall back to the user's `receptionist`.
    pub async fn set_inbound_agent(
        &self,
        user_id: &str,
        agent: &str,
    ) -> Result<(), crate::core::error::AppError> {
        self.set_runtime_config(&Self::inbound_agent_key(user_id), agent.trim())
            .await
    }

    fn inbound_greeting_key(user_id: &str) -> String {
        format!("voice.inbound_greeting.{user_id}")
    }

    /// Return the user's inbound welcome greeting, if they have set one. Blank
    /// values are treated as unset.
    pub async fn get_inbound_greeting(&self, user_id: &str) -> Option<String> {
        self.get_runtime_config(&Self::inbound_greeting_key(user_id))
            .await
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Set the user's inbound welcome greeting. A blank value clears it, so the
    /// server-level default (`voice.inbound_welcome_greeting`) applies instead.
    pub async fn set_inbound_greeting(
        &self,
        user_id: &str,
        greeting: &str,
    ) -> Result<(), crate::core::error::AppError> {
        self.set_runtime_config(&Self::inbound_greeting_key(user_id), greeting.trim())
            .await
    }

    /// Return the allowlist entries on a user's allowlist (DB-stored).
    pub async fn get_allowlist(&self, user_id: &str) -> Vec<AllowlistEntry> {
        let key = Self::allowlist_key(user_id);
        match self.get_runtime_config(&key).await {
            Ok(Some(json)) => {
                // Try new format (Vec<AllowlistEntry>) first, fall back to
                // legacy Vec<String> and convert.
                serde_json::from_str::<Vec<AllowlistEntry>>(&json)
                    .unwrap_or_else(|_| {
                        let phones: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                        phones.into_iter().map(|p| AllowlistEntry { phone: p, name: None }).collect()
                    })
            }
            _ => Vec::new(),
        }
    }

    /// Add `phone` (with optional `name`) to the user's allowlist (idempotent).
    pub async fn add_to_allowlist(
        &self,
        user_id: &str,
        phone: &str,
        name: Option<&str>,
    ) -> Result<(), crate::core::error::AppError> {
        let normalized = crate::tool::voice::normalize_phone(phone);
        if normalized.is_empty() || normalized == "+" {
            return Err(crate::core::error::AppError::Validation(
                "Invalid phone number".into(),
            ));
        }
        let mut list = self.get_allowlist(user_id).await;
        // If the phone already exists, update the name; otherwise add.
        if let Some(entry) = list.iter_mut().find(|e| {
            crate::tool::voice::normalize_phone(&e.phone) == normalized
        }) {
            if let Some(n) = name {
                entry.name = Some(n.to_string());
            }
        } else {
            list.push(AllowlistEntry {
                phone: normalized,
                name: name.map(|s| s.to_string()),
            });
        }
        let json = serde_json::to_string(&list)
            .map_err(|e| crate::core::error::AppError::Internal(e.to_string()))?;
        self.set_runtime_config(&Self::allowlist_key(user_id), &json)
            .await
    }

    /// Remove `phone` from the user's allowlist (no-op if absent).
    pub async fn remove_from_allowlist(
        &self,
        user_id: &str,
        phone: &str,
    ) -> Result<(), crate::core::error::AppError> {
        let normalized = crate::tool::voice::normalize_phone(phone);
        let mut list = self.get_allowlist(user_id).await;
        list.retain(|e| crate::tool::voice::normalize_phone(&e.phone) != normalized);
        let json = serde_json::to_string(&list)
            .map_err(|e| crate::core::error::AppError::Internal(e.to_string()))?;
        self.set_runtime_config(&Self::allowlist_key(user_id), &json)
            .await
    }

    /// Resolve which platform user "owns" an inbound call from `phone`.
    ///
    /// Scans every user's DB allowlist (key prefix
    /// `voice.inbound_allowlist.{user_id}`); the first match wins. Ownership is
    /// entirely per-user — there is no global/static fallback list.
    ///
    /// Returns `None` when the caller is not on any user's allowlist.
    /// When matched, returns the entry's `name` if set.
    pub async fn find_user_for_caller(
        &self,
        phone: &str,
    ) -> Option<(String, Option<String>)> {
        let normalized = crate::tool::voice::normalize_phone(phone);
        if normalized.is_empty() || normalized == "+" {
            return None;
        }

        // Check every per-user DB allowlist.
        let mut result = self
            .db
            .query(
                "SELECT key, value FROM runtime_config \
                 WHERE string::starts_with(key, 'voice.inbound_allowlist.')",
            )
            .await
            .ok()?;

        let rows: Vec<serde_json::Value> = result.take(0).ok()?;

        const PREFIX: &str = "voice.inbound_allowlist.";
        for row in &rows {
            let (Some(key), Some(value)) = (
                row.get("key").and_then(|v| v.as_str()),
                row.get("value").and_then(|v| v.as_str()),
            ) else {
                continue; // skip malformed rows
            };
            let Some(user_id) = key.strip_prefix(PREFIX) else {
                continue;
            };

            // Try new format (Vec<AllowlistEntry>) first, fall back to Vec<String>.
            let entries: Vec<AllowlistEntry> = serde_json::from_str(value)
                .unwrap_or_else(|_| {
                    let phones: Vec<String> = serde_json::from_str(value).unwrap_or_default();
                    phones.into_iter().map(|p| AllowlistEntry { phone: p, name: None }).collect()
                });
            for entry in &entries {
                if crate::tool::voice::normalize_phone(&entry.phone) == normalized {
                    return Some((user_id.to_string(), entry.name.clone()));
                }
            }
        }

        None
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
        let (id1, _) = sessions.register("chat-1").await;
        sessions.register("chat-2").await;
        sessions.remove("chat-1", id1).await;
        assert_eq!(sessions.count().await, 1);
    }

    #[tokio::test]
    async fn test_register_cancels_previous() {
        let sessions = ActiveSessions::default();
        let (_, first) = sessions.register("chat-1").await;
        let _second = sessions.register("chat-1").await;
        assert!(first.is_cancelled());
        assert_eq!(sessions.count().await, 1);
    }

    #[tokio::test]
    async fn test_remove_with_stale_id_is_noop() {
        // A superseded run must not delete the successor's token: when a new
        // turn (id2) replaces an old one (id1) for the same chat, the old
        // turn's cleanup `remove(chat, id1)` should leave id2 in place so Stop
        // still targets the live run.
        let sessions = ActiveSessions::default();
        let (id1, _first) = sessions.register("chat-1").await;
        let (_id2, _second) = sessions.register("chat-1").await;
        sessions.remove("chat-1", id1).await; // stale — no-op
        assert_eq!(sessions.count().await, 1);
        assert!(sessions.cancel("chat-1").await, "successor token still present");
    }

    #[tokio::test]
    async fn test_is_current_true_for_the_registered_generation() {
        let sessions = ActiveSessions::default();
        let (id, _) = sessions.register("chat-1").await;
        assert!(sessions.is_current("chat-1", id).await);
    }

    #[tokio::test]
    async fn test_is_current_false_once_superseded() {
        // The old generation (id1) is no longer current once a new run (id2)
        // registers for the same chat — this is what lets a cancelled run
        // tell "the user hit Stop" apart from "an interrupt superseded me".
        let sessions = ActiveSessions::default();
        let (id1, _) = sessions.register("chat-1").await;
        let (_id2, _) = sessions.register("chat-1").await;
        assert!(!sessions.is_current("chat-1", id1).await);
    }

    #[tokio::test]
    async fn test_is_current_false_for_unknown_chat() {
        let sessions = ActiveSessions::default();
        assert!(!sessions.is_current("nonexistent", 0).await);
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

    #[tokio::test]
    async fn test_register_token_makes_cancel_fire_the_supplied_token() {
        // Regression: a task run drives its own cancel token. Registering that
        // same token means the chat-level `cancel` fires the token the run is
        // actually listening on. Previously a throwaway token was registered,
        // so stopping a task from the chat view was silently ignored.
        let sessions = ActiveSessions::default();
        let token = CancellationToken::new();
        let _id = sessions.register_token("chat-1", token.clone()).await;
        assert!(!token.is_cancelled());
        assert!(sessions.cancel("chat-1").await);
        assert!(token.is_cancelled(), "cancel() must fire the caller's token");
    }

    #[tokio::test]
    async fn test_register_token_cancels_previous() {
        let sessions = ActiveSessions::default();
        let (_, first) = sessions.register("chat-1").await;
        let _id = sessions
            .register_token("chat-1", CancellationToken::new())
            .await;
        assert!(first.is_cancelled(), "register_token supersedes the prior run");
        assert_eq!(sessions.count().await, 1);
    }

    #[test]
    fn test_is_shutting_down() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }
}
