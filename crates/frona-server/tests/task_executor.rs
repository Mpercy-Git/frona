use std::sync::Arc;

use chrono::Utc;
use frona::agent::task::executor::TaskExecutor;
use frona::agent::task::models::{Task, TaskKind, TaskStatus};
use frona::chat::message::models::MessageRole;
use frona::chat::message::models::MessageEvent;
use frona::storage::StorageService;
use frona::core::config::Config;
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::core::state::AppState;
use frona::core::repository::Repository;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

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
        browser: Some(frona::core::config::BrowserConfig {
            ws_url: "ws://localhost:0".to_string(),
            api_token: None,
            profiles_path: format!("{base}/profiles"),
            connection_timeout_ms: 30000,
        }),
        storage: frona::core::config::StorageConfig {
            workspaces_path: format!("{base}/workspaces"),
            files_path: format!("{base}/files"),
            shared_config_dir: format!("{base}/config"),
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn test_app_state() -> (AppState, tempfile::TempDir) {
    let db = test_db().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let storage = StorageService::new(&config);
    let resource_manager = std::sync::Arc::new(
        frona::tool::sandbox::driver::resource_monitor::SystemResourceManager::new(80.0, 80.0, 90.0, 90.0),
    );
    let metrics_handle = frona::core::metrics::setup_metrics_recorder();
    let state = AppState::new(db, &config, Some(frona::inference::config::ModelRegistryConfig::empty()), storage, metrics_handle, resource_manager);
    (state, tmp)
}

fn make_executor(state: &AppState) -> Arc<TaskExecutor> {
    Arc::new(TaskExecutor::new(state.clone()))
}

fn make_task(kind: TaskKind) -> Task {
    let now = Utc::now();
    Task {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: "user-1".to_string(),
        agent_id: "agent-1".to_string(),
        space_id: None,
        chat_id: None,
        title: "Test task".to_string(),
        description: "Do something".to_string(),
        status: TaskStatus::Pending,
        kind,
        run_at: None,
        result_summary: None,
        error_message: None,
        quarantined: false,
        result_schema: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn ensure_task_chat_creates_when_missing() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);
    let mut task = make_task(TaskKind::Direct { source_chat_id: None });
    assert!(task.chat_id.is_none());

    let chat_id = executor.ensure_task_chat(&mut task).await.unwrap();

    assert!(!chat_id.is_empty());
    assert_eq!(task.chat_id.as_deref(), Some(chat_id.as_str()));
}

#[tokio::test]
async fn ensure_task_chat_returns_existing() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);
    let mut task = make_task(TaskKind::Direct { source_chat_id: None });
    task.chat_id = Some("existing-chat-id".to_string());

    let chat_id = executor.ensure_task_chat(&mut task).await.unwrap();

    assert_eq!(chat_id, "existing-chat-id");
    assert_eq!(task.chat_id.as_deref(), Some("existing-chat-id"));
}

#[tokio::test]
async fn save_initial_message_saves_on_first_run() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);
    let mut task = make_task(TaskKind::Direct { source_chat_id: None });
    let chat_id = executor.ensure_task_chat(&mut task).await.unwrap();

    executor
        .save_initial_message_if_needed(&task, &chat_id)
        .await
        .unwrap();

    let messages = state.chat_service.get_stored_messages(&chat_id).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Do something");
}

#[tokio::test]
async fn save_initial_message_skips_on_resume() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);
    let mut task = make_task(TaskKind::Direct { source_chat_id: None });
    let chat_id = executor.ensure_task_chat(&mut task).await.unwrap();

    executor
        .save_initial_message_if_needed(&task, &chat_id)
        .await
        .unwrap();
    executor
        .save_initial_message_if_needed(&task, &chat_id)
        .await
        .unwrap();

    let messages = state.chat_service.get_stored_messages(&chat_id).await.unwrap();
    assert_eq!(messages.len(), 1);
}

