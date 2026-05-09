//! Integration test: execute a task end-to-end and verify every SSE event
//! that the frontend would receive.

#[allow(dead_code)]
mod helpers;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use frona::agent::service::AgentService;
use frona::agent::task::executor::TaskExecutor;
use frona::agent::task::models::{Task, TaskKind, TaskStatus};
use frona::core::config::Config;
use frona::core::repository::Repository;
use frona::core::state::AppState;
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::storage::StorageService;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;
use tokio::sync::mpsc;

use helpers::{
    drain_sse_frames, MockModelProvider, MockResponse, SseFrame,
    test_model_group,
};

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

/// Build an `AppState` whose provider registry contains a mock provider
/// and a model group named "primary" that resolves to it.
async fn test_app_state_with_mock(
    mock: Arc<MockModelProvider>,
) -> (AppState, tempfile::TempDir) {
    let db = test_db().await;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_string_lossy().to_string();

    let config = Config {
        server: frona::core::config::ServerConfig {
            port: 0,
            static_dir: format!("{base}/static"),
            max_concurrent_tasks: 10,
            ..Default::default()
        },
        auth: frona::core::config::AuthConfig {
            encryption_secret: "test-secret".to_string(),
            ..Default::default()
        },
        database: frona::core::config::DatabaseConfig {
            path: format!("{base}/db"),
        },
        storage: frona::core::config::StorageConfig {
            workspaces_path: format!("{base}/workspaces"),
            files_path: format!("{base}/files"),
            shared_config_dir: format!("{base}/config"),
            ..Default::default()
        },
        ..Default::default()
    };

    let storage = StorageService::new(&config);
    let resource_manager = Arc::new(
        frona::tool::sandbox::driver::resource_monitor::SystemResourceManager::new(
            80.0, 80.0, 90.0, 90.0,
        ),
    );
    let policy_service = {
        let schema = frona::policy::schema::build_schema();
        let repo: std::sync::Arc<dyn frona::policy::repository::PolicyRepository> =
            std::sync::Arc::new(SurrealRepo::<frona::policy::models::Policy>::new(db.clone()));
        let tool_manager = std::sync::Arc::new(frona::tool::manager::ToolManager::new(false));
        let storage = frona::storage::StorageService::new(&config);
        frona::policy::service::PolicyService::new(repo, schema, tool_manager, storage)
    };
    let agent_service = AgentService::new(
        SurrealRepo::new(db.clone()),
        &config.cache,
        std::path::PathBuf::from(&config.storage.shared_config_dir).join("agents"),
        resource_manager.clone(),
        policy_service,
    );

    // Build a provider registry with the mock provider and a "primary" model group.
    let mut providers: HashMap<String, Arc<dyn frona::inference::provider::ModelProvider>> =
        HashMap::new();
    providers.insert("mock".to_string(), mock);

    let mut model_groups = HashMap::new();
    model_groups.insert("primary".to_string(), test_model_group());

    let provider_registry =
        frona::inference::registry::ModelProviderRegistry::for_testing(providers, model_groups);

    let user_service =
        frona::auth::UserService::new(SurrealRepo::new(db.clone()), &config.cache);
    let prompt_loader =
        frona::agent::prompt::PromptLoader::new(format!("{base}/prompts"));

    let provider_registry_arc = Arc::new(provider_registry.clone());
    let memory_service = frona::memory::service::MemoryService::new(
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        provider_registry_arc,
        prompt_loader.clone(),
        storage.clone(),
    );

    let chat_service = frona::chat::service::ChatService::new(
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        agent_service.clone(),
        provider_registry,
        storage.clone(),
        user_service,
        memory_service,
        prompt_loader.clone(),
        frona::chat::broadcast::BroadcastService::new(),
    );

    let metrics_handle = frona::core::metrics::setup_metrics_recorder();
    let mut state =
        AppState::new(db, &config, Some(frona::inference::config::ModelRegistryConfig::empty()), storage, metrics_handle, resource_manager);
    // Replace the chat_service with our version that has the mock provider.
    state.chat_service = chat_service;
    // Replace the agent_service so chat_service in state shares the same
    // underlying repo as the one above.
    state.agent_service = agent_service;

    (state, tmp)
}

fn make_task() -> Task {
    let now = Utc::now();
    Task {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: "user-1".to_string(),
        agent_id: "test-agent".to_string(),
        space_id: None,
        chat_id: None,
        title: "Test task".to_string(),
        description: "Say hello".to_string(),
        status: TaskStatus::Pending,
        kind: TaskKind::Direct { source_chat_id: None },
        run_at: None,
        result_summary: None,
        error_message: None,
        quarantined: false,
        result_schema: None,
        created_at: now,
        updated_at: now,
    }
}

