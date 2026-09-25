use super::*;
use crate::agent::models::CreateAgentRequest;
use crate::agent::service::AgentService;
use crate::auth::UserService;
use crate::chat::repository::ChatRepository;
use crate::core::config::CacheConfig;
use crate::core::repository::Repository;
use crate::db::init as db_init;
use crate::db::repo::agents::SurrealAgentRepo;
use crate::db::repo::chats::SurrealChatRepo;
use crate::db::repo::generic::SurrealRepo;
use crate::inference::config::RetryConfig;
use crate::inference::provider::{ModelProvider, ModelRef};
use crate::inference::registry::ModelProviderRegistry;
use crate::policy::schema::build_schema;
use crate::policy::service::PolicyService;
use crate::tool::manager::ToolManager;
use crate::tool::sandbox::driver::resource_monitor::SystemResourceManager;
use serde_json::json;
use surrealdb::engine::local::Mem;
use surrealdb::Surreal;

/// Never invoked: `compaction_model_group_for_chats` only resolves model
/// groups against the registry, it doesn't run inference.
struct NoopProvider;

#[async_trait::async_trait]
impl ModelProvider for NoopProvider {
    async fn inference(
        &self,
        _model: &ModelRef,
        _system_prompt: &str,
        _chat_history: Vec<rig_core::completion::Message>,
        _tools: Vec<rig_core::completion::request::ToolDefinition>,
        _max_tokens: Option<u64>,
        _temperature: Option<f64>,
    ) -> Result<crate::inference::provider::InferenceOutput, crate::inference::error::InferenceError>
    {
        unreachable!("NoopProvider is never called by this test")
    }

    async fn stream_inference(
        &self,
        _model: &ModelRef,
        _system_prompt: &str,
        _chat_history: Vec<rig_core::completion::Message>,
        _tools: Vec<rig_core::completion::request::ToolDefinition>,
        _token_tx: tokio::sync::mpsc::Sender<crate::inference::provider::StreamToken>,
        _max_tokens: Option<u64>,
        _temperature: Option<f64>,
    ) -> Result<crate::inference::provider::InferenceOutput, crate::inference::error::InferenceError>
    {
        unreachable!("NoopProvider is never called by this test")
    }

    async fn structured_inference(
        &self,
        _model: &ModelRef,
        _system_prompt: &str,
        _chat_history: Vec<rig_core::completion::Message>,
        _schema: serde_json::Value,
        _max_tokens: Option<u64>,
        _temperature: Option<f64>,
    ) -> Result<serde_json::Value, crate::inference::error::InferenceError> {
        unreachable!("NoopProvider is never called by this test")
    }
}

fn test_model_group(name: &str) -> ModelGroup {
    ModelGroup {
        name: name.into(),
        main: ModelRef {
            provider: "mock".into(),
            model_id: name.into(),
        },
        fallbacks: vec![],
        max_tokens: Some(4096),
        temperature: None,
        context_window: 128_000,
        retry: RetryConfig {
            max_retries: 1,
            initial_backoff_ms: 1,
            backoff_multiplier: 1.0,
            max_backoff_ms: 10,
        },
        inference: Default::default(),
    }
}

async fn test_user_service(db: &Surreal<surrealdb::engine::local::Db>) -> UserService {
    let repo: SurrealRepo<crate::auth::User> = SurrealRepo::new(db.clone());
    let now = Utc::now();
    for id in ["user", "other-user"] {
        let user: crate::auth::User = serde_json::from_value(json!({
            "id": id, "handle": id, "email": format!("{id}@example.com"),
            "name": id, "password_hash": "", "groups": [],
            "created_at": now, "updated_at": now,
        }))
        .unwrap();
        repo.create(&user).await.unwrap();
    }
    UserService::new(SurrealRepo::new(db.clone()), &CacheConfig::default())
}

fn test_policy_service(db: &Surreal<surrealdb::engine::local::Db>, users: UserService) -> PolicyService {
    let repo: std::sync::Arc<dyn crate::policy::repository::PolicyRepository> =
        std::sync::Arc::new(SurrealRepo::<crate::policy::models::Policy>::new(db.clone()));
    let storage = crate::storage::StorageService::new(&crate::core::config::Config::default());
    PolicyService::new(repo, build_schema(), Arc::new(ToolManager::new(false)), storage, users)
}