#[tokio::test]
async fn handle_cancelled_marks_task_cancelled() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);
    let mut task = make_task(TaskKind::Direct { source_chat_id: None });
    let _chat_id = executor.ensure_task_chat(&mut task).await.unwrap();

    let repo: SurrealRepo<Task> = SurrealRepo::new(state.db.clone());
    repo.create(&task).await.unwrap();

    executor
        .handle_cancelled(&task)
        .await
        .unwrap();

    let updated = repo.find_by_id(&task.id).await.unwrap().unwrap();
    assert_eq!(updated.status, TaskStatus::Cancelled);
}

#[tokio::test]
async fn handle_error_marks_failed_and_delivers() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);

    let source_chat = state
        .chat_service
        .create_chat(
            "user-1",
            frona::chat::models::CreateChatRequest {
                space_id: None,
                task_id: None,
                agent_id: "agent-1".to_string(),
                title: Some("Source chat".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

    let mut task = make_task(TaskKind::Delegation {
        source_agent_id: "agent-1".to_string(),
        source_chat_id: source_chat.id.clone(),
        resume_parent: false,
    });
    task.chat_id = Some("task-chat-id".to_string());

    let repo: SurrealRepo<Task> = SurrealRepo::new(state.db.clone());
    repo.create(&task).await.unwrap();

    let error = frona::core::error::AppError::Internal("something broke".to_string());
    executor.handle_error(&task, &error).await.unwrap();

    let updated = repo.find_by_id(&task.id).await.unwrap().unwrap();
    assert_eq!(updated.status, TaskStatus::Failed);

    let source_messages = state
        .chat_service
        .get_stored_messages(&source_chat.id)
        .await.unwrap();
    assert_eq!(source_messages.len(), 1);
    assert!(source_messages[0].content.contains("something broke"));
}

#[tokio::test]
async fn lifecycle_complete_event_detected() {
    let (state, _tmp) = test_app_state().await;
    let _executor = make_executor(&state);

    let source_chat = state
        .chat_service
        .create_chat(
            "user-1",
            frona::chat::models::CreateChatRequest {
                space_id: None,
                task_id: Some("task-1".to_string()),
                agent_id: "agent-1".to_string(),
                title: Some("Task chat".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

    // Save a System lifecycle event (simulating what TaskControlTool does)
    state
        .chat_service
        .save_system_event(
            &source_chat.id,
            MessageEvent::TaskCompletion {
                task_id: "task-1".to_string(),
                chat_id: Some(source_chat.id.clone()),
                status: TaskStatus::Completed,
                summary: Some("Research findings here".to_string()),
            },
        )
        .await
        .unwrap();

    // Verify the System message was saved
    let messages = state.chat_service.get_stored_messages(&source_chat.id).await.unwrap();
    let system_msgs: Vec<_> = messages
        .iter()
        .filter(|m| m.role == MessageRole::System)
        .collect();
    assert_eq!(system_msgs.len(), 1);
    assert!(matches!(
        &system_msgs[0].event,
        Some(MessageEvent::TaskCompletion {
            status: TaskStatus::Completed,
            summary: Some(s),
            ..
        }) if s == "Research findings here"
    ));
}

#[tokio::test]
async fn lifecycle_defer_event_detected() {
    let (state, _tmp) = test_app_state().await;
    let _executor = make_executor(&state);

    let task_chat = state
        .chat_service
        .create_chat(
            "user-1",
            frona::chat::models::CreateChatRequest {
                space_id: None,
                task_id: Some("task-2".to_string()),
                agent_id: "agent-1".to_string(),
                title: Some("Task chat".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

    state
        .chat_service
        .save_system_event(
            &task_chat.id,
            MessageEvent::TaskDeferred {
                task_id: "task-2".to_string(),
                delay_minutes: 30,
                reason: "Waiting for external API".to_string(),
            },
        )
        .await
        .unwrap();

    let messages = state.chat_service.get_stored_messages(&task_chat.id).await.unwrap();
    let system_msgs: Vec<_> = messages
        .iter()
        .filter(|m| m.role == MessageRole::System)
        .collect();
    assert_eq!(system_msgs.len(), 1);
    assert!(matches!(
        &system_msgs[0].event,
        Some(MessageEvent::TaskDeferred {
            delay_minutes: 30,
            reason,
            ..
        }) if reason == "Waiting for external API"
    ));
}

#[tokio::test]
async fn mark_deferred_sets_pending_with_run_at() {
    let (state, _tmp) = test_app_state().await;

    let repo: SurrealRepo<Task> = SurrealRepo::new(state.db.clone());
    let mut task = make_task(TaskKind::Direct { source_chat_id: None });
    task.status = TaskStatus::InProgress;
    repo.create(&task).await.unwrap();

    let run_at = Utc::now() + chrono::Duration::minutes(30);
    state
        .task_service
        .mark_deferred(&task.id, run_at, "waiting")
        .await
        .unwrap();

    let updated = repo.find_by_id(&task.id).await.unwrap().unwrap();
    assert_eq!(updated.status, TaskStatus::Pending);
    assert!(updated.run_at.is_some());
}

#[tokio::test]
async fn deliver_to_source_skips_direct_tasks() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);
    let task = make_task(TaskKind::Direct { source_chat_id: None });

    executor
        .deliver_event_to_source(
            &task,
            frona::agent::task::executor::TaskLifecycleEvent::Completion {
                status: TaskStatus::Completed,
                summary: Some("result".to_string()),
            },
            vec![],
        )
        .await;
}

#[tokio::test]
async fn deliver_to_source_sends_to_delegation() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);

    let source_chat = state
        .chat_service
        .create_chat(
            "user-1",
            frona::chat::models::CreateChatRequest {
                space_id: None,
                task_id: None,
                agent_id: "agent-1".to_string(),
                title: Some("Source".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

    let mut task = make_task(TaskKind::Delegation {
        source_agent_id: "agent-1".to_string(),
        source_chat_id: source_chat.id.clone(),
        resume_parent: false,
    });
    task.chat_id = Some("task-chat".to_string());

    executor
        .deliver_event_to_source(
            &task,
            frona::agent::task::executor::TaskLifecycleEvent::Completion {
                status: TaskStatus::Completed,
                summary: Some("All done".to_string()),
            },
            vec![],
        )
        .await;

    let messages = state
        .chat_service
        .get_stored_messages(&source_chat.id)
        .await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "All done");
}

#[tokio::test]
async fn deliver_to_source_sends_to_direct_with_source_chat() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);

    let source_chat = state
        .chat_service
        .create_chat(
            "user-1",
            frona::chat::models::CreateChatRequest {
                space_id: None,
                task_id: None,
                agent_id: "agent-1".to_string(),
                title: Some("User chat".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

    let mut task = make_task(TaskKind::Direct {
        source_chat_id: Some(source_chat.id.clone()),
    });
    task.chat_id = Some("self-task-chat".to_string());

    executor
        .deliver_event_to_source(
            &task,
            frona::agent::task::executor::TaskLifecycleEvent::Completion {
                status: TaskStatus::Completed,
                summary: Some("Self-task result".to_string()),
            },
            vec![],
        )
        .await;

    let messages = state
        .chat_service
        .get_stored_messages(&source_chat.id)
        .await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Self-task result");
}

#[tokio::test]
async fn broadcast_task_status_emits_event() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    state.broadcast_service.register_session("user-1", tx).await;

    let mut task = make_task(TaskKind::Direct { source_chat_id: None });
    task.chat_id = Some("chat-123".to_string());

    executor.broadcast_task_status(&task, "completed", Some("All done"));

    // Wait briefly for the dispatcher to route the event
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let event = rx.try_recv().expect("Expected to receive an SSE event");
    let event: Result<axum::response::sse::Event, std::convert::Infallible> = event;
    let _sse_event = event.unwrap();
    // The fact that we received an event confirms the broadcast works.
    // Detailed field-level assertions are covered by API integration tests.
}

#[tokio::test]
async fn concurrency_global_limit() {
    let (mut state, _tmp) = test_app_state().await;
    state.max_concurrent_tasks = 1;
    let executor = make_executor(&state);

    let repo: SurrealRepo<Task> = SurrealRepo::new(state.db.clone());

    let mut task1 = make_task(TaskKind::Direct { source_chat_id: None });
    task1.id = "task-1".to_string();
    task1.status = TaskStatus::InProgress;
    repo.create(&task1).await.unwrap();
    executor.spawn_execution(task1).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut task2 = make_task(TaskKind::Direct { source_chat_id: None });
    task2.id = "task-2".to_string();
    repo.create(&task2).await.unwrap();
    executor.spawn_execution(task2).await.unwrap();

    let t2 = repo.find_by_id("task-2").await.unwrap().unwrap();
    assert_eq!(t2.status, TaskStatus::Pending, "Second task should stay pending when limit reached");
}

#[tokio::test]
async fn deliver_to_source_signal_only_sends_empty_content() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);

    let source_chat = state
        .chat_service
        .create_chat(
            "user-1",
            frona::chat::models::CreateChatRequest {
                space_id: None,
                task_id: None,
                agent_id: "agent-1".to_string(),
                title: Some("Source".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

    let mut task = make_task(TaskKind::Delegation {
        source_agent_id: "agent-1".to_string(),
        source_chat_id: source_chat.id.clone(),
        resume_parent: false,
    });
    task.chat_id = Some("task-chat".to_string());

    // Signal-only completion: no result text, no deliverables
    executor
        .deliver_event_to_source(
            &task,
            frona::agent::task::executor::TaskLifecycleEvent::Completion {
                status: TaskStatus::Completed,
                summary: None,
            },
            vec![],
        )
        .await;

    let messages = state
        .chat_service
        .get_stored_messages(&source_chat.id)
        .await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].content.is_empty(), "Signal-only completion should have empty content");
}

#[tokio::test]
async fn deliver_to_source_saves_message_to_user_chat() {
    let (state, _tmp) = test_app_state().await;
    let executor = make_executor(&state);

    // Create a user chat (no task_id) — resume_parent=true so
    // check_and_resume_parent runs, but it should bail out because
    // the source chat is not a task chat.
    let user_chat = state
        .chat_service
        .create_chat(
            "user-1",
            frona::chat::models::CreateChatRequest {
                space_id: None,
                task_id: None,
                agent_id: "agent-1".to_string(),
                title: Some("User chat".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

    let mut task = make_task(TaskKind::Delegation {
        source_agent_id: "agent-1".to_string(),
        source_chat_id: user_chat.id.clone(),
        resume_parent: true,
    });
    task.chat_id = Some("task-chat".to_string());

    let repo: SurrealRepo<Task> = SurrealRepo::new(state.db.clone());
    repo.create(&task).await.unwrap();

    executor
        .deliver_event_to_source(
            &task,
            frona::agent::task::executor::TaskLifecycleEvent::Completion {
                status: TaskStatus::Completed,
                summary: Some("Done".to_string()),
            },
            vec![],
        )
        .await;

    // Message should be delivered
    let messages = state
        .chat_service
        .get_stored_messages(&user_chat.id)
        .await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Done");
}

#[tokio::test]
async fn lifecycle_event_saved_after_assistant_message() {
    let (state, _tmp) = test_app_state().await;

    let chat = state
        .chat_service
        .create_chat(
            "user-1",
            frona::chat::models::CreateChatRequest {
                space_id: None,
                task_id: Some("task-order".to_string()),
                agent_id: "agent-1".to_string(),
                title: Some("Task chat".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

    // Simulate the executor flow: save assistant message first
    state
        .chat_service
        .save_agent_message(&chat.id, "agent-1", "Here is my answer.".to_string(), None)
        .await
        .unwrap();

    // Then save lifecycle event
    state
        .chat_service
        .save_system_event(
            &chat.id,
            MessageEvent::TaskCompletion {
                task_id: "task-order".to_string(),
                chat_id: Some(chat.id.clone()),
                status: TaskStatus::Completed,
                summary: None,
            },
        )
        .await
        .unwrap();

    // Verify ordering: assistant message comes before system event
    let messages = state.chat_service.get_stored_messages(&chat.id).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::Agent);
    assert_eq!(messages[0].content, "Here is my answer.");
    assert_eq!(messages[1].role, MessageRole::System);
    assert!(matches!(
        &messages[1].event,
        Some(MessageEvent::TaskCompletion { status: TaskStatus::Completed, .. })
    ));
}