/// Create a minimal agent in the DB so that `ChatSessionContext::build` can
/// resolve it.
async fn seed_agent(db: &Surreal<Db>) {
    let agent = frona::agent::models::Agent {
        id: "test-agent".to_string(),
        user_id: Some("user-1".to_string()),
        name: "Test Agent".to_string(),
        description: String::new(),
        model_group: "primary".to_string(),
        enabled: true,
        skills: None,
        sandbox_limits: None,
        max_concurrent_tasks: None,
        avatar: None,
        identity: Default::default(),
        prompt: Some("You are a test agent. Do what the user asks.".to_string()),
        heartbeat_interval: None,
        next_heartbeat_at: None,
        heartbeat_chat_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let repo: SurrealRepo<frona::agent::models::Agent> = SurrealRepo::new(db.clone());
    repo.create(&agent).await.unwrap();
}

/// Create a minimal user in the DB.
async fn seed_user(db: &Surreal<Db>) {
    let user = frona::auth::User {
        id: "user-1".to_string(),
        username: "testuser".to_string(),
        email: "test@test.com".to_string(),
        name: "Test".to_string(),
        password_hash: String::new(),
        timezone: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let repo: SurrealRepo<frona::auth::User> = SurrealRepo::new(db.clone());
    repo.create(&user).await.unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Execute a task that produces a simple text response and verify the
/// complete sequence of SSE events the frontend would receive.
#[tokio::test]
async fn task_execution_emits_expected_sse_events() {
    helpers::init_metrics();

    let mock = Arc::new(MockModelProvider::new(vec![
        MockResponse::Text("Hello from the task!".to_string()),
    ]));

    let (state, _tmp) = test_app_state_with_mock(mock).await;
    seed_agent(&state.db).await;
    seed_user(&state.db).await;

    // Register an SSE session to capture all events for user-1.
    let (tx, mut rx) = mpsc::unbounded_channel();
    state.broadcast_service.register_session("user-1", tx).await;

    // Create the task in DB.
    let task = make_task();
    let repo: SurrealRepo<Task> = SurrealRepo::new(state.db.clone());
    repo.create(&task).await.unwrap();

    // Execute the task (spawns a background tokio task).
    let task_id = task.id.clone();
    let executor = Arc::new(TaskExecutor::new(state.clone()));
    executor.spawn_execution(task).await.unwrap();

    // Wait for the task to reach a terminal status.
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some(t) = repo.find_by_id(&task_id).await.unwrap() && matches!(t.status, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled) {
            break;
        }
    }

    // Give the dispatcher a moment to route remaining events.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let frames: Vec<SseFrame> = drain_sse_frames(&mut rx).await;
    let event_names: Vec<&str> = frames.iter().map(|f| f.event.as_str()).collect();

    println!("SSE events received: {event_names:?}");
    for frame in &frames {
        println!("  {}: {}", frame.event, frame.data);
    }

    // Assert the exact sequence of SSE events for the first turn:
    //   task_update(inprogress) → chat_message → token → inference_done → ...
    // The executor retries multiple turns (no lifecycle tool), but we only
    // care about the first turn's sequence plus the final task_update.
    assert!(
        event_names.len() >= 4,
        "Expected at least 4 events, got {}: {event_names:?}",
        event_names.len()
    );

    // First event: task_update with status=inprogress
    assert_eq!(event_names[0], "task_update", "First event should be task_update");
    assert_eq!(
        frames[0].data["status"].as_str().unwrap(),
        "inprogress",
        "First task_update should be inprogress"
    );

    // Second event: chat_message (task description saved to the task chat)
    assert_eq!(event_names[1], "chat_message", "Second event should be chat_message");

    // Third event: token (streamed text from the mock provider)
    assert_eq!(event_names[2], "token", "Third event should be token");
    assert_eq!(
        frames[2].data["content"].as_str().unwrap(),
        "Hello from the task!",
        "Token should carry the response text"
    );

    // Fourth event: inference_done with the completed message
    assert_eq!(event_names[3], "inference_done", "Fourth event should be inference_done");
    let message = &frames[3].data["message"];
    assert!(message.is_object(), "inference_done should carry a message object");
    assert_eq!(
        message["content"].as_str().unwrap(),
        "Hello from the task!",
        "inference_done message should contain the response text"
    );
    assert_eq!(
        message["status"].as_str().unwrap(),
        "completed",
        "inference_done message status should be completed"
    );

    // Last event: task_update with terminal status
    let last = frames.last().unwrap();
    assert_eq!(last.event, "task_update", "Last event should be task_update"
    );
}