/// The most recently active chat in scope decides the compaction model,
/// unless `memory.model_group` names one explicitly.
#[tokio::test]
async fn scheduled_memory_uses_latest_chat_in_scope_or_explicit_memory_group() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db_init::setup_schema(&db).await.unwrap();

    let users = test_user_service(&db).await;
    let agents = AgentService::new(
        SurrealAgentRepo::new(db.clone()),
        &CacheConfig::default(),
        Arc::new(SystemResourceManager::new(80.0, 80.0, 90.0, 90.0)),
        test_policy_service(&db, users.clone()),
        users,
    );

    let mock = Arc::new(NoopProvider);
    let mut providers = std::collections::HashMap::new();
    providers.insert("mock".to_string(), mock as Arc<dyn ModelProvider>);
    let mut model_groups = std::collections::HashMap::new();
    for name in ["older", "newer", "unrelated", "dedicated"] {
        model_groups.insert(name.to_string(), test_model_group(name));
    }
    let registry = ModelProviderRegistry::for_testing(providers, model_groups);

    let chat_repo: SurrealChatRepo = SurrealRepo::new(db.clone());
    let now = Utc::now();
    for (index, group) in ["older", "newer", "unrelated"].into_iter().enumerate() {
        let user_id = if group == "unrelated" { "other-user" } else { "user" };
        let agent = agents
            .create(
                user_id,
                CreateAgentRequest {
                    id: None,
                    handle: None,
                    name: group.into(),
                    description: String::new(),
                    model_group: Some(group.into()),
                    tools: None,
                    skills: None,
                    sandbox_policy: None,
                    sandbox_limits: None,
                    voice_id: None,
                    private_memory: None,
                },
            )
            .await
            .unwrap();
        let space_id = if group == "unrelated" { "other-space" } else { "space" };
        let chat = crate::chat::models::Chat {
            id: format!("chat-{group}"),
            user_id: user_id.to_string(),
            space_id: Some(space_id.to_string()),
            task_id: None,
            agent_id: agent.id,
            title: None,
            archived_at: None,
            channel_id: None,
            channel_external_id: None,
            metadata: Default::default(),
            created_at: now,
            updated_at: now + chrono::Duration::seconds(index as i64),
        };
        chat_repo.create(&chat).await.unwrap();
    }

    let mut memory = BasicMemoryService::new(
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        Arc::new(registry),
        PromptLoader::new(std::path::PathBuf::from("resources/prompts")),
        crate::inference::usage::UsageService::new(
            crate::inference::metadata::ModelCatalogStore::new(
                crate::inference::metadata::ModelCatalogSnapshot::empty(),
            ),
            SurrealRepo::new(db.clone()),
            crate::chat::broadcast::BroadcastService::new(),
            Arc::new(std::collections::HashMap::new()),
        ),
        MemoryConfig::default(),
    );

    let user_chats = memory.chat_repo.find_by_user_id("user").await.unwrap();
    let space_chats = memory.chat_repo.find_by_space_id("space").await.unwrap();
    for chats in [&user_chats, &space_chats] {
        assert_eq!(
            memory
                .compaction_model_group_for_chats(&agents, chats)
                .await
                .unwrap()
                .name,
            "newer",
            "the most recently updated in-scope chat's model wins"
        );
    }

    // No configured memory group and nothing in scope: nothing to fall back to.
    memory.memory_config.model_group.clear();
    assert_eq!(
        memory
            .compaction_model_group_for_chats(&agents, &user_chats)
            .await
            .unwrap()
            .name,
        "newer"
    );
    assert!(
        memory
            .compaction_model_group_for_chats(&agents, &[])
            .await
            .is_err()
    );

    // An explicit memory group always wins, regardless of what's in scope.
    memory.memory_config.model_group = "dedicated".into();
    for chats in [user_chats.as_slice(), space_chats.as_slice(), &[]] {
        assert_eq!(
            memory
                .compaction_model_group_for_chats(&agents, chats)
                .await
                .unwrap()
                .name,
            "dedicated"
        );
    }
}
