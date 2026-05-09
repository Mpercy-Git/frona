//! End-to-end tests for `SignalService`.
//!
//! Exercises the full evaluation pipeline: register watches, dispatch
//! candidates, run matchers, enforce Cedar policy, fire via `TaskExecutor`
//! using a mock model provider so `run_agent_loop` actually runs without
//! touching a real LLM.

#[allow(dead_code)]
mod helpers;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use frona::agent::models::Agent;
use frona::agent::signal::{CandidateEvent, SignalService, Watch};
use frona::agent::task::models::{TaskKind, TaskStatus};
use frona::auth::User;
use frona::chat::service::ChatService;
use frona::core::config::Config;
use frona::core::error::AppError;
use frona::core::repository::Repository;
use frona::core::state::AppState;
use frona::db::init as db_init;
use frona::db::repo::generic::SurrealRepo;
use frona::inference::registry::ModelProviderRegistry;
use frona::storage::StorageService;
use helpers::{init_metrics, test_model_group, MockModelProvider, MockResponse};
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

fn test_config(tmp: &tempfile::TempDir) -> Config {
    let base = tmp.path().to_string_lossy().to_string();
    Config {
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
    }
}

async fn build_state(provider: Arc<MockModelProvider>) -> (AppState, tempfile::TempDir) {
    init_metrics();
    let db: Surreal<Db> = Surreal::new::<Mem>(()).await.unwrap();
    db_init::setup_schema(&db).await.unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let storage = StorageService::new(&config);
    let resource_manager = std::sync::Arc::new(
        frona::tool::sandbox::driver::resource_monitor::SystemResourceManager::new(80.0, 80.0, 90.0, 90.0),
    );
    let metrics_handle = frona::core::metrics::setup_metrics_recorder();

    let mut state = AppState::new(db.clone(), &config, Some(frona::inference::config::ModelRegistryConfig::empty()), storage, metrics_handle, resource_manager);

    // Replace the default chat_service with one wired to the mock model
    // provider so run_agent_loop doesn't try to reach a real LLM.
    let mut providers: HashMap<String, Arc<dyn frona::inference::provider::ModelProvider>> =
        HashMap::new();
    providers.insert("mock".to_string(), provider);
    let mut groups = HashMap::new();
    groups.insert("test".to_string(), test_model_group());
    let mock_registry = ModelProviderRegistry::for_testing(providers, groups);

    let chat_service = ChatService::new(
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        state.agent_service.clone(),
        mock_registry,
        state.storage_service.clone(),
        state.user_service.clone(),
        state.memory_service.clone(),
        state.prompts.clone(),
        state.broadcast_service.clone(),
    );
    state.chat_service = chat_service;

    state.init_task_executor();
    let signal_svc = state.init_signal_service();
    state.policy_service.sync_base_policies().await.unwrap();
    signal_svc.start().await.unwrap();

    (state, tmp)
}

async fn seed_user_and_agent(state: &AppState, user_id: &str, agent_id: &str) {
    // User
    let user_repo: SurrealRepo<User> = SurrealRepo::new(state.db.clone());
    user_repo
        .create(&User {
            id: user_id.into(),
            username: format!("user-{user_id}"),
            email: format!("{user_id}@test.com"),
            name: "Test User".into(),
            password_hash: String::new(),
            timezone: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    // Agent
    let agent_repo: SurrealRepo<Agent> = SurrealRepo::new(state.db.clone());
    agent_repo
        .create(&Agent {
            id: agent_id.into(),
            user_id: Some(user_id.into()),
            name: "Test Agent".into(),
            description: String::new(),
            model_group: "test".into(),
            enabled: true,
            skills: None,
            sandbox_limits: None,
            max_concurrent_tasks: Some(5),
            avatar: None,
            identity: Default::default(),
            // Inline prompt avoids needing an AGENT.md file in the workspace.
            prompt: Some("You are a test agent.".into()),
            heartbeat_interval: None,
            next_heartbeat_at: None,
            heartbeat_chat_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
async fn create_signal_task(
    state: &AppState,
    user_id: &str,
    agent_id: &str,
    source_chat_id: &str,
    tags: Vec<&str>,
    expected_channels: Vec<&str>,
    expected_contacts: Vec<&str>,
    max_evaluations: u32,
) -> frona::agent::task::models::Task {
    create_signal_task_with_mode(
        state,
        user_id,
        agent_id,
        source_chat_id,
        frona::agent::task::models::SignalMode::Once,
        tags,
        expected_channels,
        expected_contacts,
        max_evaluations,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn create_signal_task_with_mode(
    state: &AppState,
    user_id: &str,
    agent_id: &str,
    source_chat_id: &str,
    mode: frona::agent::task::models::SignalMode,
    tags: Vec<&str>,
    expected_channels: Vec<&str>,
    expected_contacts: Vec<&str>,
    max_evaluations: u32,
) -> frona::agent::task::models::Task {
    state
        .task_service
        .create_signal(
            user_id,
            agent_id.into(),
            source_chat_id.into(),
            "Test signal".into(),
            "Wait for: test signal".into(),
            true,
            mode,
            tags.into_iter().map(String::from).collect(),
            expected_channels.into_iter().map(String::from).collect(),
            expected_contacts.into_iter().map(String::from).collect(),
            None,
            max_evaluations,
            None,
        )
        .await
        .unwrap()
}

fn make_candidate(
    user_id: &str,
    categories: Vec<&str>,
    channel: Option<&str>,
    sender: Option<&str>,
) -> CandidateEvent {
    use frona::agent::signal::Annotation;
    CandidateEvent {
        user_id: user_id.into(),
        space_id: None,
        chat_id: None,
        message_id: None,
        connector_id: Some("space-test".into()),
        channel_id: channel.map(String::from),
        contact_id: None,
        sender: sender.map(String::from),
        annotations: categories
            .into_iter()
            .map(|c| Annotation::category("agent:test", c))
            .collect(),
        content: "candidate content".into(),
    }
}

async fn install_forbid_policy(state: &AppState, name: &str, policy_text: &str) {
    use cedar_policy::Policy;
    let policy = Policy::parse(Some(cedar_policy::PolicyId::new(name)), policy_text)
        .expect("policy parses");
    state.policy_service.register_managed_policy(policy);
}

async fn signal_service(state: &AppState) -> Arc<SignalService> {
    state.signal_service().expect("signal service initialized")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_and_unregister_round_trip() {
    let provider = Arc::new(MockModelProvider::new(vec![]));
    let (state, _tmp) = build_state(provider).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;
    let svc = signal_service(&state).await;

    let task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec![],
        vec![],
        50,
    )
    .await;

    // start() ran rebuild_from_db on an empty DB, so the watch isn't there yet
    assert_eq!(svc.watch_count("user-1").await, 0);

    let watch = Watch::from_task(&task).unwrap();
    svc.register(watch).await;
    assert_eq!(svc.watch_count("user-1").await, 1);

    svc.unregister("user-1", &task.id).await;
    assert_eq!(svc.watch_count("user-1").await, 0);
}

#[tokio::test]
async fn rebuild_from_db_hydrates_pending_signal_tasks() {
    let provider = Arc::new(MockModelProvider::new(vec![]));
    let (state, _tmp) = build_state(provider).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;

    // Persist a Signal task BEFORE building a fresh service — so we exercise
    // hydration on startup.
    let _task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec![],
        vec![],
        50,
    )
    .await;

    // Spin up a fresh SignalService against the same DB.
    let fresh = Arc::new(SignalService::new(
        state.task_service.clone(),
        state.task_executor.clone(),
        state.agent_service.clone(),
        state.contact_service.clone(),
        state.user_service.clone(),
        state.policy_service.clone(),
        state.prompts.clone(),
    ));
    fresh.start().await.unwrap();
    assert_eq!(fresh.watch_count("user-1").await, 1);
}

#[tokio::test]
async fn evaluate_with_no_watches_returns_empty() {
    let provider = Arc::new(MockModelProvider::new(vec![]));
    let (state, _tmp) = build_state(provider).await;
    let svc = signal_service(&state).await;

    let cand = make_candidate("user-1", vec!["verification_code"], Some("sms"), None);
    let fired = svc.evaluate("user-1", cand).await.unwrap();
    assert!(fired.is_empty());
}

#[tokio::test]
async fn evaluate_non_matching_watch_does_not_fire() {
    let provider = Arc::new(MockModelProvider::new(vec![]));
    let (state, _tmp) = build_state(provider.clone()).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;
    let svc = signal_service(&state).await;

    let task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec![],
        vec![],
        50,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;

    let cand = make_candidate("user-1", vec!["scheduling"], Some("telegram"), None);
    let fired = svc.evaluate("user-1", cand).await.unwrap();
    assert!(fired.is_empty(), "non-matching tags should not fire");
    assert_eq!(provider.calls(), 0, "no inference should run");
}

#[tokio::test]
async fn evaluate_skips_watches_for_other_users() {
    let provider = Arc::new(MockModelProvider::new(vec![]));
    let (state, _tmp) = build_state(provider).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;
    let svc = signal_service(&state).await;

    let task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec![],
        vec![],
        50,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;

    // Different user — same matching tags, but should not see user-1's watches.
    let cand = make_candidate("user-2", vec!["verification_code"], Some("sms"), None);
    let fired = svc.evaluate("user-2", cand).await.unwrap();
    assert!(fired.is_empty());
}

#[tokio::test]
async fn evaluate_stale_task_unregisters_silently() {
    let provider = Arc::new(MockModelProvider::new(vec![]));
    let (state, _tmp) = build_state(provider).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;
    let svc = signal_service(&state).await;

    let task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec![],
        vec![],
        50,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;
    assert_eq!(svc.watch_count("user-1").await, 1);

    // Cancel the underlying task — fire should detect stale state.
    state
        .task_service
        .mark_cancelled(&task.id)
        .await
        .unwrap();

    let cand = make_candidate("user-1", vec!["verification_code"], Some("sms"), None);
    let fired = svc.evaluate("user-1", cand).await.unwrap();
    assert!(fired.is_empty());
    assert_eq!(
        svc.watch_count("user-1").await,
        0,
        "stale watch should be removed from the index"
    );
}

#[tokio::test]
async fn evaluate_budget_exceeded_marks_task_failed() {
    // max_evaluations = 1 → first fire succeeds (eval_count: 0 → 1, threshold
    // not exceeded). Second fire trips the guard (eval_count: 1 → 2, > 1).
    let provider = Arc::new(MockModelProvider::new(vec![
        MockResponse::Text("first run ack".into()),
    ]));
    let (state, _tmp) = build_state(provider.clone()).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;
    let svc = signal_service(&state).await;

    let task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec![],
        vec![],
        1,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;

    let cand1 = make_candidate("user-1", vec!["verification_code"], Some("sms"), None);
    let fired1 = svc.evaluate("user-1", cand1).await.unwrap();
    assert_eq!(fired1, vec![task.id.clone()], "first fire is allowed");

    // Second fire trips the budget — should not be reported as fired and
    // should mark the task Failed + unregister.
    let cand2 = make_candidate("user-1", vec!["verification_code"], Some("sms"), None);
    let fired2 = svc.evaluate("user-1", cand2).await.unwrap();
    assert!(fired2.is_empty(), "budget-exceeded fires must not be reported");

    let reloaded = state.task_service.find_by_id(&task.id).await.unwrap().unwrap();
    assert_eq!(reloaded.status, TaskStatus::Failed);
    assert_eq!(svc.watch_count("user-1").await, 0);
}

#[tokio::test]
async fn evaluate_with_default_policy_fires_and_runs_agent() {
    // Mock the agent's response — a plain text reply is enough; we just need
    // run_agent_loop to complete without error so fire_signal returns Ok.
    let provider = Arc::new(MockModelProvider::new(vec![MockResponse::Text(
        "Acknowledged.".into(),
    )]));
    let (state, _tmp) = build_state(provider.clone()).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;
    let svc = signal_service(&state).await;

    let task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec!["sms"],
        vec![],
        50,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;

    let cand = make_candidate(
        "user-1",
        vec!["verification_code", "auth"],
        Some("sms"),
        Some("+15551234"),
    );
    let fired = svc.evaluate("user-1", cand).await.unwrap();
    assert_eq!(fired, vec![task.id.clone()]);
    assert!(provider.calls() >= 1, "agent inference should have run");

    // Task gets a chat assigned during ensure_task_chat.
    let reloaded = state.task_service.find_by_id(&task.id).await.unwrap().unwrap();
    assert!(reloaded.chat_id.is_some(), "fire_signal should ensure C₂");
    assert_eq!(
        reloaded.status,
        TaskStatus::Pending,
        "task stays Pending when agent did not call complete_task"
    );
    if let TaskKind::Signal { evaluation_count, .. } = reloaded.kind {
        assert_eq!(evaluation_count, 1, "evaluation_count incremented");
    } else {
        panic!("expected Signal kind");
    }
}

#[tokio::test]
async fn evaluate_with_forbid_policy_blocks_fire() -> Result<(), AppError> {
    let provider = Arc::new(MockModelProvider::new(vec![]));
    let (state, _tmp) = build_state(provider.clone()).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;

    // Install a forbid policy: agent-1 cannot receive signals from connector
    // "space-test" (which is what `make_candidate` sets).
    install_forbid_policy(
        &state,
        "block-test-connector",
        r#"forbid(
            principal == Policy::Agent::"agent-1",
            action == Policy::Action::"receive_signal",
            resource
        ) when { resource.connector_id == "space-test" };"#,
    )
    .await;

    let svc = signal_service(&state).await;
    let task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec![],
        vec![],
        50,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;

    let cand = make_candidate(
        "user-1",
        vec!["verification_code"],
        Some("sms"),
        Some("+15551234"),
    );
    let fired = svc.evaluate("user-1", cand).await?;
    assert!(fired.is_empty(), "policy denial must drop the match");
    assert_eq!(provider.calls(), 0, "no inference should run on policy denial");
    Ok(())
}

#[tokio::test]
async fn evaluate_with_handle_based_policy_blocks_match() -> Result<(), AppError> {
    let provider = Arc::new(MockModelProvider::new(vec![]));
    let (state, _tmp) = build_state(provider.clone()).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;

    // Create a contact that will be referenced by the candidate.
    let contact = state
        .contact_service
        .find_or_create_by_phone("user-1", "+15551234", "Test Contact")
        .await
        .unwrap();

    install_forbid_policy(
        &state,
        "block-by-sender-address",
        r#"@id("block-by-sender-address")
forbid(
    principal,
    action == Policy::Action::"receive_signal",
    resource
) when {
    resource.sender.addresses.contains("+15551234")
};"#,
    )
    .await;

    let svc = signal_service(&state).await;
    let task = create_signal_task(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        vec!["verification_code"],
        vec![],
        vec![],
        50,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;

    let mut cand = make_candidate(
        "user-1",
        vec!["verification_code"],
        Some("sms"),
        Some("+15551234"),
    );
    cand.contact_id = Some(contact.id.clone());

    let fired = svc.evaluate("user-1", cand).await?;
    assert!(fired.is_empty(), "handle-based policy denial must drop the match");
    assert_eq!(provider.calls(), 0);
    Ok(())
}

#[tokio::test]
async fn continuous_task_stays_pending_after_match() {
    let provider = Arc::new(MockModelProvider::new(vec![MockResponse::Text(
        "noted".into(),
    )]));
    let (state, _tmp) = build_state(provider.clone()).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;
    let svc = signal_service(&state).await;

    let task = create_signal_task_with_mode(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        frona::agent::task::models::SignalMode::Continuous,
        vec!["verification_code"],
        vec![],
        vec![],
        50,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;

    let cand = make_candidate(
        "user-1",
        vec!["verification_code"],
        Some("sms"),
        Some("+15551234"),
    );
    let fired = svc.evaluate("user-1", cand).await.unwrap();
    assert_eq!(fired, vec![task.id.clone()]);

    assert_eq!(svc.watch_count("user-1").await, 1);

    let reloaded = state.task_service.find_by_id(&task.id).await.unwrap().unwrap();
    assert_eq!(
        reloaded.status,
        TaskStatus::Pending,
        "continuous task stays Pending across matches"
    );
    if let TaskKind::Signal { evaluation_count, mode, .. } = reloaded.kind {
        assert_eq!(evaluation_count, 1);
        assert_eq!(mode, frona::agent::task::models::SignalMode::Continuous);
    } else {
        panic!("expected Signal kind");
    }
}

#[tokio::test]
async fn continuous_task_budget_exhaustion_marks_completed_not_failed() {
    let provider = Arc::new(MockModelProvider::new(vec![MockResponse::Text(
        "noted".into(),
    )]));
    let (state, _tmp) = build_state(provider).await;
    seed_user_and_agent(&state, "user-1", "agent-1").await;
    let svc = signal_service(&state).await;

    let task = create_signal_task_with_mode(
        &state,
        "user-1",
        "agent-1",
        "chat-A",
        frona::agent::task::models::SignalMode::Continuous,
        vec!["verification_code"],
        vec![],
        vec![],
        1,
    )
    .await;
    svc.register(Watch::from_task(&task).unwrap()).await;

    let cand1 = make_candidate("user-1", vec!["verification_code"], Some("sms"), None);
    let fired1 = svc.evaluate("user-1", cand1).await.unwrap();
    assert_eq!(fired1, vec![task.id.clone()]);

    let cand2 = make_candidate("user-1", vec!["verification_code"], Some("sms"), None);
    let fired2 = svc.evaluate("user-1", cand2).await.unwrap();
    assert!(fired2.is_empty(), "budget-exhausted fires aren't reported");

    let reloaded = state.task_service.find_by_id(&task.id).await.unwrap().unwrap();
    assert_eq!(
        reloaded.status,
        TaskStatus::Completed,
        "continuous tasks complete (not fail) on budget exhaustion"
    );
    assert_eq!(svc.watch_count("user-1").await, 0);
}
